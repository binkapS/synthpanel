//! Per-site user scoping (CloudPanel parity).
//!
//! The `user_sites` table is the source of truth for which sites a user may
//! see. This module deliberately contains no role logic: the web layer
//! decides that administrators and operators bypass the mapping, while the
//! core only stores and checks it.

use anyhow::Result;
use sqlx::SqlitePool;

/// List the site ids granted to a user.
///
/// The result is sorted so IPC responses are stable. A user with no grants
/// gets an empty vector.
pub async fn list_site_ids_for_user(pool: &SqlitePool, user_id: &str) -> Result<Vec<String>> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT site_id FROM user_sites WHERE user_id = ? ORDER BY site_id")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|row| row.0).collect())
}

/// Replace all site grants for a user with `site_ids`.
///
/// The replacement is atomic: readers never observe a partially applied set.
/// Duplicate ids are ignored; an unknown site (or an unknown user) is
/// rejected by the foreign key constraints in the schema.
pub async fn set_sites_for_user(
    pool: &SqlitePool,
    user_id: &str,
    site_ids: &[String],
) -> Result<()> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = ?)")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    if !exists {
        anyhow::bail!("user {user_id} does not exist");
    }

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_sites WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for site_id in site_ids {
        sqlx::query("INSERT OR IGNORE INTO user_sites (user_id, site_id) VALUES (?, ?)")
            .bind(user_id)
            .bind(site_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Check a single grant. This is a read-only membership test; role-based
/// bypass belongs in the web authorization layer.
pub async fn user_can_access_site(pool: &SqlitePool, user_id: &str, site_id: &str) -> Result<bool> {
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM user_sites WHERE user_id = ? AND site_id = ?)",
    )
    .bind(user_id)
    .bind(site_id)
    .fetch_one(pool)
    .await?;
    Ok(allowed)
}

/// Pure membership test for callers that already hold the user's site list.
pub fn site_ids_contain(site_ids: &[String], site_id: &str) -> bool {
    site_ids.iter().any(|id| id == site_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::sites::{CreateSite, Sites};
    use crate::users::{CreateUser, Users};
    use uuid::Uuid;

    async fn test_pool() -> SqlitePool {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-site-acl-{}.db", Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        pool
    }

    async fn make_user(pool: &SqlitePool, email: &str) -> String {
        Users::new(pool.clone())
            .create(CreateUser {
                email: email.into(),
                name: "Test".into(),
                password_hash: "hash".into(),
                role: "user".into(),
                quota_sites: 10,
                quota_dbs: 10,
            })
            .await
            .unwrap()
            .id
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
    async fn set_list_and_replace_grants() {
        let pool = test_pool().await;
        let user = make_user(&pool, "user@example.com").await;
        let alpha = make_site(&pool, "alpha.example.com").await;
        let beta = make_site(&pool, "beta.example.com").await;

        assert!(
            list_site_ids_for_user(&pool, &user)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(!user_can_access_site(&pool, &user, &alpha).await.unwrap());

        set_sites_for_user(&pool, &user, &[alpha.clone(), beta.clone()])
            .await
            .unwrap();
        let listed = list_site_ids_for_user(&pool, &user).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert!(site_ids_contain(&listed, &alpha));
        assert!(user_can_access_site(&pool, &user, &beta).await.unwrap());

        // Replacing shrinks the set and ignores duplicates.
        set_sites_for_user(&pool, &user, &[beta.clone(), beta.clone()])
            .await
            .unwrap();
        assert_eq!(
            list_site_ids_for_user(&pool, &user).await.unwrap(),
            vec![beta.clone()]
        );
        assert!(!user_can_access_site(&pool, &user, &alpha).await.unwrap());
    }

    #[tokio::test]
    async fn grants_cascade_on_delete() {
        let pool = test_pool().await;
        let user = make_user(&pool, "user@example.com").await;
        let site = make_site(&pool, "alpha.example.com").await;
        set_sites_for_user(&pool, &user, &[site.clone()])
            .await
            .unwrap();

        Sites::new(pool.clone()).delete(&site).await.unwrap();
        assert!(
            list_site_ids_for_user(&pool, &user)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn unknown_user_is_rejected() {
        let pool = test_pool().await;
        let result = set_sites_for_user(&pool, "does-not-exist", &[]).await;
        assert!(result.is_err());
    }

    #[test]
    fn pure_membership_check() {
        let ids = vec!["a".to_string(), "b".to_string()];
        assert!(site_ids_contain(&ids, "a"));
        assert!(!site_ids_contain(&ids, "c"));
    }
}
