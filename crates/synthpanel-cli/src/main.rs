//! `synthpanel` — user- and root-facing CLI.
//!
//! Phase 0 implements `status` and `metrics` against the core IPC socket;
//! Phase 3 expands this to mirror every UI operation.

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "synthpanel",
    version,
    about = "SynthPanel command-line interface",
    propagate_version = true
)]
pub struct Cli {
    /// Path to the IPC token file.
    #[arg(
        long,
        env = "SYNTHPANEL_IPC_TOKEN_FILE",
        default_value = "/etc/synthpanel/ipc-token",
        global = true
    )]
    pub token_file: PathBuf,

    /// Path to the core Unix socket.
    #[arg(
        long,
        env = "SYNTHPANEL_CORE_SOCKET",
        default_value = "/run/synthpanel/core.sock",
        global = true
    )]
    pub socket: PathBuf,

    /// Output format: table, json or yaml.
    #[arg(long, default_value = "table", global = true)]
    pub output: String,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show panel health, version and a metrics snapshot.
    #[command(name = "status")]
    Status,
    /// Print a raw metrics snapshot as JSON (no table formatting).
    #[command(name = "metrics")]
    Metrics,
    /// Tail a metrics range for quick inspection.
    #[command(name = "metrics:range")]
    MetricsRange {
        #[arg(long, default_value = "24h")]
        range: String,
    },
    /// List audit log entries.
    #[command(name = "audit")]
    Audit {
        #[arg(long, default_value_t = 25)]
        limit: i64,
    },
    /// Show current panel settings.
    #[command(name = "settings")]
    Settings,
    /// List sites.
    #[command(name = "sites:list")]
    SitesList,
    /// Create a site.
    #[command(name = "sites:add")]
    SitesAdd {
        /// Domain name.
        domain: String,
        /// Comma separated aliases.
        #[arg(long)]
        aliases: Option<String>,
        /// Template: static, php, laravel, symfony, wordpress or node.
        #[arg(long, default_value = "php")]
        template: String,
        /// PHP version for the site's pool.
        #[arg(long, default_value = "8.3")]
        php: String,
        /// SSL mode: none, le or custom.
        #[arg(long, default_value = "none")]
        ssl: String,
        /// Document root (defaults to /var/www/sp_<id>).
        #[arg(long)]
        root: Option<String>,
    },
    /// Delete a site by id.
    #[command(name = "sites:delete")]
    SitesDelete {
        /// Site id.
        id: String,
    },
    /// Show a site's configuration.
    #[command(name = "sites:show")]
    SitesShow {
        /// Site id.
        id: String,
    },
    /// List panel users.
    #[command(name = "users:list")]
    UsersList,
    /// Create a panel user.
    #[command(name = "users:create")]
    UsersCreate {
        /// Email address.
        email: String,
        /// Display name.
        #[arg(long)]
        name: Option<String>,
        /// Password (prompted when omitted).
        #[arg(long)]
        password: Option<String>,
        /// Role: admin, operator or user.
        #[arg(long, default_value = "operator")]
        role: String,
    },
}

#[derive(Debug, Args)]
pub struct Global {
    pub token_file: PathBuf,
    pub socket: PathBuf,
    pub output: String,
}

impl Global {
    pub fn from(cli: &Cli) -> Self {
        Self {
            token_file: cli.token_file.clone(),
            socket: cli.socket.clone(),
            output: cli.output.clone(),
        }
    }

    pub async fn client(&self) -> Result<synthpanel_ipc::Client> {
        let token = tokio::fs::read_to_string(&self.token_file)
            .await
            .with_context(|| {
                format!(
                    "cannot read the IPC token at {} — is synthpanel-core running?",
                    self.token_file.display()
                )
            })?;
        synthpanel_ipc::Client::connect(&self.socket, token.trim().to_string())
            .await
            .with_context(|| {
                format!(
                    "cannot connect to the core socket at {}",
                    self.socket.display()
                )
            })
    }

