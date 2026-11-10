//! Database records: the databases and users the panel manages.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

/// A database the panel manages.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DatabaseRecord {
    pub id: String,
    pub engine: String,
    pub name: String,
    pub owner_id: Option<String>,
    pub created_at: String,
}

/// A user of a managed database.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DatabaseUserRecord {
    pub id: String,
    pub database_id: String,
    pub username: String,
    pub password_encrypted: String,
    pub privileges: String,
    pub created_at: String,
}

/// The privileges the panel can grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privilege {
    All,
    Read,
    ReadWrite,
    ReadWriteAdmin,
}

impl Privilege {
    pub fn as_str(&self) -> &'static str {
        match self {
            Privilege::All => "ALL",
            Privilege::Read => "READ",
            Privilege::ReadWrite => "READWRITE",
            Privilege::ReadWriteAdmin => "READWRITEADMIN",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "ALL" => Some(Privilege::All),
            "READ" => Some(Privilege::Read),
            "READWRITE" | "READ_WRITE" => Some(Privilege::ReadWrite),
            "READWRITEADMIN" | "READ_WRITE_ADMIN" => Some(Privilege::ReadWriteAdmin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Databases {
    pool: SqlitePool,
}

impl Databases {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        engine: &str,
        name: &str,
        owner_id: Option<String>,
    ) -> Result<DatabaseRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO databases (id, engine, name, owner_id, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(engine)
        .bind(name)
        .bind(owner_id)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("database vanished after insert"))
    }

    pub async fn get(&self, id: &str) -> Result<Option<DatabaseRecord>> {
        let row = sqlx::query_as::<_, DatabaseRecord>("SELECT * FROM databases WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn get_by_name(&self, engine: &str, name: &str) -> Result<Option<DatabaseRecord>> {
        let row = sqlx::query_as::<_, DatabaseRecord>(
            "SELECT * FROM databases WHERE engine = ? AND name = ?",
        )
        .bind(engine)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list(&self) -> Result<Vec<DatabaseRecord>> {
        let rows =
            sqlx::query_as::<_, DatabaseRecord>("SELECT * FROM databases ORDER BY engine, name")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM databases WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn add_user(
        &self,
        database_id: &str,
        username: &str,
        password_encrypted: &str,
        privileges: Privilege,
    ) -> Result<DatabaseUserRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO db_users
                (id, database_id, username, password_encrypted, privileges, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(database_id)
        .bind(username)
        .bind(password_encrypted)
        .bind(privileges.as_str())
        .bind(&now)
        .execute(&self.pool)
        .await?;
        let row = sqlx::query_as::<_, DatabaseUserRecord>("SELECT * FROM db_users WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn list_users(&self, database_id: &str) -> Result<Vec<DatabaseUserRecord>> {
        let rows = sqlx::query_as::<_, DatabaseUserRecord>(
            "SELECT * FROM db_users WHERE database_id = ? ORDER BY username",
        )
        .bind(database_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn delete_user(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM db_users WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM databases")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn test_pool() -> SqlitePool {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-dbs-{}.db", Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn database_and_user_lifecycle() {
        let pool = test_pool().await;
        let databases = Databases::new(pool);
        let record = databases.create("mysql", "shop", None).await.unwrap();
        assert_eq!(record.engine, "mysql");

        let found = databases
            .get_by_name("mysql", "shop")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, record.id);

        let user = databases
            .add_user(&record.id, "shop_user", "encrypted", Privilege::All)
            .await
            .unwrap();
        assert_eq!(user.privileges, "ALL");

        let users = databases.list_users(&record.id).await.unwrap();
        assert_eq!(users.len(), 1);

        assert!(databases.delete_user(&user.id).await.unwrap());
        assert!(databases.delete(&record.id).await.unwrap());
        assert_eq!(databases.count().await.unwrap(), 0);
    }

    #[test]
    fn privileges_roundtrip() {
        assert_eq!(Privilege::parse("all"), Some(Privilege::All));
        assert_eq!(Privilege::parse("readwrite"), Some(Privilege::ReadWrite));
        assert_eq!(Privilege::parse("nope"), None);
        assert_eq!(Privilege::Read.as_str(), "READ");
    }
}
