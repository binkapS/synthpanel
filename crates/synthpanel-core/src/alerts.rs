//! Alerting: rules, channels and delivery.
//!
//! An *event* (a failed backup, a failing cron job, a certificate about to
//! expire, a disk filling up) is matched against the enabled rules. Every
//! matching rule delivers the event to its channel — a webhook or an email —
//! and the outcome is recorded so the UI can show a history.

use anyhow::{Context, Result, bail};
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::SqlitePool;
use std::time::Duration;
use tracing::{info, warn};

/// Events the panel can raise.
pub mod events {
    pub const BACKUP_FAILED: &str = "backup.failed";
    pub const BACKUP_SUCCEEDED: &str = "backup.succeeded";
    pub const CRON_FAILED: &str = "cron.failed";
    pub const DEPLOY_FAILED: &str = "deploy.failed";
    pub const DEPLOY_SUCCEEDED: &str = "deploy.succeeded";
    pub const SSL_EXPIRING: &str = "ssl.expiring";
    pub const DISK_USAGE: &str = "disk.usage";
    pub const CPU_USAGE: &str = "cpu.usage";
    pub const UPDATE_FAILED: &str = "update.failed";
    pub const SERVICE_DOWN: &str = "service.down";
}

/// All events the panel knows about.
pub const ALL_EVENTS: [&str; 10] = [
    events::BACKUP_FAILED,
    events::BACKUP_SUCCEEDED,
    events::CRON_FAILED,
    events::DEPLOY_FAILED,
    events::DEPLOY_SUCCEEDED,
    events::SSL_EXPIRING,
    events::DISK_USAGE,
    events::CPU_USAGE,
    events::UPDATE_FAILED,
    events::SERVICE_DOWN,
];

/// Severities an event can carry.
pub const ALL_SEVERITIES: [&str; 3] = ["info", "warning", "critical"];

/// How a channel delivers an alert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    /// `email` or `webhook`.
    pub kind: String,
    pub config: Value,
    pub enabled: bool,
    pub created_at: String,
}

/// A rule that turns an event into a delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub event: String,
    pub severity: String,
    pub channel_id: String,
    pub enabled: bool,
    /// Repeat suppression in seconds.
    pub cooldown_secs: i64,
    pub created_at: String,
}

/// A delivered (or failed) alert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: String,
    pub rule_id: Option<String>,
    pub event: String,
    pub severity: String,
    pub message: String,
    /// `delivered` or `failed`.
    pub status: String,
    pub error: Option<String>,
    pub created_at: String,
    pub delivered_at: Option<String>,
}

/// List every channel.
pub async fn list_channels(pool: &SqlitePool) -> Result<Vec<Channel>> {
    let rows = sqlx::query_as::<_, ChannelRow>(
        "SELECT id, name, kind, config, enabled, created_at
         FROM alert_channels ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Channel::from).collect())
}

