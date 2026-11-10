//! Site records and their per-site nginx/php state.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SiteRecord {
    pub id: String,
    pub domain: String,
    pub aliases: String,
    pub document_root: String,
    pub template: String,
    pub php_version: Option<String>,
    pub pool_user: String,
    pub status: String,
    pub ssl_mode: String,
    pub ssl_cert_id: Option<String>,
    pub owner_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SiteNginx {
    pub site_id: String,
    pub directives: String,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SitePhp {
    pub site_id: String,
    pub overrides_json: String,
}

/// Input for creating a new site. The web process hashes passwords and
/// sanitises input before calling this.
#[derive(Debug, Clone)]
pub struct CreateSite {
    pub domain: String,
    pub aliases: Vec<String>,
    pub document_root: String,
    pub template: String,
    pub php_version: Option<String>,
    pub ssl_mode: String,
    pub owner_id: Option<String>,
    pub directives: String,
}

#[derive(Debug, Clone, Default)]
pub struct SitePatch {
    pub domain: Option<String>,
    pub aliases: Option<Vec<String>>,
    pub document_root: Option<String>,
    pub template: Option<String>,
    pub php_version: Option<Option<String>>,
    pub ssl_mode: Option<String>,
    pub status: Option<String>,
    pub directives: Option<String>,
    pub overrides_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Sites {
    pool: SqlitePool,
}

impl Sites {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, input: CreateSite) -> Result<SiteRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let aliases_json = serde_json::to_string(&input.aliases)?;

        sqlx::query(
            "INSERT INTO sites
                (id, domain, aliases, document_root, template, php_version, pool_user,
                 status, ssl_mode, ssl_cert_id, owner_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, NULL, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&input.domain)
        .bind(&aliases_json)
        .bind(&input.document_root)
        .bind(&input.template)
        .bind(&input.php_version)
        .bind(format!("sp_{id}"))
        .bind(&input.ssl_mode)
        .bind(&input.owner_id)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        sqlx::query("INSERT INTO site_nginx (site_id, directives) VALUES (?, ?)")
            .bind(&id)
            .bind(&input.directives)
            .execute(&self.pool)
            .await?;

        sqlx::query("INSERT INTO site_php (site_id, overrides_json) VALUES (?, ?)")
            .bind(&id)
            .bind("{}")
            .execute(&self.pool)
            .await?;

        self.get(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("site vanished after insert"))
    }

    pub async fn get(&self, id: &str) -> Result<Option<SiteRecord>> {
        let row = sqlx::query_as::<_, SiteRecord>("SELECT * FROM sites WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn list(&self) -> Result<Vec<SiteRecord>> {
        let rows =
            sqlx::query_as::<_, SiteRecord>("SELECT * FROM sites ORDER BY created_at, domain")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }

    pub async fn get_with_extras(&self, id: &str) -> Result<Option<SiteExtras>> {
        let site = match self.get(id).await? {
            Some(site) => site,
            None => return Ok(None),
        };
        let nginx: Option<(String,)> =
            sqlx::query_as("SELECT directives FROM site_nginx WHERE site_id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        let php: Option<(String,)> =
            sqlx::query_as("SELECT overrides_json FROM site_php WHERE site_id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(Some(SiteExtras {
            site,
            nginx: nginx.map(|row| row.0),
            php: php.map(|row| row.0),
        }))
    }

    pub async fn update(&self, id: &str, patch: SitePatch) -> Result<Option<SiteRecord>> {
        let mut site = match self.get(id).await? {
            Some(site) => site,
            None => return Ok(None),
        };
        if let Some(value) = patch.domain {
            site.domain = value;
        }
        if let Some(value) = &patch.aliases {
            site.aliases = serde_json::to_string(value)?;
        }
        if let Some(value) = patch.document_root {
            site.document_root = value;
        }
        if let Some(value) = patch.template {
            site.template = value;
        }
        if let Some(value) = patch.ssl_mode {
            site.ssl_mode = value;
        }
        if let Some(value) = patch.status {
            site.status = value;
        }
        if let Some(value) = patch.php_version {
            site.php_version = value;
        }
        site.updated_at = Utc::now().to_rfc3339();

        sqlx::query(
            "UPDATE sites SET domain = ?, aliases = ?, document_root = ?, template = ?,
                php_version = ?, pool_user = ?, status = ?, ssl_mode = ?, owner_id = ?,
                updated_at = ? WHERE id = ?",
        )
        .bind(&site.domain)
        .bind(&site.aliases)
        .bind(&site.document_root)
        .bind(&site.template)
        .bind(&site.php_version)
        .bind(&site.pool_user)
        .bind(&site.status)
        .bind(&site.ssl_mode)
        .bind(&site.owner_id)
        .bind(&site.updated_at)
        .bind(id)
        .execute(&self.pool)
        .await?;

        if let Some(value) = patch.directives {
            sqlx::query("UPDATE site_nginx SET directives = ? WHERE site_id = ?")
                .bind(value)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        if let Some(value) = patch.overrides_json {
            sqlx::query("UPDATE site_php SET overrides_json = ? WHERE site_id = ?")
                .bind(value)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        Ok(Some(site))
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM sites WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Count all sites; used by quotas.
    pub async fn count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sites")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SiteExtras {
    pub site: SiteRecord,
    pub nginx: Option<String>,
    pub php: Option<String>,
}

impl SiteRecord {
    pub fn alias_list(&self) -> Vec<String> {
        serde_json::from_str(&self.aliases).unwrap_or_default()
    }

    pub fn all_server_names(&self) -> Vec<String> {
        let mut names = vec![self.domain.clone()];
        names.extend(self.alias_list());
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn test_pool() -> SqlitePool {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-sites-{}.db", Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn site_crud_roundtrip() {
        let pool = test_pool().await;
        let sites = Sites::new(pool);
        let created = sites
            .create(CreateSite {
                domain: "example.com".into(),
                aliases: vec!["www.example.com".into()],
                document_root: "/var/www/sp_test/public".into(),
                template: "php".into(),
                php_version: Some("8.3".into()),
                ssl_mode: "le".into(),
                owner_id: None,
                directives: String::new(),
            })
            .await
            .unwrap();
        assert_eq!(created.status, "active");
        assert!(created.pool_user.starts_with("sp_"));
        assert_eq!(created.alias_list(), vec!["www.example.com"]);

        let extras = sites.get_with_extras(&created.id).await.unwrap().unwrap();
        assert_eq!(extras.nginx.as_deref(), Some(""));
        assert_eq!(extras.php.as_deref(), Some("{}"));

        let updated = sites
            .update(
                &created.id,
                SitePatch {
                    ssl_mode: Some("none".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.ssl_mode, "none");

        assert!(sites.delete(&created.id).await.unwrap());
        assert!(sites.get(&created.id).await.unwrap().is_none());
        assert_eq!(sites.count().await.unwrap(), 0);
    }
}
