//! API key site assignments (CloudPanel parity wave 4).
//!
//! The `api_keys` table stores only the SHA-256 hash of a token; this module
//! never sees a raw key and never writes one to the database or the logs.
//! Assignments are keyed by the key's immutable UUID, so the web layer can
//! hand a key id to the core without exposing the token again.
//!
//! An empty assignment means "no explicit restriction" (legacy keys keep
//! working). A non-empty assignment restricts the key to those sites, on top
//! of the owner's own site grants which the web authorization layer checks.

use anyhow::{Result, bail};
use serde_json::Value;
use sqlx::SqlitePool;

fn push_site_id(ids: &mut Vec<String>, id: &str) {
    let id = id.trim();
    if !id.is_empty() && !ids.iter().any(|known| known == id) {
        ids.push(id.to_string());
    }
}

/// Parse a `site_ids` IPC parameter into a normalised list of ids.
///
/// A plain list of strings is the documented shape. A list of records with a
/// `site_id` (or `id`) field and an object wrapping either shape are accepted
/// as well, so callers cannot accidentally widen access by sending a shape
/// that silently parses to nothing. Anything else yields an empty list.
pub fn parse_site_ids(value: Option<&Value>) -> Vec<String> {
    let entries: Vec<&Value> = match value {
        Some(Value::Array(list)) => list.iter().collect(),
        Some(Value::Object(map)) => match map
            .get("site_ids")
            .or_else(|| map.get("sites"))
            .or_else(|| map.get("assignments"))
        {
            Some(Value::Array(list)) => list.iter().collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };

    let mut ids: Vec<String> = Vec::new();
    for entry in entries {
        match entry {
            Value::String(id) => push_site_id(&mut ids, id),
            Value::Object(_) => {
                if let Some(id) = entry
                    .get("site_id")
                    .or_else(|| entry.get("id"))
                    .and_then(Value::as_str)
                {
                    push_site_id(&mut ids, id);
                }
            }
            _ => {}
        }
    }
    ids
}

/// Normalise an already-decoded list of ids (trim, drop blanks, deduplicate).
pub fn normalize_site_ids(site_ids: &[String]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for id in site_ids {
        push_site_id(&mut ids, id);
    }
    ids
}

/// Replace the site assignments of an API key atomically.
///
/// An empty `site_ids` clears the restriction; the key itself is left intact.
/// Unknown sites are rejected by the foreign key constraint, and an unknown
/// key is rejected up front so a typo cannot create orphaned assignments.
pub async fn set_sites_for_key(pool: &SqlitePool, key_id: &str, site_ids: &[String]) -> Result<()> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_keys WHERE id = ?)")
        .bind(key_id)
        .fetch_one(pool)
        .await?;
    if !exists {
        bail!("API key {key_id} does not exist");
    }

    let site_ids = normalize_site_ids(site_ids);
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM api_key_sites WHERE id = ?")
        .bind(key_id)
        .execute(&mut *tx)
        .await?;
    for site_id in &site_ids {
        sqlx::query("INSERT INTO api_key_sites (id, site_id) VALUES (?, ?)")
            .bind(key_id)
            .bind(site_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// List the sites an API key is restricted to, sorted for stable responses.
pub async fn list_site_ids_for_key(pool: &SqlitePool, key_id: &str) -> Result<Vec<String>> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT site_id FROM api_key_sites WHERE id = ? ORDER BY site_id")
            .bind(key_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|row| row.0).collect())
}

/// Read-only membership check for a single key/site pair.
pub async fn key_can_access_site(pool: &SqlitePool, key_id: &str, site_id: &str) -> Result<bool> {
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM api_key_sites WHERE id = ? AND site_id = ?)",
    )
    .bind(key_id)
    .bind(site_id)
    .fetch_one(pool)
    .await?;
    Ok(allowed)
}

