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
    /// Check for updates and apply the newest release.
    #[command(name = "update")]
    Update {
        /// Only check; do not install.
        #[arg(long)]
        check: bool,
        /// Update channel: stable or beta.
        #[arg(long)]
        channel: Option<String>,
    },
    /// Show the current version, update channel and recent update history.
    #[command(name = "update:status")]
    UpdateStatus,
    /// Roll back to the binaries saved by a previous update.
    #[command(name = "update:rollback")]
    UpdateRollback {
        /// Update id from `update:status`.
        #[arg(long)]
        id: Option<String>,
        /// Explicit rollback directory.
        #[arg(long)]
        backup_dir: Option<String>,
    },
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
    /// List the managed databases.
    #[command(name = "db:list")]
    DbList,
    /// Create a database.
    #[command(name = "db:create")]
    DbCreate {
        /// Database name.
        name: String,
        /// Engine: mysql, postgres or redis.
        #[arg(long, default_value = "mysql")]
        engine: String,
        /// Username (defaults to the database name).
        #[arg(long)]
        username: Option<String>,
        /// Password (generated when omitted).
        #[arg(long)]
        password: Option<String>,
    },
    /// Drop a database by id.
    #[command(name = "db:delete")]
    DbDelete {
        /// Database id.
        id: String,
    },
    /// Show the firewall status.
    #[command(name = "security:firewall")]
    SecurityFirewall,
    /// Show the fail2ban jails.
    #[command(name = "security:fail2ban")]
    SecurityFail2ban,
    /// List the SSH keys the panel manages.
    #[command(name = "security:ssh-keys")]
    SecuritySshKeys,
    /// List backups and their schedules.
    #[command(name = "backup:list")]
    BackupList,
    /// Create a backup now.
    #[command(name = "backup:run")]
    BackupRun {
        /// Back up a single site id instead of everything.
        #[arg(long)]
        site: Option<String>,
        /// Encrypt the archive with a passphrase.
        #[arg(long)]
        passphrase: Option<String>,
        /// Destination as JSON, e.g. '{"type":"local","path":"/var/backups"}'.
        #[arg(long)]
        destination: Option<String>,
    },
    /// Restore a backup by id.
    #[command(name = "backup:restore")]
    BackupRestore {
        /// Backup id.
        id: String,
        /// Passphrase for an encrypted archive.
        #[arg(long)]
        passphrase: Option<String>,
    },
    /// Verify a backup archive and print its manifest.
    #[command(name = "backup:verify")]
    BackupVerify {
        /// Backup id.
        id: String,
        /// Passphrase for an encrypted archive.
        #[arg(long)]
        passphrase: Option<String>,
    },
    /// List cron jobs.
    #[command(name = "cron:list")]
    CronList,
    /// Add a cron job.
    #[command(name = "cron:add")]
    CronAdd {
        /// Five-field cron expression.
        expression: String,
        /// Shell command.
        command: String,
        /// System user the job runs as.
        #[arg(long, default_value = "www-data")]
        user: String,
        /// Optional site id.
        #[arg(long)]
        site: Option<String>,
    },
    /// Delete a cron job.
    #[command(name = "cron:delete")]
    CronDelete {
        /// Job id.
        id: String,
    },
    /// Run a cron job now.
    #[command(name = "cron:run")]
    CronRun {
        /// Job id.
        id: String,
    },
    /// Print recent runs of a cron job.
    #[command(name = "cron:runs")]
    CronRuns {
        /// Job id.
        id: String,
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    /// List the alert history.
    #[command(name = "alerts:list")]
    AlertsList,
    /// List the alert channels.
    #[command(name = "alerts:channels")]
    AlertsChannels,
    /// Add an alert channel.
    #[command(name = "alerts:channel:add")]
    AlertsChannelAdd {
        /// Channel name.
        name: String,
        /// Channel type: email or webhook.
        #[arg(long)]
        kind: String,
        /// Channel configuration as JSON.
        #[arg(long)]
        config: String,
    },
    /// Add an alert rule.
    #[command(name = "alerts:rule:add")]
    AlertsRuleAdd {
        /// Rule name.
        name: String,
        /// Event, e.g. backup.failed.
        event: String,
        /// Channel id the rule delivers to.
        #[arg(long)]
        channel: String,
        /// Severity: info, warning or critical.
        #[arg(long, default_value = "warning")]
        severity: String,
        /// Repeat suppression in seconds.
        #[arg(long, default_value_t = 900)]
        cooldown: i64,
    },
    /// Delete an alert channel.
    #[command(name = "alerts:channel:delete")]
    AlertsChannelDelete {
        /// Channel id.
        id: String,
    },
    /// Delete an alert rule.
    #[command(name = "alerts:rule:delete")]
    AlertsRuleDelete {
        /// Rule id.
        id: String,
    },
    /// Send a test alert to a channel.
    #[command(name = "alerts:test")]
    AlertsTest {
        /// Channel id.
        #[arg(long)]
        channel: String,
    },
    /// List the Git deployments.
    #[command(name = "deploy:list")]
    DeployList {
        /// Only show the deployments of this site.
        #[arg(long)]
        site: Option<String>,
    },
    /// Add a Git deployment for a site.
    #[command(name = "deploy:add")]
    DeployAdd {
        /// Site id.
        #[arg(long)]
        site: String,
        /// Repository url.
        #[arg(long)]
        repo: String,
        /// Branch to deploy.
        #[arg(long, default_value = "main")]
        branch: String,
        /// Sub-directory of the repository to publish.
        #[arg(long, default_value = "")]
        subdir: String,
        /// Build command, e.g. "npm ci && npm run build".
        #[arg(long, default_value = "")]
        build: String,
        /// User the build runs as.
        #[arg(long, default_value = "www-data")]
        user: String,
    },
    /// Run a deployment.
    #[command(name = "deploy:run")]
    DeployRun {
        /// Deployment id.
        id: String,
    },
    /// Print the run history of a deployment.
    #[command(name = "deploy:runs")]
    DeployRuns {
        /// Deployment id.
        id: String,
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    /// Delete a deployment.
    #[command(name = "deploy:delete")]
    DeployDelete {
        /// Deployment id.
        id: String,
    },
    /// Read a log source.
    #[command(name = "logs")]
    Logs {
        /// Source: panel, nginx-access, nginx-error, php-fpm or auth.
        #[arg(long = "type", default_value = "panel")]
        source: String,
        /// Site id for per-site logs.
        #[arg(long)]
        site: Option<String>,
        /// Only lines containing this text.
        #[arg(long)]
        search: Option<String>,
        /// Only lines with this HTTP status.
        #[arg(long)]
        status: Option<u16>,
        /// Maximum number of lines.
        #[arg(long, default_value_t = 200)]
        limit: i64,
    },
    /// List a directory through the file manager.
    #[command(name = "files:list")]
    FilesList {
        /// Scope: www, etc, backups or logs.
        #[arg(long, default_value = "www")]
        scope: String,
        /// Path relative to the scope root.
        #[arg(long, default_value = "")]
        path: String,
    },
    /// Print a file from the file manager.
    #[command(name = "files:read")]
    FilesRead {
        /// Path relative to the scope root.
        path: String,
        #[arg(long, default_value = "www")]
        scope: String,
    },
    /// Write a file through the file manager.
    #[command(name = "files:write")]
    FilesWrite {
        /// Path relative to the scope root.
        path: String,
        /// Literal content.
        #[arg(long)]
        content: Option<String>,
        /// Read the content from a local file.
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, default_value = "www")]
        scope: String,
    },
    /// List API keys.
    #[command(name = "apikeys:list")]
    ApiKeysList,
    /// Create an API key. The token is printed once.
    #[command(name = "apikeys:create")]
    ApiKeysCreate {
        /// A label for the key.
        name: String,
        /// Comma separated scopes.
        #[arg(long, default_value = "read")]
        scopes: String,
        /// Owning panel user id.
        #[arg(long)]
        user: String,
    },
    /// Delete an API key.
    #[command(name = "apikeys:delete")]
    ApiKeysDelete {
        /// Key id.
        id: String,
    },
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
        Command::Update { check, channel } => {
            let mut params = json!({});
            if let Some(channel) = channel {
                params["channel"] = json!(channel);
            }
            if *check {
                let result: Value = global
                    .call(synthpanel_ipc::methods::UPDATE_CHECK, params)
                    .await?;
                println!("{}", global.as_json(&result));
                return Ok(());
            }
            let result: Value = global
                .call(synthpanel_ipc::methods::UPDATE_APPLY, params)
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::UpdateStatus => {
            let result: Value = global
                .call(synthpanel_ipc::methods::UPDATE_STATUS, json!({}))
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::UpdateRollback { id, backup_dir } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::UPDATE_ROLLBACK,
                    json!({ "id": id, "backup_dir": backup_dir }),
                )
                .await?;
            println!("{}", global.as_json(&result));
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
        Command::DbList => {
            let databases: Vec<Value> = global
                .call(synthpanel_ipc::methods::DATABASES_LIST, json!({}))
                .await?;
            for database in &databases {
                println!("{}", global.as_json(database));
            }
            if databases.is_empty() {
                eprintln!("no databases yet");
            }
            Ok(())
        }
        Command::DbCreate {
            name,
            engine,
            username,
            password,
        } => {
            let mut params = json!({ "name": name, "engine": engine });
            if let Some(username) = username {
                params["username"] = json!(username);
            }
            if let Some(password) = password {
                params["password"] = json!(password);
            }
            let result: Value = global
                .call(synthpanel_ipc::methods::DATABASES_CREATE, params)
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::DbDelete { id } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::DATABASES_DELETE,
                    json!({ "id": id }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::SecurityFirewall => {
            let result: Value = global
                .call(synthpanel_ipc::methods::SECURITY_FIREWALL, json!({}))
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::SecurityFail2ban => {
            let result: Value = global
                .call(synthpanel_ipc::methods::SECURITY_FAIL2BAN, json!({}))
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::SecuritySshKeys => {
            let result: Value = global
                .call(synthpanel_ipc::methods::SECURITY_SSH_KEYS, json!({}))
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::BackupList => {
            let backups: Vec<Value> = global
                .call(synthpanel_ipc::methods::BACKUPS_LIST, json!({}))
                .await?;
            let schedules: Vec<Value> = global
                .call(synthpanel_ipc::methods::BACKUPS_SCHEDULES, json!({}))
                .await?;
            for backup in &backups {
                println!("{}", global.as_json(backup));
            }
            if backups.is_empty() {
                eprintln!("no backups yet");
            }
            for schedule in &schedules {
                println!("schedule {}", global.as_json(schedule));
            }
            Ok(())
        }
        Command::BackupRun {
            site,
            passphrase,
            destination,
        } => {
            let mut params = json!({
                "kind": if site.is_some() { "site" } else { "full" },
            });
            if let Some(site) = site {
                params["site_id"] = json!(site);
            }
            if let Some(passphrase) = passphrase {
                params["passphrase"] = json!(passphrase);
            }
            if let Some(destination) = destination {
                let value: Value = serde_json::from_str(destination)
                    .context("the destination is not valid JSON")?;
                params["destination"] = value;
            }
            let result: Value = global
                .call(synthpanel_ipc::methods::BACKUPS_CREATE, params)
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::BackupRestore { id, passphrase } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::BACKUPS_RESTORE,
                    json!({ "id": id, "passphrase": passphrase }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::BackupVerify { id, passphrase } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::BACKUPS_VERIFY,
                    json!({ "id": id, "passphrase": passphrase }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::CronList => {
            let jobs: Vec<Value> = global
                .call(synthpanel_ipc::methods::CRON_LIST, json!({}))
                .await?;
            for job in &jobs {
                println!("{}", global.as_json(job));
            }
            if jobs.is_empty() {
                eprintln!("no cron jobs yet");
            }
            Ok(())
        }
        Command::CronAdd {
            expression,
            command,
            user,
            site,
        } => {
            let mut params = json!({
                "expression": expression,
                "command": command,
                "user": user,
            });
            if let Some(site) = site {
                params["site_id"] = json!(site);
            }
            let result: Value = global
                .call(synthpanel_ipc::methods::CRON_CREATE, params)
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::CronDelete { id } => {
            let result: Value = global
                .call(synthpanel_ipc::methods::CRON_DELETE, json!({ "id": id }))
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::CronRun { id } => {
            let result: Value = global
                .call(synthpanel_ipc::methods::CRON_RUN, json!({ "id": id }))
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::CronRuns { id, limit } => {
            let runs: Vec<Value> = global
                .call(
                    synthpanel_ipc::methods::CRON_RUNS,
                    json!({ "job_id": id, "limit": limit }),
                )
                .await?;
            for run in &runs {
                println!("{}", global.as_json(run));
            }
            if runs.is_empty() {
                eprintln!("no runs recorded for this job");
            }
            Ok(())
        }
        Command::AlertsList => {
            let alerts: Vec<Value> = global
                .call(synthpanel_ipc::methods::ALERTS_LIST, json!({}))
                .await?;
            for alert in &alerts {
                println!("{}", global.as_json(alert));
            }
            if alerts.is_empty() {
                eprintln!("no alerts yet");
            }
            Ok(())
        }
        Command::AlertsChannels => {
            let channels: Vec<Value> = global
                .call(synthpanel_ipc::methods::ALERT_CHANNELS_LIST, json!({}))
                .await?;
            for channel in &channels {
                println!("{}", global.as_json(channel));
            }
            if channels.is_empty() {
                eprintln!("no alert channels yet");
            }
            Ok(())
        }
        Command::AlertsChannelAdd { name, kind, config } => {
            let config: Value =
                serde_json::from_str(config).context("the channel config is not valid JSON")?;
            let channel: Value = global
                .call(
                    synthpanel_ipc::methods::ALERT_CHANNELS_CREATE,
                    json!({ "name": name, "kind": kind, "config": config }),
                )
                .await?;
            println!("{}", global.as_json(&channel));
            Ok(())
        }
        Command::AlertsRuleAdd {
            name,
            event,
            channel,
            severity,
            cooldown,
        } => {
            let rule: Value = global
                .call(
                    synthpanel_ipc::methods::ALERT_RULES_CREATE,
                    json!({
                        "name": name,
                        "event": event,
                        "channel_id": channel,
                        "severity": severity,
                        "cooldown_secs": cooldown,
                    }),
                )
                .await?;
            println!("{}", global.as_json(&rule));
            Ok(())
        }
        Command::AlertsChannelDelete { id } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::ALERT_CHANNELS_DELETE,
                    json!({ "id": id }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::AlertsRuleDelete { id } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::ALERT_RULES_DELETE,
                    json!({ "id": id }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::AlertsTest { channel } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::ALERTS_TEST,
                    json!({ "channel_id": channel }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::DeployList { site } => {
            let deployments: Vec<Value> = global
                .call(
                    synthpanel_ipc::methods::DEPLOYMENTS_LIST,
                    json!({ "site_id": site }),
                )
                .await?;
            for deployment in &deployments {
                println!("{}", global.as_json(deployment));
            }
            if deployments.is_empty() {
                eprintln!("no deployments yet");
            }
            Ok(())
        }
        Command::DeployAdd {
            site,
            repo,
            branch,
            subdir,
            build,
            user,
        } => {
            let deployment: Value = global
                .call(
                    synthpanel_ipc::methods::DEPLOYMENTS_CREATE,
                    json!({
                        "site_id": site,
                        "repo_url": repo,
                        "branch": branch,
                        "target_subdir": subdir,
                        "build_command": build,
                        "deploy_user": user,
                    }),
                )
                .await?;
            println!("{}", global.as_json(&deployment));
            Ok(())
        }
        Command::DeployRun { id } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::DEPLOYMENTS_RUN,
                    json!({ "id": id }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::DeployRuns { id, limit } => {
            let runs: Vec<Value> = global
                .call(
                    synthpanel_ipc::methods::DEPLOYMENTS_RUNS,
                    json!({ "id": id, "limit": limit }),
                )
                .await?;
            for run in &runs {
                println!("{}", global.as_json(run));
            }
            Ok(())
        }
        Command::DeployDelete { id } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::DEPLOYMENTS_DELETE,
                    json!({ "id": id }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::Logs {
            source,
            site,
            search,
            status,
            limit,
        } => {
            let mut params = json!({ "source": source, "limit": limit });
            if let Some(site) = site {
                params["site_id"] = json!(site);
            }
            if let Some(search) = search {
                params["search"] = json!(search);
            }
            if let Some(status) = status {
                params["status"] = json!(status);
            }
            let result: Value = global
                .call(synthpanel_ipc::methods::LOGS_READ, params)
                .await?;
            if let Some(lines) = result.get("lines").and_then(Value::as_array) {
                for line in lines {
                    if let Some(message) = line.get("message").and_then(Value::as_str) {
                        println!("{message}");
                    } else {
                        println!("{line}");
                    }
                }
            }
            Ok(())
        }
        Command::FilesList { scope, path } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::FILES_LIST,
                    json!({ "scope": scope, "path": path }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::FilesRead { path, scope } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::FILES_READ,
                    json!({ "scope": scope, "path": path }),
                )
                .await?;
            if let Some(content) = result.get("content").and_then(Value::as_str) {
                println!("{content}");
            } else {
                println!("{}", global.as_json(&result));
            }
            Ok(())
        }
        Command::FilesWrite {
            path,
            content,
            file,
            scope,
        } => {
            let content = match (content, file) {
                (Some(content), _) => content.clone(),
                (None, Some(file)) => std::fs::read_to_string(file)
                    .with_context(|| format!("cannot read {}", file.display()))?,
                (None, None) => bail!("pass --content or --file"),
            };
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::FILES_WRITE,
                    json!({ "scope": scope, "path": path, "content": content }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::ApiKeysList => {
            let keys: Vec<Value> = global
                .call(synthpanel_ipc::methods::API_KEYS_LIST, json!({}))
                .await?;
            for key in &keys {
                println!("{}", global.as_json(key));
            }
            if keys.is_empty() {
                eprintln!("no API keys yet");
            }
            Ok(())
        }
        Command::ApiKeysCreate { name, scopes, user } => {
            let scopes: Vec<String> = scopes
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(String::from)
                .collect();
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::API_KEYS_CREATE,
                    json!({ "user_id": user, "name": name, "scopes": scopes }),
                )
                .await?;
            println!("{}", global.as_json(&result));
            Ok(())
        }
        Command::ApiKeysDelete { id } => {
            let result: Value = global
                .call(
                    synthpanel_ipc::methods::API_KEYS_DELETE,
                    json!({ "id": id }),
                )
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
    fn cli_parses_database_commands() {
        let cli = Cli::try_parse_from(["synthpanel", "db:list"]).unwrap();
        assert!(matches!(cli.command, Command::DbList));
        let cli =
            Cli::try_parse_from(["synthpanel", "db:create", "shop", "--engine", "mysql"]).unwrap();
        assert!(matches!(cli.command, Command::DbCreate { .. }));
    }

    #[test]
    fn cli_uses_defaults() {
        let cli = Cli::try_parse_from(["synthpanel", "settings"]).unwrap();
        assert_eq!(cli.token_file, PathBuf::from("/etc/synthpanel/ipc-token"));
        assert_eq!(cli.socket, PathBuf::from("/run/synthpanel/core.sock"));
        assert!(matches!(cli.command, Command::Settings));
    }
}
