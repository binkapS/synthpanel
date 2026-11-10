//! SQLite connection management and schema migrations.

use anyhow::Result;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use tracing::info;

/// Embedded migrations compiled into the binary.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Open (and if necessary create) the state database.
pub async fn connect(path: impl AsRef<Path>) -> Result<SqlitePool> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(10))
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await?;

    Ok(pool)
}

/// Apply all pending migrations.
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    MIGRATOR.run(pool).await?;
    info!("database schema is up to date");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_apply_on_a_fresh_database() {
        let dir = tempfile_dir();
        let path = dir.join("test.db");
        let pool = connect(&path).await.unwrap();
        migrate(&pool).await.unwrap();

        // A second migrate must be a no-op.
        migrate(&pool).await.unwrap();

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(count >= 20, "expected the full schema, got {count} tables");
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("sp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