/// Pure membership test for callers that already hold the key's site list.
pub fn site_ids_contain(site_ids: &[String], site_id: &str) -> bool {
    site_ids.iter().any(|id| id == site_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::sites::{CreateSite, Sites};
    use crate::users::{CreateUser, Users};
    use serde_json::json;
    use uuid::Uuid;

    // A hex-encoded SHA-256 digest, not a token. The helpers must work with
    // the hash the key store keeps, and must never require the raw key.
    const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    #[test]
    fn site_ids_are_parsed_from_every_supported_shape() {
        let cases = [
            (json!(["site-1", "site-2"]), vec!["site-1", "site-2"]),
            (json!([" site-1 ", "site-1", "", " "]), vec!["site-1"]),
            (
                json!([{"site_id": "site-1"}, {"id": "site-2"}]),
                vec!["site-1", "site-2"],
            ),
            (json!({"site_ids": ["site-1"]}), vec!["site-1"]),
            (json!({"sites": [{"site_id": "site-3"}]}), vec!["site-3"]),
            (json!({"assignments": ["site-4"]}), vec!["site-4"]),
            (json!([]), vec![]),
            (json!({}), vec![]),
            (json!(null), vec![]),
            (json!("site-1"), vec![]),
            (json!([42, null, true, {}, []]), vec![]),
            (json!({"site_ids": "site-1"}), vec![]),
        ];
        for (value, expected) in cases {
            assert_eq!(
                parse_site_ids(Some(&value)),
                expected.into_iter().map(str::to_string).collect::<Vec<_>>(),
                "value {value}"
            );
        }
        assert!(parse_site_ids(None).is_empty());
    }

    #[test]
    fn decoded_lists_are_normalised() {
        let ids = vec![
            "site-1".into(),
            " site-2 ".into(),
            "site-1".into(),
            "".into(),
        ];
        assert_eq!(normalize_site_ids(&ids), vec!["site-1", "site-2"]);
        assert!(site_ids_contain(&normalize_site_ids(&ids), "site-2"));
        assert!(!site_ids_contain(&normalize_site_ids(&ids), "site-9"));
    }

    async fn test_pool() -> SqlitePool {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-api-key-sites-{}.db", Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        pool
    }

    async fn make_key(pool: &SqlitePool) -> String {
        let user = Users::new(pool.clone())
            .create(CreateUser {
                email: "owner@example.com".into(),
                name: "Owner".into(),
                password_hash: "hash".into(),
                role: "admin".into(),
                quota_sites: 10,
                quota_dbs: 10,
            })
            .await
            .unwrap()
            .id;
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO api_keys (id, user_id, name, prefix, hash, scopes_json, created_at)
             VALUES (?, ?, 'test', 'abcdef', ?, '[\"read\"]', ?)",
        )
        .bind(&id)
        .bind(user)
        .bind(HASH)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn make_site(pool: &SqlitePool, domain: &str) -> String {
        Sites::new(pool.clone())
            .create(CreateSite {
                domain: domain.into(),
                aliases: Vec::new(),
                document_root: format!("/tmp/sp-test-{domain}"),
                template: "php".into(),
                php_version: None,
                ssl_mode: "none".into(),
                owner_id: None,
                directives: String::new(),
            })
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn assignments_are_set_listed_and_replaced() {
        let pool = test_pool().await;
        let key = make_key(&pool).await;
        let alpha = make_site(&pool, "alpha.example.com").await;
        let beta = make_site(&pool, "beta.example.com").await;

        assert!(list_site_ids_for_key(&pool, &key).await.unwrap().is_empty());
        assert!(!key_can_access_site(&pool, &key, &alpha).await.unwrap());

        set_sites_for_key(&pool, &key, &[alpha.clone(), beta.clone(), alpha.clone()])
            .await
            .unwrap();
        let mut listed = list_site_ids_for_key(&pool, &key).await.unwrap();
        listed.sort();
        let mut expected = vec![alpha.clone(), beta.clone()];
        expected.sort();
        assert_eq!(listed, expected);
        assert!(key_can_access_site(&pool, &key, &alpha).await.unwrap());

        // Replacing shrinks the set and ignores duplicates/blank ids.
        set_sites_for_key(&pool, &key, &[beta.clone(), " ".into(), beta.clone()])
            .await
            .unwrap();
        assert_eq!(
            list_site_ids_for_key(&pool, &key).await.unwrap(),
            vec![beta]
        );
        assert!(!key_can_access_site(&pool, &key, &alpha).await.unwrap());

        // Clearing leaves the key usable but removes the restriction.
        set_sites_for_key(&pool, &key, &[]).await.unwrap();
        assert!(list_site_ids_for_key(&pool, &key).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn assignments_cascade_and_validate_foreign_keys() {
        let pool = test_pool().await;
        let key = make_key(&pool).await;
        let site = make_site(&pool, "alpha.example.com").await;
        set_sites_for_key(&pool, &key, &[site.clone()])
            .await
            .unwrap();

        assert!(set_sites_for_key(&pool, "missing-key", &[]).await.is_err());
        assert!(
            set_sites_for_key(&pool, &key, &["missing-site".into()])
                .await
                .is_err()
        );
        // The failed replacement must not have committed a partial change.
        assert_eq!(
            list_site_ids_for_key(&pool, &key).await.unwrap(),
            vec![site.clone()]
        );

        Sites::new(pool.clone()).delete(&site).await.unwrap();
        assert!(list_site_ids_for_key(&pool, &key).await.unwrap().is_empty());

        assert!(set_sites_for_key(&pool, &key, &[site]).await.is_err());
        sqlx::query("DELETE FROM api_keys WHERE id = ?")
            .bind(&key)
            .execute(&pool)
            .await
            .unwrap();
        assert!(list_site_ids_for_key(&pool, &key).await.unwrap().is_empty());
    }
}
