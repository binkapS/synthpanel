//! `synthpanel-installer` — bootstrap a fresh server.
//!
//! The installer is what `curl -sSfL https://get.synthpanel.io | sudo bash`
//! downloads and runs. It supports interactive prompts and an unattended mode.

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use tracing::{info, warn};

#[derive(Debug, Parser)]
#[command(
    name = "synthpanel-installer",
    version,
    about = "Install SynthPanel on a fresh server"
)]
pub struct Args {
    /// Non-interactive mode: no prompts, use provided flags.
    #[arg(long)]
    pub unattended: bool,

    /// Panel hostname (required for a Let's Encrypt cert later).
    #[arg(long)]
    pub hostname: Option<String>,

    /// Email for the admin account (auto-generated when omitted).
    #[arg(long)]
    pub admin_email: Option<String>,

    /// Admin password, or omitted for a generated one.
    #[arg(long)]
    pub admin_password: Option<String>,

    /// Update channel.
    #[arg(long, value_enum, default_value = "stable")]
    pub channel: Channel,

    /// Skip package installation (assumes they are already present).
    #[arg(long)]
    pub skip_packages: bool,

    /// Skip the systemd wiring (useful in containers).
    #[arg(long)]
    pub skip_systemd: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Channel {
    Stable,
    Beta,
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn `{cmd}`"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{cmd} {args:?} failed: {stderr}");
    }
    Ok(())
}

fn run_ok(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn require_root() -> Result<()> {
    if std::env::var("EUID").map(|v| v != "0").unwrap_or(true) {
        bail!("the installer must run as root (use `sudo`); re-run with root privileges");
    }
    Ok(())
}

/// Ubuntu releases the installer knows how to bootstrap.
pub const UBUNTU_VERSIONS: &[&str] = &["22.04", "24.04", "26.04"];
/// Debian releases the installer knows how to bootstrap.
pub const DEBIAN_VERSIONS: &[&str] = &["12", "13"];

/// Read `ID` and `VERSION_ID` from an os-release file body.
fn parse_os_release(text: &str) -> (String, String) {
    fn value(text: &str, key: &str) -> String {
        text.lines()
            .find(|line| line.starts_with(key))
            .map(|line| line.trim_start_matches(key).trim_matches('"').to_string())
            .unwrap_or_default()
    }
    (value(text, "ID="), value(text, "VERSION_ID="))
}

fn check_distro(id: &str, version: &str) -> Result<()> {
    let supported = match id {
        "ubuntu" => UBUNTU_VERSIONS.contains(&version),
        "debian" => DEBIAN_VERSIONS.contains(&version),
        _ => false,
    };
    if !supported {
        bail!(
            "unsupported distro `{id} {version}`; SynthPanel supports Ubuntu {} and Debian {}",
            UBUNTU_VERSIONS.join("/"),
            DEBIAN_VERSIONS.join("/")
        );
    }
    Ok(())
}

fn detect_distro() -> Result<String> {
    let output = Command::new("cat")
        .arg("/etc/os-release")
        .output()
        .context("cannot read /etc/os-release")?;
    let text = String::from_utf8_lossy(&output.stdout);
    let (id, version) = parse_os_release(&text);
    check_distro(&id, &version)?;
    Ok(format!("{id}-{version}"))
}

/// Packages shared by every supported release. The unversioned `php-fpm`
/// meta package is used on purpose: a specific PHP package may not exist on
/// every distro, and the panel discovers FPM binaries at runtime instead.
const BASE_PACKAGES: &[&str] = &[
    "nginx",
    "php-fpm",
    "php-mysql",
    "php-cli",
    "mariadb-server",
    "redis-server",
    "ufw",
    "fail2ban",
    "ca-certificates",
];

/// Per-distro package lists. Newer releases currently share the base list;
/// versioned PHP/DB packages are intentionally excluded so an install never
/// fails because a package is unavailable on a given release.
pub fn base_packages(distro: &str) -> Vec<&'static str> {
    match distro {
        "ubuntu-22.04" | "ubuntu-24.04" | "ubuntu-26.04" | "debian-12" | "debian-13" => {
            BASE_PACKAGES.to_vec()
        }
        _ => Vec::new(),
    }
}

