//! Append-only audit log.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub action: String,
    pub target: Option<String>,
    pub metadata_json: String,
    pub ip: Option<String>,
    pub created_at: String,
}

impl AuditEntry {
    pub fn metadata(&self) -> Value {
        serde_json::from_str(&self.metadata_json).unwrap_or(Value::Null)
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn record(
    pool: &SqlitePool,
    actor_id: Option<&str>,
    actor_name: Option<&str>,
    action: &str,
    target: Option<&str>,
    metadata: &Value,
    ip: Option<&str>,
) -> Result<i64> {
    let metadata_json = serde_json::to_string(metadata)?;
    let now = Utc::now().to_rfc3339();
    let id = sqlx::query(
        "INSERT INTO audit_log
            (actor_id, actor_name, action, target, metadata_json, ip, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(actor_id)
    .bind(actor_name)
    .bind(action)
    .bind(target)
    .bind(metadata_json)
    .bind(ip)
    .bind(now)
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

pub async fn list(pool: &SqlitePool, limit: i64) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>("SELECT * FROM audit_log ORDER BY id DESC LIMIT ?")
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn entries_are_returned_newest_first() {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-audit-{}.db", uuid::Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();

        record(
            &pool,
            None,
            None,
            "settings.set",
            Some("hostname"),
            &serde_json::json!({"value":"a"}),
            None,
        )
        .await
        .unwrap();
        record(
            &pool,
            Some("u1"),
            Some("Admin"),
            "sites.create",
            Some("example.com"),
            &serde_json::json!({}),
            Some("127.0.0.1"),
        )
        .await
        .unwrap();

        let entries = list(&pool, 10).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].action, "sites.create");
        assert_eq!(entries[0].id, 2);
        assert_eq!(entries[1].action, "settings.set");
    }
}