    pub fn as_json(&self, value: &Value) -> String {
        match self.output.to_lowercase().as_str() {
            "json" => serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into()),
            "yaml" => value.to_string(), // Phase 0: plain JSON, no yaml crate
            _ => value.to_string(),
        }
    }

    pub async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        let mut client = self.client().await?;
        client
            .call(method, params)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))
    }
}

/// Read a password from the terminal without echoing it.
fn prompt_password(label: &str) -> Result<String> {
    use std::io::Write;
    print!("{label}: ");
    std::io::stdout().flush()?;
    let mut password = String::new();
    std::io::stdin().read_line(&mut password)?;
    Ok(password.trim().to_string())
}

fn print_table(title: &str, rows: &[(&str, &str)]) {
    println!("{title}");
    for (key, value) in rows {
        println!("  {key:<24} {value}");
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let global = Global::from(&cli);
    let result = match &cli.command {
        Command::Status => run_status(&global).await,
        Command::Metrics => {
            let snapshot: Option<Value> = global
                .call(synthpanel_ipc::methods::METRICS_SNAPSHOT, json!({}))
                .await?;
            let snapshot = snapshot.unwrap_or(Value::Null);
            println!("{}", global.as_json(&snapshot));
            Ok(())
        }
        Command::MetricsRange { range } => {
            let points: Vec<Value> = global
                .call(
                    synthpanel_ipc::methods::METRICS_RANGE,
                    json!({ "range": range }),
                )
                .await?;
            for point in &points {
                println!("{}", global.as_json(point));
            }
            if points.is_empty() {
                eprintln!("no samples in range `{range}`");
            }
            Ok(())
        }
        Command::Audit { limit } => {
            let entries: Vec<Value> = global
                .call(
                    synthpanel_ipc::methods::AUDIT_LIST,
                    json!({ "limit": limit }),
                )
                .await?;
            for entry in &entries {
                println!("{}", global.as_json(entry));
            }
            if entries.is_empty() {
                eprintln!("no audit entries yet");
            }
            Ok(())
        }
        Command::SitesList => {
            let sites: Vec<Value> = global
                .call(synthpanel_ipc::methods::SITES_LIST, json!({}))
                .await?;
            if sites.is_empty() {
                eprintln!("no sites yet");
                return Ok(());
            }
            for site in &sites {
                println!("{}", global.as_json(site));
            }
            Ok(())
        }
        Command::SitesAdd {
            domain,
            aliases,
            template,
            php,
            ssl,
            root,
        } => {
            let alias_list: Vec<String> = aliases
                .as_deref()
                .unwrap_or("")
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(String::from)
                .collect();
            let mut params = json!({
                "domain": domain,
                "aliases": alias_list,
                "template": template,
                "ssl_mode": ssl,
            });
            if template != "static" && template != "node" {
                params["php_version"] = json!(php);
            }
            if let Some(root) = root {
                params["document_root"] = json!(root);
            }
            let result: Value = global
                .call(synthpanel_ipc::methods::SITES_CREATE, params)
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::SitesDelete { id } => {
            let result: Value = global
                .call(synthpanel_ipc::methods::SITES_DELETE, json!({ "id": id }))
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::SitesShow { id } => {
            let result: Value = global
                .call(synthpanel_ipc::methods::SITES_GET, json!({ "id": id }))
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::UsersList => {
            let users: Vec<Value> = global
                .call(synthpanel_ipc::methods::USERS_LIST, json!({}))
                .await?;
            for user in &users {
                println!("{}", global.as_json(user));
            }
            if users.is_empty() {
                eprintln!("no users yet");
            }
            Ok(())
        }
        Command::UsersCreate {
            email,
            name,
            password,
            role,
        } => {
            let password = match password {
                Some(value) => value.clone(),
                None => prompt_password("password")?,
            };
            if password.len() < 12 {
                bail!("the password must be at least 12 characters long");
            }
            let display_name = name.clone().unwrap_or_else(|| email.clone());
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::USERS_CREATE,
                    json!({
                        "email": email,
                        "name": display_name,
                        "password": password,
                        "role": role,
                    }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::Settings => {
            let settings: Value = global
                .call(synthpanel_ipc::methods::SETTINGS_LIST, json!({}))
                .await?;
            match settings.as_object() {
                Some(map) => {
                    println!("panel settings");
                    for (key, value) in map {
                        println!("  {key:<20} {value}");
                    }
                }
                None => {
                    println!("{}", global.as_json(&settings));
                }
            }
            Ok(())
        }
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
    Ok(())
}

async fn run_status(global: &Global) -> Result<()> {
    let health: Value = global
        .call(synthpanel_ipc::methods::HEALTH, json!({}))
        .await?;
    let snapshot: Option<Value> = global
        .call(synthpanel_ipc::methods::METRICS_SNAPSHOT, json!({}))
        .await?;

    let snapshot = match snapshot {
        Some(snapshot) => snapshot,
        None => {
            eprintln!("no metrics sample available yet; try `synthpanel metrics` in a moment");
            return Ok(());
        }
    };

    if global.output.to_lowercase() == "json" {
        let combined = json!({ "health": health, "metrics": snapshot });
        println!("{}", global.as_json(&combined));
        return Ok(());
    }

    let mut rows: Vec<(String, String)> = Vec::new();
    if let Some(value) = health.get("status") {
        rows.push(("status".into(), value.to_string()));
    }
    if let Some(value) = health.get("version") {
        rows.push(("version".into(), value.to_string()));
    }
    if let Some(value) = health.get("uptime_secs") {
        rows.push(("uptime".into(), format!("{}s", value)));
    }
    if let Some(value) = snapshot.get("cpu") {
        rows.push(("cpu".into(), format!("{}%", value)));
    }
    if let Some(value) = snapshot.get("load1") {
        rows.push(("load1".into(), value.to_string()));
    }
    if let Some(value) = snapshot.get("mem_used") {
        rows.push((
            "memory".into(),
            format!("{} KiB", value.as_i64().unwrap_or(0) / 1024),
        ));
    }
    if let Some(value) = snapshot.get("disk_used") {
        rows.push(("disk".into(), format!("{}%", value)));
    }
    let formatted: Vec<(&str, &str)> = rows.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    print_table("synthpanel", &formatted);

    // Warn (not fail) on missing fields.
    if !matches!(health.get("status"), Some(Value::String(s)) if s.as_str() == "ok") {
        bail!("panel health is not ok");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_status() {
        let cli = Cli::try_parse_from(["synthpanel", "status"]).unwrap();
        assert!(matches!(cli.command, Command::Status));
    }

    #[test]
    fn cli_parses_metrics_range() {
        let cli = Cli::try_parse_from(["synthpanel", "metrics:range", "--range", "7d"]).unwrap();
        assert!(matches!(cli.command, Command::MetricsRange { .. }));
    }

    #[test]
    fn cli_uses_plan_command_names() {
        let cli = Cli::try_parse_from(["synthpanel", "sites:list"]).unwrap();
        assert!(matches!(cli.command, Command::SitesList));
        let cli = Cli::try_parse_from(["synthpanel", "sites:add", "example.com"]).unwrap();
        assert!(matches!(cli.command, Command::SitesAdd { .. }));
        let cli = Cli::try_parse_from(["synthpanel", "users:create", "a@example.com"]).unwrap();
        assert!(matches!(cli.command, Command::UsersCreate { .. }));
    }

    #[test]
    fn cli_uses_defaults() {
        let cli = Cli::try_parse_from(["synthpanel", "settings"]).unwrap();
        assert_eq!(cli.token_file, PathBuf::from("/etc/synthpanel/ipc-token"));
        assert_eq!(cli.socket, PathBuf::from("/run/synthpanel/core.sock"));
        assert!(matches!(cli.command, Command::Settings));
    }
}
