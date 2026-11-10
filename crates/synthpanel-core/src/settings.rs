//! Key/value settings repository.

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};
use std::collections::BTreeMap;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SettingEntry {
    pub key: String,
    pub value_json: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pool: SqlitePool,
}

impl Settings {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_raw(&self, key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value_json FROM settings WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| r.0))
    }

    pub async fn get(&self, key: &str) -> Result<Option<Value>> {
        match self.get_raw(key).await? {
            Some(s) => Ok(Some(serde_json::from_str(&s)?)),
            None => Ok(None),
        }
    }

    pub async fn get_parsed<T: DeserializeOwned>(&self, key: &str, default: T) -> Result<T> {
        match self.get(key).await? {
            Some(v) => Ok(serde_json::from_value(v).unwrap_or(default)),
            None => Ok(default),
        }
    }

    pub async fn set(&self, key: &str, value: &Value) -> Result<()> {
        let json = serde_json::to_string(value)?;
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO settings (key, value_json, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET
                value_json = excluded.value_json,
                updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(json)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<SettingEntry>> {
        let rows = sqlx::query_as("SELECT key, value_json, updated_at FROM settings ORDER BY key")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    pub async fn map(&self) -> Result<BTreeMap<String, Value>> {
        let mut map = BTreeMap::new();
        for entry in self.list().await? {
            if let Ok(value) = serde_json::from_str(&entry.value_json) {
                map.insert(entry.key, value);
            }
        }
        Ok(map)
    }
}