/// Parse the optional `SYNTHPANEL_EXTRA_PACKAGES` override (space or comma
/// separated). It is never required; an empty value adds nothing.
fn parse_extra_packages(value: &str) -> Vec<String> {
    value
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn extra_packages() -> Vec<String> {
    parse_extra_packages(&std::env::var("SYNTHPANEL_EXTRA_PACKAGES").unwrap_or_default())
}

fn install_packages(skip: bool, distro: &str) -> Result<()> {
    if skip {
        info!("skipping package installation");
        return Ok(());
    }
    let mut packages: Vec<String> = base_packages(distro)
        .into_iter()
        .map(str::to_string)
        .collect();
    if packages.is_empty() {
        bail!("no package list for distro `{distro}`");
    }
    packages.extend(extra_packages());
    info!(distro, "installing base packages");
    run("apt-get", &["update", "-qq"])?;
    let borrowed: Vec<&str> = packages.iter().map(String::as_str).collect();
    let mut args: Vec<&str> = vec!["-y", "-qq", "install", "--no-install-recommends", "--"];
    args.extend(borrowed);
    run("apt-get", &args)?;
    info!("base packages installed");
    Ok(())
}

fn create_layout() -> Result<()> {
    info!("creating directories and users");
    for dir in [
        "/var/www",
        "/var/lib/synthpanel",
        "/var/log/synthpanel",
        "/var/backups/synthpanel",
        "/etc/synthpanel",
        "/run/synthpanel",
    ] {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {dir}"))?;
    }
    if !run_ok("id", &["synthpanel"]) {
        run("useradd", &["-r", "-s", "/usr/sbin/nologin", "synthpanel"])
            .with_context(|| "cannot create the synthpanel user")?;
    }
    if !run_ok("getent", &["group", "synthpanel"]) {
        run("groupadd", &["synthpanel"]).ok();
    }

    // The web process writes the panel certificate, so the TLS directory has
    // to belong to it. Without this the web service cannot start.
    std::fs::create_dir_all("/etc/synthpanel/tls")
        .with_context(|| "cannot create /etc/synthpanel/tls")?;
    run("chown", &["synthpanel:synthpanel", "/etc/synthpanel/tls"])
        .with_context(|| "cannot hand /etc/synthpanel/tls to the web user")?;
    run("chmod", &["0750", "/etc/synthpanel/tls"]).ok();
    for dir in [
        "/var/lib/synthpanel",
        "/var/log/synthpanel",
        "/var/backups/synthpanel",
    ] {
        run("chown", &["synthpanel:synthpanel", dir]).ok();
    }
    Ok(())
}

fn install_binaries() -> Result<()> {
    let source_dir = Path::new("/opt/synthpanel");
    if !source_dir.exists() {
        bail!(
            "/opt/synthpanel is missing; the bootstrap script is expected to download the \
             binaries there before invoking the installer"
        );
    }
    for name in ["synthpanel-core", "synthpanel-web", "synthpanel"] {
        let src = source_dir.join(name);
        let dst = Path::new("/usr/local/bin").join(name);
        if src.exists() {
            std::fs::copy(&src, &dst)
                .with_context(|| format!("cannot copy {} to {}", src.display(), dst.display()))?;
            std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    info!("binaries installed");
    Ok(())
}

fn write_systemd_units(skip: bool) -> Result<()> {
    if skip {
        info!("skipping systemd wiring");
        return Ok(());
    }
    info!("writing systemd units");
    std::fs::write(
        "/etc/systemd/system/synthpanel-core.service",
        r#"
[Unit]
Description=SynthPanel privileged core
Wants=network-online.target
After=network-online.target

[Service]
User=root
Group=root
Type=simple
EnvironmentFile=/etc/synthpanel/daemon.env
ExecStart=/usr/local/bin/synthpanel-core
Restart=on-failure
RestartSec=2

[Install]
WantedBy=multi-user.target
"#,
    )?;
    std::fs::write(
        "/etc/systemd/system/synthpanel-web.service",
        r#"
[Unit]
Description=SynthPanel web UI and API
Wants=synthpanel-core.service
After=network-online.target synthpanel-core.service

[Service]
User=synthpanel
Group=synthpanel
Type=simple
EnvironmentFile=/etc/synthpanel/daemon.env
ExecStart=/usr/local/bin/synthpanel-web
Restart=on-failure
RestartSec=2

[Install]
WantedBy=multi-user.target
"#,
    )?;

    std::fs::write("/etc/synthpanel/daemon.env", generate_daemon_env())?;
    run("systemctl", &["daemon-reload"])?;
    run("systemctl", &["enable", "synthpanel-core.service"])?;
    run("systemctl", &["enable", "synthpanel-web.service"])?;
    info!("systemd units installed and enabled");
    Ok(())
}

/// Reuse an existing secret when present so re-running the installer does not
/// invalidate the token or key of a live installation.
fn read_or_create_secret(path: &Path, bytes: usize) -> String {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let existing = existing.trim();
        if !existing.is_empty() {
            return existing.to_string();
        }
    }
    let generated = generate_secure_token(bytes);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // The token is the master credential of the core: it lives in the run
    // directory, which is a tmpfs and not one of the file-manager scopes.
    std::fs::write(path, &generated).ok();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).ok();
    generated
}

fn generate_daemon_env() -> String {
    // Ensure both secrets exist before the units are enabled; the values are
    // read from these files at runtime, not embedded in the env file.
    read_or_create_secret(Path::new("/run/synthpanel/ipc-token"), 32);
    read_or_create_secret(Path::new("/var/lib/synthpanel/secret.key"), 32);
    concat!(
        "SYNTHPANEL_ETC_DIR=/etc/synthpanel\n",
        "SYNTHPANEL_LIB_DIR=/var/lib/synthpanel\n",
        "SYNTHPANEL_LOG_DIR=/var/log/synthpanel\n",
        "SYNTHPANEL_RUN_DIR=/run/synthpanel\n",
        "SYNTHPANEL_WWW_DIR=/var/www\n",
        "SYNTHPANEL_BACKUP_DIR=/var/backups/synthpanel\n",
        "SYNTHPANEL_SOCKET_PATH=/run/synthpanel/core.sock\n",
        "SYNTHPANEL_IPC_TOKEN_FILE=/run/synthpanel/ipc-token\n",
        "SYNTHPANEL_TLS_DIR=/etc/synthpanel/tls\n",
        "SYNTHPANEL_SECRET_KEY_FILE=/var/lib/synthpanel/secret.key\n",
        "SYNTHPANEL_CORE_SOCKET=/run/synthpanel/core.sock\n",
        "SYNTHPANEL_WEB_ADDR=0.0.0.0:8443\n",
        "SYNTHPANEL_SESSION_TTL=86400\n",
        "SYNTHPANEL_SECURE_COOKIES=1\n",
        "RUST_LOG=info\n",
    )
    .to_string()
}

fn generate_secure_token(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

fn apply_baseline_security(skip_packages: bool) -> Result<()> {
    if skip_packages {
        info!("skipping firewall baseline because packages were skipped");
        return Ok(());
    }
    info!("applying firewall baseline");
    run("ufw", &["default", "deny", "incoming"])?;
    run("ufw", &["allow", "22/tcp"])?;
    run("ufw", &["allow", "80"])?;
    run("ufw", &["allow", "443"])?;
    run("ufw", &["allow", "8443"])?;
    run("ufw", &["--force", "enable"]).ok();
    run("systemctl", &["enable", "fail2ban"]).ok();
    Ok(())
}

async fn create_admin(args: &Args) -> Result<String> {
    let email = match &args.admin_email {
        Some(email) => email.clone(),
        None if args.unattended => "admin@localhost".to_string(),
        None => prompt("admin email", "admin@localhost")?,
    };
    let password = match &args.admin_password {
        Some(password) => password.clone(),
        None if args.unattended => generate_secure_token(12),
        None => prompt_secure("admin password")?,
    };
    if password.len() < 12 {
        bail!("the admin password must be at least 12 characters long");
    }
    let hash = synthpanel_core::auth::hash_password(&password)?;

    // Connect to core through the IPC client and create the user.
    let socket = Path::new("/run/synthpanel/core.sock");
    if !socket.exists() {
        warn!("core socket is not up yet; deferring admin user creation to the first-run wizard");
        return Ok(email);
    }
    let token = std::fs::read_to_string("/run/synthpanel/ipc-token")
        .context("cannot read the IPC token")?;
    let mut client = synthpanel_ipc::Client::connect(socket, token.trim().to_string()).await?;
    let existing: Option<serde_json::Value> = client
        .call(
            synthpanel_ipc::methods::USERS_GET_BY_EMAIL,
            serde_json::json!({ "email": email }),
        )
        .await?;
    if existing.is_some() {
        info!(email, "admin user already exists; skipping");
        return Ok(email);
    }
    let user: serde_json::Value = client
        .call(
            synthpanel_ipc::methods::USERS_CREATE,
            serde_json::json!({
                "email": email,
                "name": "Administrator",
                "password_hash": hash,
                "role": "admin",
                "quota_sites": 25,
                "quota_dbs": 25,
            }),
        )
        .await?;
    if user.is_null() {
        bail!("core refused to create the admin user");
    }
    info!(email, "created the admin account");
    Ok(email)
}

fn prompt(prompt_label: &str, default: &str) -> Result<String> {
    use std::io::Write;
    print!("{} [{}]: ", prompt_label, default);
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let line = line.trim().to_string();
    Ok(if line.is_empty() {
        default.to_string()
    } else {
        line
    })
}

fn prompt_secure(label: &str) -> Result<String> {
    if std::env::args().any(|arg| arg == "--unattended") {
        return Ok(generate_secure_token(16));
    }
    match dialoguer::Password::new().with_prompt(label).interact() {
        Ok(value) if value.len() >= 12 => Ok(value),
        Ok(_) => bail!("the password must be at least 12 characters long"),
        Err(error) => bail!("cannot read password: {error}"),
    }
}

fn show_summary(email: &str, password: &Option<String>) {
    let ip = local_ip();
    println!();
    println!("=========================================");
    println!(" SynthPanel installed");
    println!("=========================================");
    println!();
    println!("  Panel URL:      https://{ip}:8443");
    println!("  Admin email:    {email}");
    if let Some(password) = password {
        println!("  Admin password: {password}");
    }
    println!();
    println!("  Sign in with the credentials above. A Let's Encrypt");
    println!("  certificate can be issued later via Settings -> Panel TLS");
    println!("  once a hostname + DNS record are in place.");
    println!();
}

fn local_ip() -> String {
    use std::net::{SocketAddr, UdpSocket};
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(socket) => socket,
        Err(_) => return "localhost".into(),
    };
    let target: SocketAddr = "8.8.8.8:80".parse().unwrap();
    match socket.connect(target) {
        Ok(()) => socket
            .local_addr()
            .map(|addr| addr.ip().to_string())
            .unwrap_or_else(|_| "localhost".into()),
        Err(_) => "localhost".into(),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let args = Args::parse();

    require_root()?;
    let distro = detect_distro()?;
    info!(distro, "supported distro detected");

    install_packages(args.skip_packages, &distro)?;
    create_layout()?;
    install_binaries()?;
    write_systemd_units(args.skip_systemd)?;
    apply_baseline_security(args.skip_packages)?;

    let email = create_admin(&args).await?;
    show_summary(&email, &args.admin_password);
    info!("installer finished");
    Ok(())
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_current_releases() {
        for (release, expected) in [
            ("ID=ubuntu\nVERSION_ID=\"22.04\"\n", "ubuntu-22.04"),
            ("ID=ubuntu\nVERSION_ID=\"24.04\"\n", "ubuntu-24.04"),
            ("ID=ubuntu\nVERSION_ID=\"26.04\"\n", "ubuntu-26.04"),
            ("ID=debian\nVERSION_ID=\"12\"\n", "debian-12"),
            ("ID=debian\nVERSION_ID=\"13\"\n", "debian-13"),
        ] {
            let (id, version) = parse_os_release(release);
            check_distro(&id, &version).unwrap();
            assert_eq!(format!("{id}-{version}"), expected);
        }
    }

    #[test]
    fn rejects_other_releases() {
        assert!(check_distro("ubuntu", "25.10").is_err());
        assert!(check_distro("debian", "11").is_err());
        assert!(check_distro("fedora", "42").is_err());
    }

    #[test]
    fn every_supported_distro_has_a_safe_package_list() {
        for distro in [
            "ubuntu-22.04",
            "ubuntu-24.04",
            "ubuntu-26.04",
            "debian-12",
            "debian-13",
        ] {
            let packages = base_packages(distro);
            assert!(packages.contains(&"nginx"));
            assert!(packages.contains(&"php-fpm"));
            assert!(packages.contains(&"mariadb-server"));
            // Neither a cache nor an MTA is part of the base install.
            assert!(!packages.iter().any(|p| p.contains("varnish")));
            assert!(
                !packages
                    .iter()
                    .any(|p| p.contains("mail") || p.contains("postfix"))
            );
        }
        assert!(base_packages("fedora-42").is_empty());
    }

    #[test]
    fn optional_extra_packages() {
        assert!(parse_extra_packages("").is_empty());
        assert_eq!(
            parse_extra_packages(" php-curl,php-xml  php-mbstring,"),
            ["php-curl", "php-xml", "php-mbstring"]
        );
    }
}
