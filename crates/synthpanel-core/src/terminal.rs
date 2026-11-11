//! Interactive shell sessions for the web terminal.
//!
//! The core owns the sessions: it allocates a pseudo-terminal, starts a shell
//! inside it and keeps the output buffered. Clients open a session, write
//! keystrokes and read the output. Sessions never leave the privileged
//! process, so the web UI never needs shell access of its own.
//!
//! The panel deliberately does **not** expose an SSH port: the terminal is
//! reachable through the authenticated API only.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::unix::AsyncFd;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// How much output is kept per session.
const BUFFER_LIMIT: usize = 256 * 1024;

/// Sessions that were not used for this long are closed.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Options for a new session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TerminalOptions {
    /// The shell to run. Defaults to `$SHELL` or `/bin/bash`.
    pub shell: Option<String>,
    /// The working directory. Defaults to `/root`.
    pub cwd: Option<String>,
    /// The user to run as. Only `root` is supported by default.
    pub user: Option<String>,
    pub rows: Option<u16>,
    pub cols: Option<u16>,
}

/// Public information about a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub shell: String,
    pub cwd: String,
    pub created_at: String,
    pub closed: bool,
}

/// A chunk of terminal output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalChunk {
    pub id: String,
    /// The output as a UTF-8 string (lossy for binary output).
    pub data: String,
    pub closed: bool,
}

/// The registry of open sessions.
#[derive(Clone, Default)]
pub struct TerminalRegistry {
    sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
}

/// One shell session.
struct Session {
    id: String,
    shell: String,
    cwd: String,
    created_at: String,
    master: Arc<AsyncFd<MasterFd>>,
    child: Mutex<Option<Child>>,
    state: Mutex<SessionState>,
    last_used: Mutex<Instant>,
}

#[derive(Default)]
struct SessionState {
    buffer: Vec<u8>,
    closed: bool,
}

/// A file descriptor wrapper so it can be registered with the reactor.
struct MasterFd(OwnedFd);

impl AsRawFd for MasterFd {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.0.as_raw_fd()
    }
}

