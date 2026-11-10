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

fn detect_distro() -> Result<String> {
    let output = Command::new("cat")
        .arg("/etc/os-release")
        .output()
        .context("cannot read /etc/os-release")?;
    let text = String::from_utf8_lossy(&output.stdout);
    let id = text
        .lines()
        .find(|line| line.starts_with("ID="))
        .map(|line| line.trim_start_matches("ID=").trim_matches('"').to_string())
        .unwrap_or_default();
    let version = text
        .lines()
        .find(|line| line.starts_with("VERSION_ID="))
        .map(|line| {
            line.trim_start_matches("VERSION_ID=")
                .trim_matches('"')
                .to_string()
        })
        .unwrap_or_default();
    if !matches!(id.as_str(), "ubuntu" | "debian") {
        bail!("unsupported distro `{id}`; SynthPanel supports Ubuntu LTS and Debian 12+");
    }
    if id == "ubuntu" && !matches!(version.as_str(), "22.04" | "24.04") {
        bail!("Ubuntu `{version}` is not an LTS release; use 22.04 or 24.04");
    }
    Ok(format!("{id}-{version}"))
}

fn install_packages(skip: bool) -> Result<()> {
    if skip {
        info!("skipping package installation");
        return Ok(());
    }
    info!("installing base packages");
    run("apt-get", &["update", "-qq"])?;
    run(
        "apt-get",
        &[
            "-y",
            "-qq",
            "install",
            "--no-install-recommends",
            "nginx",
            "php-fpm",
            "php-mysql",
            "php-cli",
            "mariadb-server",
            "redis-server",
            "ufw",
            "fail2ban",
            "ca-certificates",
        ],
    )?;
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

fn generate_daemon_env() -> String {
    let token = generate_secure_token(32);
    let key = generate_secure_token(32);
    std::fs::write("/etc/synthpanel/ipc-token", token.clone()).ok();
    std::fs::write("/etc/synthpanel/secret.key", key).ok();
    concat!(
        "SYNTHPANEL_ETC_DIR=/etc/synthpanel\n",
        "SYNTHPANEL_LIB_DIR=/var/lib/synthpanel\n",
        "SYNTHPANEL_LOG_DIR=/var/log/synthpanel\n",
        "SYNTHPANEL_RUN_DIR=/run/synthpanel\n",
        "SYNTHPANEL_WWW_DIR=/var/www\n",
        "SYNTHPANEL_BACKUP_DIR=/var/backups/synthpanel\n",
        "SYNTHPANEL_SOCKET_PATH=/run/synthpanel/core.sock\n",
        "SYNTHPANEL_IPC_TOKEN_FILE=/etc/synthpanel/ipc-token\n",
        "SYNTHPANEL_TLS_DIR=/etc/synthpanel/tls\n",
        "SYNTHPANEL_SECRET_KEY_FILE=/etc/synthpanel/secret.key\n",
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

fn apply_baseline_security() -> Result<()> {
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
    let token = std::fs::read_to_string("/etc/synthpanel/ipc-token")
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

    install_packages(args.skip_packages)?;
    create_layout()?;
    install_binaries()?;
    write_systemd_units(args.skip_systemd)?;
    apply_baseline_security()?;

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