/// Create a channel.
pub async fn create_channel(
    pool: &SqlitePool,
    name: &str,
    kind: &str,
    config: Value,
) -> Result<Channel> {
    if name.trim().is_empty() {
        bail!("the channel name is empty");
    }
    match kind {
        "email" => validate_email_config(&config)?,
        "webhook" => validate_webhook_config(&config)?,
        other => bail!("unknown channel type `{other}`"),
    }
    let channel = Channel {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.trim().to_string(),
        kind: kind.to_string(),
        config,
        enabled: true,
        created_at: now(),
    };
    sqlx::query(
        "INSERT INTO alert_channels (id, name, kind, config, enabled, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&channel.id)
    .bind(&channel.name)
    .bind(&channel.kind)
    .bind(serde_json::to_string(&channel.config)?)
    .bind(channel.enabled)
    .bind(&channel.created_at)
    .execute(pool)
    .await?;
    Ok(channel)
}

/// Delete a channel and the rules that use it.
pub async fn delete_channel(pool: &SqlitePool, id: &str) -> Result<bool> {
    let deleted = sqlx::query("DELETE FROM alert_channels WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0;
    if deleted {
        sqlx::query("DELETE FROM alert_rules WHERE channel_id = ?")
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(deleted)
}

/// List every rule.
pub async fn list_rules(pool: &SqlitePool) -> Result<Vec<Rule>> {
    let rows = sqlx::query_as::<_, RuleRow>(
        "SELECT id, name, event, severity, channel_id, enabled, cooldown_secs, created_at
         FROM alert_rules ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Rule::from).collect())
}

/// Create a rule.
pub async fn create_rule(
    pool: &SqlitePool,
    name: &str,
    event: &str,
    severity: &str,
    channel_id: &str,
    cooldown_secs: i64,
) -> Result<Rule> {
    if name.trim().is_empty() {
        bail!("the rule name is empty");
    }
    if !ALL_EVENTS.contains(&event) {
        bail!("unknown event `{event}`");
    }
    if !ALL_SEVERITIES.contains(&severity) {
        bail!("unknown severity `{severity}`");
    }
    let channel = sqlx::query("SELECT id FROM alert_channels WHERE id = ?")
        .bind(channel_id)
        .fetch_optional(pool)
        .await?;
    if channel.is_none() {
        bail!("the channel `{channel_id}` does not exist");
    }
    let rule = Rule {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.trim().to_string(),
        event: event.to_string(),
        severity: severity.to_string(),
        channel_id: channel_id.to_string(),
        enabled: true,
        cooldown_secs: cooldown_secs.max(0),
        created_at: now(),
    };
    sqlx::query(
        "INSERT INTO alert_rules
         (id, name, event, severity, channel_id, enabled, cooldown_secs, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&rule.id)
    .bind(&rule.name)
    .bind(&rule.event)
    .bind(&rule.severity)
    .bind(&rule.channel_id)
    .bind(rule.enabled)
    .bind(rule.cooldown_secs)
    .bind(&rule.created_at)
    .execute(pool)
    .await?;
    Ok(rule)
}

/// Delete a rule.
pub async fn delete_rule(pool: &SqlitePool, id: &str) -> Result<bool> {
    let deleted = sqlx::query("DELETE FROM alert_rules WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0;
    Ok(deleted)
}

/// List recent alerts.
pub async fn list_alerts(pool: &SqlitePool, limit: i64) -> Result<Vec<Alert>> {
    let rows = sqlx::query_as::<_, AlertRow>(
        "SELECT id, rule_id, event, severity, message, status, error, created_at, delivered_at
         FROM alerts ORDER BY created_at DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Alert::from).collect())
}

/// Raise an event: deliver it to every matching rule and record the result.
///
/// Returns the number of rules that were notified.
pub async fn emit(
    pool: &SqlitePool,
    event: &str,
    severity: &str,
    message: &str,
    context: &Value,
) -> Result<usize> {
    let rules = sqlx::query_as::<_, RuleRow>(
        "SELECT id, name, event, severity, channel_id, enabled, cooldown_secs, created_at
         FROM alert_rules WHERE enabled = 1 AND event = ?",
    )
    .bind(event)
    .fetch_all(pool)
    .await?;

    if rules.is_empty() {
        return Ok(0);
    }

    let mut notified = 0;
    for rule in rules {
        let rule = Rule::from(rule);
        if in_cooldown(pool, &rule).await? {
            continue;
        }
        let channel = match load_channel(pool, &rule.channel_id).await? {
            Some(channel) => channel,
            None => {
                warn!(rule = %rule.id, "the alert channel is missing");
                continue;
            }
        };
        let alert = Alert {
            id: uuid::Uuid::new_v4().to_string(),
            rule_id: Some(rule.id.clone()),
            event: event.to_string(),
            severity: severity.to_string(),
            message: message.to_string(),
            status: "pending".into(),
            error: None,
            created_at: now(),
            delivered_at: None,
        };
        let outcome = deliver(&channel, &alert, context).await;
        let mut stored = alert;
        match outcome {
            Ok(()) => {
                stored.status = "delivered".into();
                stored.delivered_at = Some(now());
                notified += 1;
            }
            Err(error) => {
                stored.status = "failed".into();
                stored.error = Some(error.to_string());
                warn!(event, error = %error, "the alert could not be delivered");
            }
        }
        sqlx::query(
            "INSERT INTO alerts
             (id, rule_id, event, severity, message, status, error, created_at, delivered_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&stored.id)
        .bind(&stored.rule_id)
        .bind(&stored.event)
        .bind(&stored.severity)
        .bind(&stored.message)
        .bind(&stored.status)
        .bind(&stored.error)
        .bind(&stored.created_at)
        .bind(&stored.delivered_at)
        .execute(pool)
        .await?;
    }
    Ok(notified)
}

/// Whether the rule fired recently enough to be suppressed.
async fn in_cooldown(pool: &SqlitePool, rule: &Rule) -> Result<bool> {
    if rule.cooldown_secs <= 0 {
        return Ok(false);
    }
    let last: Option<(String,)> = sqlx::query_as(
        "SELECT created_at FROM alerts WHERE rule_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&rule.id)
    .fetch_optional(pool)
    .await?;
    let Some((created,)) = last else {
        return Ok(false);
    };
    let Ok(created) = chrono::DateTime::parse_from_rfc3339(&created) else {
        return Ok(false);
    };
    let elapsed = chrono::Utc::now()
        .signed_duration_since(created)
        .num_seconds();
    Ok(elapsed < rule.cooldown_secs)
}

/// Load a channel by id.
pub async fn load_channel(pool: &SqlitePool, id: &str) -> Result<Option<Channel>> {
    let row = sqlx::query_as::<_, ChannelRow>(
        "SELECT id, name, kind, config, enabled, created_at FROM alert_channels WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Channel::from))
}

/// Deliver one alert through a channel.
pub async fn deliver(channel: &Channel, alert: &Alert, context: &Value) -> Result<()> {
    if !channel.enabled {
        bail!("the channel `{}` is disabled", channel.name);
    }
    match channel.kind.as_str() {
        "webhook" => deliver_webhook(channel, alert, context).await,
        "email" => deliver_email(channel, alert, context).await,
        other => bail!("unknown channel type `{other}`"),
    }
}

/// POST the alert as JSON to a webhook URL.
async fn deliver_webhook(channel: &Channel, alert: &Alert, context: &Value) -> Result<()> {
    let url = channel
        .config
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("the webhook has no url"))?;
    let body = json!({
        "id": alert.id,
        "event": alert.event,
        "severity": alert.severity,
        "message": alert.message,
        "created_at": alert.created_at,
        "channel": channel.name,
        "context": context,
    });
    let payload = serde_json::to_vec(&body)?;

    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    if let Some(secret) = channel.config.get("secret").and_then(Value::as_str) {
        let mut mac = <Hmac<Sha256>>::new_from_slice(secret.as_bytes())
            .context("the webhook secret is invalid")?;
        mac.update(&payload);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        headers.insert(
            HeaderName::from_static("x-synthpanel-signature"),
            HeaderValue::from_str(&signature).context("the webhook signature is invalid")?,
        );
    }
    if let Some(extra) = channel.config.get("headers").and_then(Value::as_object) {
        for (name, value) in extra {
            if let Some(value) = value.as_str() {
                if let (Ok(name), Ok(value)) = (
                    HeaderName::try_from(name.as_str()),
                    HeaderValue::from_str(value),
                ) {
                    headers.insert(name, value);
                }
            }
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let response = client
        .post(url)
        .headers(headers)
        .body(payload)
        .send()
        .await
        .with_context(|| format!("cannot reach the webhook {url}"))?;
    if !response.status().is_success() {
        bail!("the webhook returned {}", response.status());
    }
    info!(channel = %channel.name, event = %alert.event, "webhook alert delivered");
    Ok(())
}

/// Send the alert by email.
async fn deliver_email(channel: &Channel, alert: &Alert, context: &Value) -> Result<()> {
    let recipients = channel
        .config
        .get("to")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if recipients.is_empty() {
        bail!("the channel has no recipients");
    }
    let from = channel
        .config
        .get("from")
        .and_then(Value::as_str)
        .or_else(|| {
            channel
                .config
                .get("smtp")
                .and_then(|smtp| smtp.get("from"))
                .and_then(Value::as_str)
        })
        .unwrap_or("synthpanel@localhost")
        .to_string();

    let mut body = format!("{}\n\n", alert.message);
    body.push_str(&format!("event:    {}\n", alert.event));
    body.push_str(&format!("severity: {}\n", alert.severity));
    body.push_str(&format!("time:     {}\n", alert.created_at));
    if let Some(object) = context.as_object() {
        for (key, value) in object {
            body.push_str(&format!("{key}: {value}\n"));
        }
    }

    let mail = crate::mailer::Mail {
        from,
        to: recipients,
        subject: format!("[synthpanel] {} — {}", alert.severity, alert.event),
        body,
    };
    crate::mailer::send(&channel.config, &mail).await
}

/// Validate an email channel configuration.
fn validate_email_config(config: &Value) -> Result<()> {
    let has_command = config.get("command").and_then(Value::as_str).is_some();
    let has_smtp = config.get("smtp").is_some();
    if !has_command && !has_smtp {
        bail!("an email channel needs a `command` or an `smtp` section");
    }
    let recipients = config.get("to").and_then(Value::as_array);
    if recipients.map(|list| list.is_empty()).unwrap_or(true) {
        bail!("an email channel needs at least one recipient");
    }
    Ok(())
}

/// Validate a webhook channel configuration.
fn validate_webhook_config(config: &Value) -> Result<()> {
    let url = config
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("a webhook needs a url"))?;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("a webhook url must be http or https");
    }
    Ok(())
}

/// The current timestamp in RFC 3339.
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ChannelRow {
    id: String,
    name: String,
    kind: String,
    config: String,
    enabled: bool,
    created_at: String,
}

impl From<ChannelRow> for Channel {
    fn from(row: ChannelRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            kind: row.kind,
            config: serde_json::from_str(&row.config).unwrap_or(Value::Null),
            enabled: row.enabled,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct RuleRow {
    id: String,
    name: String,
    event: String,
    severity: String,
    channel_id: String,
    enabled: bool,
    cooldown_secs: i64,
    created_at: String,
}

impl From<RuleRow> for Rule {
    fn from(row: RuleRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            event: row.event,
            severity: row.severity,
            channel_id: row.channel_id,
            enabled: row.enabled,
            cooldown_secs: row.cooldown_secs,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct AlertRow {
    id: String,
    rule_id: Option<String>,
    event: String,
    severity: String,
    message: String,
    status: String,
    error: Option<String>,
    created_at: String,
    delivered_at: Option<String>,
}

impl From<AlertRow> for Alert {
    fn from(row: AlertRow) -> Self {
        Self {
            id: row.id,
            rule_id: row.rule_id,
            event: row.event,
            severity: row.severity,
            message: row.message,
            status: row.status,
            error: row.error,
            created_at: row.created_at,
            delivered_at: row.delivered_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use sqlx::SqlitePool;

    async fn test_pool() -> SqlitePool {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-alerts-{}.db", uuid::Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        pool
    }

    #[test]
    fn rejects_an_unknown_event() {
        assert!(ALL_EVENTS.contains(&events::BACKUP_FAILED));
        assert!(!ALL_EVENTS.contains(&"nonsense"));
    }

    #[tokio::test]
    async fn channels_and_rules_roundtrip() {
        let pool = test_pool().await;
        let channel = create_channel(
            &pool,
            "ops",
            "webhook",
            json!({ "url": "https://example.com/hook" }),
        )
        .await
        .unwrap();
        assert_eq!(channel.kind, "webhook");
        assert_eq!(list_channels(&pool).await.unwrap().len(), 1);

        let rule = create_rule(
            &pool,
            "failed backups",
            events::BACKUP_FAILED,
            "critical",
            &channel.id,
            60,
        )
        .await
        .unwrap();
        assert_eq!(rule.event, events::BACKUP_FAILED);
        assert_eq!(list_rules(&pool).await.unwrap().len(), 1);

        // A rule for a channel that does not exist is rejected.
        assert!(
            create_rule(
                &pool,
                "broken",
                events::BACKUP_FAILED,
                "warning",
                "missing",
                60
            )
            .await
            .is_err()
        );

        // Deleting the channel removes its rules.
        assert!(delete_channel(&pool, &channel.id).await.unwrap());
        assert!(list_rules(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_webhook_delivery_failure_is_recorded() {
        let pool = test_pool().await;
        let channel = create_channel(
            &pool,
            "broken",
            "webhook",
            // Port 9 is the discard port; nothing listens there.
            json!({ "url": "http://127.0.0.1:9/hook" }),
        )
        .await
        .unwrap();
        let rule = create_rule(
            &pool,
            "failed backups",
            events::BACKUP_FAILED,
            "critical",
            &channel.id,
            0,
        )
        .await
        .unwrap();

        let notified = emit(
            &pool,
            events::BACKUP_FAILED,
            "critical",
            "the backup failed",
            &json!({}),
        )
        .await
        .unwrap();
        assert_eq!(notified, 0);

        let alerts = list_alerts(&pool, 10).await.unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].status, "failed");
        assert!(alerts[0].error.is_some());
        assert_eq!(alerts[0].rule_id.as_deref(), Some(rule.id.as_str()));
    }

    #[tokio::test]
    async fn the_cooldown_suppresses_repeated_alerts() {
        let pool = test_pool().await;
        let channel = create_channel(
            &pool,
            "ops",
            "webhook",
            json!({ "url": "http://127.0.0.1:9/hook" }),
        )
        .await
        .unwrap();
        create_rule(
            &pool,
            "failed backups",
            events::BACKUP_FAILED,
            "warning",
            &channel.id,
            3600,
        )
        .await
        .unwrap();

        emit(&pool, events::BACKUP_FAILED, "warning", "first", &json!({}))
            .await
            .unwrap();
        emit(
            &pool,
            events::BACKUP_FAILED,
            "warning",
            "second",
            &json!({}),
        )
        .await
        .unwrap();
        assert_eq!(list_alerts(&pool, 10).await.unwrap().len(), 1);
    }

    #[test]
    fn validates_email_channels() {
        assert!(validate_email_config(&json!({ "command": "/usr/sbin/sendmail" })).is_err());
        assert!(
            validate_email_config(&json!({ "command": "/usr/sbin/sendmail", "to": [] })).is_err()
        );
        assert!(
            validate_email_config(&json!({ "command": "/usr/sbin/sendmail", "to": ["a@b.c"] }))
                .is_ok()
        );
        assert!(validate_email_config(&json!({ "smtp": { "host": "h" } })).is_err());
    }

    #[test]
    fn validates_webhook_channels() {
        assert!(validate_webhook_config(&json!({})).is_err());
        assert!(validate_webhook_config(&json!({ "url": "ftp://x" })).is_err());
        assert!(validate_webhook_config(&json!({ "url": "https://example.com/hook" })).is_ok());
    }
}
