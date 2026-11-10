//! User accounts, roles and TOTP state.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct UserRecord {
    pub id: String,
    pub email: String,
    pub name: String,
    pub password_hash: String,
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
    pub role: String,
    pub status: String,
    pub quota_sites: i64,
    pub quota_dbs: i64,
    pub last_login_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct CreateUser {
    pub email: String,
    pub name: String,
    pub password_hash: String,
    pub role: String,
    pub quota_sites: i64,
    pub quota_dbs: i64,
}

#[derive(Debug, Clone, Default)]
pub struct UserPatch {
    pub email: Option<String>,
    pub name: Option<String>,
    pub password_hash: Option<String>,
    pub totp_secret: Option<Option<String>>,
    pub totp_enabled: Option<bool>,
    pub role: Option<String>,
    pub status: Option<String>,
    pub quota_sites: Option<i64>,
    pub quota_dbs: Option<i64>,
    pub last_login_at: Option<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct Users {
    pool: SqlitePool,
}

impl Users {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, input: CreateUser) -> Result<UserRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users
                (id, email, name, password_hash, totp_secret, totp_enabled, role, status,
                 quota_sites, quota_dbs, last_login_at, created_at)
             VALUES (?, ?, ?, ?, NULL, 0, ?, 'active', ?, ?, NULL, ?)",
        )
        .bind(&id)
        .bind(&input.email)
        .bind(&input.name)
        .bind(&input.password_hash)
        .bind(&input.role)
        .bind(input.quota_sites)
        .bind(input.quota_dbs)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("user vanished after insert"))
    }

    pub async fn get(&self, id: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRecord>("SELECT * FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn get_by_email(&self, email: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRecord>("SELECT * FROM users WHERE email = ?")
            .bind(email)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn list(&self) -> Result<Vec<UserRecord>> {
        let rows =
            sqlx::query_as::<_, UserRecord>("SELECT * FROM users ORDER BY created_at, email")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }

    pub async fn count(&self) -> Result<i64> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?;
        Ok(count.0)
    }

    pub async fn update(&self, id: &str, patch: UserPatch) -> Result<Option<UserRecord>> {
        let mut user = match self.get(id).await? {
            Some(u) => u,
            None => return Ok(None),
        };
        if let Some(v) = patch.email {
            user.email = v;
        }
        if let Some(v) = patch.name {
            user.name = v;
        }
        if let Some(v) = patch.password_hash {
            user.password_hash = v;
        }
        if let Some(v) = patch.totp_secret {
            user.totp_secret = v;
        }
        if let Some(v) = patch.totp_enabled {
            user.totp_enabled = v;
        }
        if let Some(v) = patch.role {
            user.role = v;
        }
        if let Some(v) = patch.status {
            user.status = v;
        }
        if let Some(v) = patch.quota_sites {
            user.quota_sites = v;
        }
        if let Some(v) = patch.quota_dbs {
            user.quota_dbs = v;
        }
        if let Some(v) = patch.last_login_at {
            user.last_login_at = v;
        }
        sqlx::query(
            "UPDATE users SET email = ?, name = ?, password_hash = ?, totp_secret = ?,
                totp_enabled = ?, role = ?, status = ?, quota_sites = ?, quota_dbs = ?,
                last_login_at = ? WHERE id = ?",
        )
        .bind(&user.email)
        .bind(&user.name)
        .bind(&user.password_hash)
        .bind(&user.totp_secret)
        .bind(user.totp_enabled)
        .bind(&user.role)
        .bind(&user.status)
        .bind(user.quota_sites)
        .bind(user.quota_dbs)
        .bind(&user.last_login_at)
        .bind(&user.id)
        .execute(&self.pool)
        .await?;
        Ok(Some(user))
    }

    pub async fn touch_last_login(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE users SET last_login_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn test_pool() -> SqlitePool {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-users-{}.db", Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn create_get_update_roundtrip() {
        let pool = test_pool().await;
        let users = Users::new(pool);
        let user = users
            .create(CreateUser {
                email: "admin@example.com".into(),
                name: "Admin".into(),
                password_hash: "$argon2id$v=19$...".into(),
                role: "admin".into(),
                quota_sites: 10,
                quota_dbs: 5,
            })
            .await
            .unwrap();
        assert_eq!(user.email, "admin@example.com");
        assert_eq!(user.role, "admin");

        let fetched = users
            .get_by_email("admin@example.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, user.id);

        let updated = users
            .update(
                &user.id,
                UserPatch {
                    role: Some("operator".into()),
                    quota_dbs: Some(42),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.role, "operator");
        assert_eq!(updated.quota_dbs, 42);

        assert_eq!(users.count().await.unwrap(), 1);
    }
}
