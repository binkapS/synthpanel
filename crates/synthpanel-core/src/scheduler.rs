//! Background scheduler for cron jobs and backup schedules.
//!
//! The core ticks this once a minute. Cron expressions are evaluated in UTC,
//! matching the rest of the panel.

use crate::backup::{self, BackupPlan, Contents, Destination, Kind};
use crate::config::Paths;
use crate::cron::Schedule;
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tracing::{info, warn};

/// Run every cron job that is due now and refresh the crontab fragment.
pub async fn tick_cron(pool: &SqlitePool) -> Result<usize> {
    crate::cron::run_due(pool, &crate::cron::crontab_dir()).await
}

/// Run every backup schedule that is due.
pub async fn tick_backups(pool: &SqlitePool, paths: &Paths) -> Result<usize> {
    #[derive(sqlx::FromRow)]
    struct ScheduleRow {
        id: String,
        name: String,
        cron: String,
        destination_json: String,
        retention_json: String,
    }

    let schedules = sqlx::query_as::<_, ScheduleRow>(
        "SELECT id, name, cron, destination_json, retention_json
         FROM backup_sched WHERE enabled = 1",
    )
    .fetch_all(pool)
    .await?;

    let now = Utc::now();
    let mut ran = 0;

    for schedule in schedules {
        let expression = match Schedule::parse(&schedule.cron) {
            Ok(expression) => expression,
            Err(error) => {
                warn!(schedule = %schedule.id, %error, "invalid backup schedule");
                continue;
            }
        };
        if !expression.matches(now) {
            continue;
        }

        // Do not run twice in the same minute.
        let last: Option<String> = sqlx::query_scalar(
            "SELECT created_at FROM backups WHERE schedule_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&schedule.id)
        .fetch_optional(pool)
        .await?;
        if let Some(last) = last {
            if let Ok(last) = DateTime::parse_from_rfc3339(&last) {
                if last.with_timezone(&Utc).format("%Y%m%d%H%M").to_string()
                    == now.format("%Y%m%d%H%M").to_string()
                {
                    continue;
                }
            }
        }

        let destination: Destination = match serde_json::from_str(&schedule.destination_json) {
            Ok(destination) => destination,
            Err(error) => {
                warn!(schedule = %schedule.id, %error, "invalid backup destination");
                continue;
            }
        };

        let plan = BackupPlan {
            id: uuid::Uuid::new_v4().to_string(),
            kind: Kind::Full,
            site_id: None,
            site_domain: None,
            site_root: None,
            contents: Contents::default(),
            destination,
            passphrase: std::env::var("SYNTHPANEL_BACKUP_PASSPHRASE").ok(),
            staging_dir: paths.backup_dir.join("staging"),
            vhost_dir: crate::apply::Paths::from_env().nginx_vhosts,
            pool_dir: crate::apply::Paths::from_env()
                .php_pools
                .first()
                .and_then(|path| path.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| paths.etc_dir.join("php")),
            www_dir: paths.www_dir.clone(),
        };

        match backup::create_backup(&plan).await {
            Ok(result) => {
                sqlx::query(
                    "INSERT INTO backups
                     (id, kind, status, size, location, schedule_id, error, created_at,
                      name, site_id, checksum, encrypted, entries_json, duration_ms)
                     VALUES (?, 'full', 'ok', ?, ?, ?, NULL, ?, ?, NULL, ?, ?, ?, ?)",
                )
                .bind(&result.id)
                .bind(result.size as i64)
                .bind(&result.location)
                .bind(&schedule.id)
                .bind(&result.name)
                .bind(&result.checksum)
                .bind(if result.encrypted { 1i64 } else { 0i64 })
                .bind(serde_json::to_string(&result.entries).unwrap_or_else(|_| "[]".into()))
                .bind(result.duration_ms as i64)
                .execute(pool)
                .await?;
                ran += 1;
                info!(schedule = %schedule.name, backup = %result.id, "scheduled backup created");
                prune_retention(pool, &plan.destination, &schedule.retention_json).await;
            }
            Err(error) => {
                warn!(schedule = %schedule.name, %error, "scheduled backup failed");
                let id = uuid::Uuid::new_v4().to_string();
                let _ = sqlx::query(
                    "INSERT INTO backups
                     (id, kind, status, size, location, schedule_id, error, created_at,
                      name, site_id, checksum, encrypted, entries_json, duration_ms)
                     VALUES (?, 'full', 'failed', 0, '', ?, ?, ?, '', NULL, '', 0, '[]', 0)",
                )
                .bind(&id)
                .bind(&schedule.id)
                .bind(error.to_string())
                .bind(Utc::now().to_rfc3339())
                .execute(pool)
                .await;
            }
        }
    }

    Ok(ran)
}

/// Keep only the newest `count` archives for a local destination.
async fn prune_retention(pool: &SqlitePool, destination: &Destination, retention_json: &str) {
    let keep = serde_json::from_str::<serde_json::Value>(retention_json)
        .ok()
        .and_then(|value| value.get("count").and_then(|count| count.as_i64()))
        .unwrap_or(30)
        .clamp(1, 3650);

    let Destination::Local { path } = destination else {
        return;
    };

    let mut names: Vec<String> = match std::fs::read_dir(path) {
        Ok(entries) => entries
            .flatten()
            .filter(|entry| entry.path().is_file())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect(),
        Err(_) => return,
    };
    names.sort();
    if names.len() <= keep as usize {
        return;
    }
    let remove = names.len() - keep as usize;
    for name in names.into_iter().take(remove) {
        let target = path.join(&name);
        let _ = std::fs::remove_file(&target);
        let _ = sqlx::query("DELETE FROM backups WHERE name = ?")
            .bind(&name)
            .execute(pool)
            .await;
    }
}