impl TerminalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new shell session.
    pub async fn open(&self, options: TerminalOptions) -> Result<SessionInfo> {
        let shell = options
            .shell
            .filter(|value| !value.trim().is_empty())
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/bash".to_string());
        let cwd = options
            .cwd
            .filter(|value| !value.trim().is_empty())
            .or_else(|| std::env::var("HOME").ok())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "/".to_string());
        let rows = options.rows.unwrap_or(24).clamp(1, 500);
        let cols = options.cols.unwrap_or(80).clamp(1, 500);

        let (master, slave) = open_pty()?;

        let child = match spawn_shell(&shell, &cwd, rows, cols, &slave).await {
            Ok(child) => child,
            Err(error) => {
                // The requested directory may not be accessible (a user's home
                // directory, for example); fall back to the root directory.
                if cwd == "/" {
                    return Err(error);
                }
                warn!(%cwd, %error, "the working directory is not usable; using /");
                spawn_shell(&shell, "/", rows, cols, &slave).await?
            }
        };

        // The master end must not block the reactor.
        set_nonblocking(master.as_raw_fd())?;

        let id = uuid::Uuid::new_v4().to_string();
        let master_fd = MasterFd(master);
        // SAFETY: the descriptor is owned by `master_fd` and stays open for as
        // long as the session lives.
        let async_fd =
            unsafe { AsyncFd::register(master_fd) }.context("cannot register the terminal")?;
        let master = Arc::new(async_fd);

        let session = Arc::new(Session {
            id: id.clone(),
            shell: shell.clone(),
            cwd: cwd.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            master: master.clone(),
            child: Mutex::new(Some(child)),
            state: Mutex::new(SessionState::default()),
            last_used: Mutex::new(Instant::now()),
        });

        // Drain the terminal into the buffer.
        spawn_reader(session.clone());

        let mut sessions = self.sessions.lock().await;
        sessions.insert(id.clone(), session);
        info!(%id, %shell, "terminal session opened");
        Ok(SessionInfo {
            id,
            shell,
            cwd,
            created_at: chrono::Utc::now().to_rfc3339(),
            closed: false,
        })
    }

    /// Write input to a session.
    pub async fn write(&self, id: &str, data: &[u8]) -> Result<usize> {
        let session = self.session(id).await?;
        let written = write_to_master(&session.master, data).await?;
        *session.last_used.lock().await = Instant::now();
        Ok(written)
    }

    /// Read the buffered output of a session.
    pub async fn read(&self, id: &str, max: usize) -> Result<TerminalChunk> {
        let session = self.session(id).await?;
        let mut state = session.state.lock().await;
        let take = max.clamp(1, BUFFER_LIMIT).min(state.buffer.len());
        let data = state.buffer.drain(..take).collect::<Vec<u8>>();
        Ok(TerminalChunk {
            id: id.to_string(),
            data: String::from_utf8_lossy(&data).to_string(),
            closed: state.closed,
        })
    }

    /// Resize the terminal window.
    pub async fn resize(&self, id: &str, rows: u16, cols: u16) -> Result<()> {
        let session = self.session(id).await?;
        let size = libc::winsize {
            ws_row: rows.clamp(1, 500),
            ws_col: cols.clamp(1, 500),
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let raw = session.master.as_raw_fd();
        let result = unsafe { libc::ioctl(raw, libc::TIOCSWINSZ, &size) };
        if result < 0 {
            bail!(
                "cannot resize the terminal: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(())
    }

    /// Close a session.
    pub async fn close(&self, id: &str) -> Result<bool> {
        let mut sessions = self.sessions.lock().await;
        let Some(session) = sessions.remove(id) else {
            return Ok(false);
        };
        if let Some(mut child) = session.child.lock().await.take() {
            let _ = child.kill().await;
        }
        session.state.lock().await.closed = true;
        info!(%id, "terminal session closed");
        Ok(true)
    }

    /// List the open sessions.
    pub async fn list(&self) -> Vec<SessionInfo> {
        let sessions = self.sessions.lock().await;
        let mut out = Vec::new();
        for session in sessions.values() {
            let closed = session.state.lock().await.closed;
            out.push(SessionInfo {
                id: session.id.clone(),
                shell: session.shell.clone(),
                cwd: session.cwd.clone(),
                created_at: session.created_at.clone(),
                closed,
            });
        }
        out
    }

    /// Close the sessions that have been idle for too long.
    pub async fn reap(&self) {
        let mut sessions = self.sessions.lock().await;
        let stale: Vec<String> = sessions
            .iter()
            .filter(|(_, session)| {
                session
                    .last_used
                    .try_lock()
                    .map(|last| last.elapsed() > IDLE_TIMEOUT)
                    .unwrap_or(false)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some(session) = sessions.remove(&id) {
                if let Some(mut child) = session.child.lock().await.take() {
                    let _ = child.kill().await;
                }
                info!(%id, "the idle terminal session was closed");
            }
        }
    }

    /// Look a session up.
    async fn session(&self, id: &str) -> Result<Arc<Session>> {
        let sessions = self.sessions.lock().await;
        sessions
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("the terminal session `{id}` does not exist"))
    }
}

/// Read the terminal into the session buffer until the shell exits.
fn spawn_reader(session: Arc<Session>) {
    tokio::spawn(async move {
        let master = session.master.clone();
        let mut buffer = vec![0u8; 8192];
        loop {
            let mut guard = match master.readable().await {
                Ok(guard) => guard,
                Err(error) => {
                    warn!(%error, "the terminal reader stopped");
                    break;
                }
            };
            let read = guard.try_io(|inner| {
                let raw = inner.get_ref().as_raw_fd();
                let count = unsafe {
                    libc::read(
                        raw,
                        buffer.as_mut_ptr().cast::<libc::c_void>(),
                        buffer.len(),
                    )
                };
                if count < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(count as usize)
            });
            match read {
                Ok(Ok(0)) => break,
                Ok(Ok(count)) => {
                    let mut state = session.state.lock().await;
                    state.buffer.extend_from_slice(&buffer[..count]);
                    if state.buffer.len() > BUFFER_LIMIT {
                        let overflow = state.buffer.len() - BUFFER_LIMIT;
                        state.buffer.drain(..overflow);
                    }
                }
                Ok(Err(error)) => {
                    warn!(%error, "the terminal read failed");
                    break;
                }
                Err(_) => continue,
            }
        }
        session.state.lock().await.closed = true;
    });
}

/// Write to the terminal master.
async fn write_to_master(master: &Arc<AsyncFd<MasterFd>>, data: &[u8]) -> Result<usize> {
    let mut written = 0;
    while written < data.len() {
        let mut guard = master.writable().await?;
        let result = guard.try_io(|inner| {
            let raw = inner.get_ref().as_raw_fd();
            let count = unsafe {
                libc::write(
                    raw,
                    data[written..].as_ptr().cast::<libc::c_void>(),
                    data.len() - written,
                )
            };
            if count < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(count as usize)
        });
        match result {
            Ok(Ok(count)) => written += count,
            Ok(Err(error)) => bail!("cannot write to the terminal: {error}"),
            Err(_) => continue,
        }
    }
    Ok(written)
}

/// Start the shell inside the pseudo-terminal.
async fn spawn_shell(
    shell: &str,
    cwd: &str,
    rows: u16,
    cols: u16,
    slave: &OwnedFd,
) -> Result<Child> {
    let mut command = Command::new(shell);
    command
        .arg("-i")
        .current_dir(cwd)
        .env("TERM", "xterm-256color")
        .env("LINES", rows.to_string())
        .env("COLUMNS", cols.to_string())
        .stdin(std::process::Stdio::from(
            slave
                .try_clone()
                .context("cannot duplicate the terminal descriptor")?,
        ))
        .stdout(std::process::Stdio::from(
            slave
                .try_clone()
                .context("cannot duplicate the terminal descriptor")?,
        ))
        .stderr(std::process::Stdio::from(
            slave
                .try_clone()
                .context("cannot duplicate the terminal descriptor")?,
        ))
        .kill_on_drop(true);

    let raw = slave.as_raw_fd();
    unsafe {
        command.pre_exec(move || {
            // Become a session leader and take the pseudo-terminal as the
            // controlling terminal so job control works.
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ioctl(raw, libc::TIOCSCTTY, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    command
        .spawn()
        .with_context(|| format!("cannot start the shell `{shell}`"))
}

/// Allocate a pseudo-terminal pair.
fn open_pty() -> Result<(OwnedFd, OwnedFd)> {
    let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
    if master < 0 {
        bail!(
            "cannot allocate a pseudo-terminal: {}",
            std::io::Error::last_os_error()
        );
    }
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    if unsafe { libc::grantpt(master.as_raw_fd()) } < 0 {
        bail!(
            "cannot grant the pseudo-terminal: {}",
            std::io::Error::last_os_error()
        );
    }
    if unsafe { libc::unlockpt(master.as_raw_fd()) } < 0 {
        bail!(
            "cannot unlock the pseudo-terminal: {}",
            std::io::Error::last_os_error()
        );
    }
    let path = slave_path(&master)?;
    let slave = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    if slave < 0 {
        bail!(
            "cannot open the pseudo-terminal: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok((master, unsafe { OwnedFd::from_raw_fd(slave) }))
}

/// The path of the slave end of a pseudo-terminal.
fn slave_path(master: &OwnedFd) -> Result<std::ffi::CString> {
    let mut buffer = vec![0i8; 256];
    let result = unsafe { libc::ptsname_r(master.as_raw_fd(), buffer.as_mut_ptr(), buffer.len()) };
    if result != 0 {
        bail!(
            "cannot resolve the pseudo-terminal: {}",
            std::io::Error::last_os_error()
        );
    }
    let path = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) };
    Ok(path.to_owned())
}

/// Put a descriptor into non-blocking mode.
fn set_nonblocking(fd: std::os::fd::RawFd) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        bail!(
            "cannot read the terminal flags: {}",
            std::io::Error::last_os_error()
        );
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        bail!(
            "cannot set the terminal flags: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn opens_a_shell_and_reads_its_output() {
        let registry = TerminalRegistry::new();
        let info = registry
            .open(TerminalOptions {
                shell: Some("/bin/sh".into()),
                cwd: Some("/tmp".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        // Turn the terminal echo off so the assertion only sees the output of
        // the command, not the echoed input.
        registry.write(&info.id, b"stty -echo\n").await.unwrap();
        registry
            .write(&info.id, b"echo synthpanel-terminal-ok\n")
            .await
            .unwrap();

        let mut output = String::new();
        for _ in 0..50 {
            let chunk = registry.read(&info.id, 8192).await.unwrap();
            output.push_str(&chunk.data);
            if output.contains("synthpanel-terminal-ok") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(output.contains("synthpanel-terminal-ok"), "got: {output}");
        assert!(registry.close(&info.id).await.unwrap());
        assert!(!registry.close(&info.id).await.unwrap());
    }

    #[tokio::test]
    async fn rejects_unknown_sessions() {
        let registry = TerminalRegistry::new();
        assert!(registry.read("nope", 10).await.is_err());
        assert!(!registry.close("nope").await.unwrap());
    }
}
