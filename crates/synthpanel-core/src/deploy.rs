//! Git-based deployments.
//!
//! A deployment describes where a site's code comes from: a Git repository, a
//! branch, an optional sub-directory and an optional build command. Running a
//! deployment clones or updates the repository, builds it, and publishes the
//! result into the site's document root. The previous release is kept so a
//! failed build or a failed publish can be rolled back automatically.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// A deployment configuration.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Deployment {
    pub id: String,
    pub site_id: String,
    pub repo_url: String,
    pub branch: String,
    pub target_subdir: String,
    pub build_command: String,
    pub deploy_user: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// The result of one deployment run.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DeployRun {
    pub id: String,
    pub deployment_id: String,
    pub site_id: String,
    pub commit_sha: String,
    pub branch: String,
    pub status: String,
    pub message: String,
    pub output: String,
    pub started_at: String,
    pub finished_at: Option<String>,
}

/// Fields that can be changed on a deployment.
#[derive(Debug, Clone, Default)]
pub struct DeploymentPatch {
    pub repo_url: Option<String>,
    pub branch: Option<String>,
    pub target_subdir: Option<String>,
    pub build_command: Option<String>,
    pub deploy_user: Option<String>,
    pub enabled: Option<bool>,
}

/// List the deployments of a site (or all of them).
pub async fn list(pool: &SqlitePool, site_id: Option<&str>) -> Result<Vec<Deployment>> {
    let rows = match site_id {
        Some(site_id) => {
            sqlx::query_as::<_, Deployment>(
                "SELECT * FROM deployments WHERE site_id = ? ORDER BY created_at DESC",
            )
            .bind(site_id)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, Deployment>("SELECT * FROM deployments ORDER BY created_at DESC")
                .fetch_all(pool)
                .await?
        }
    };
    Ok(rows)
}

/// Load one deployment.
pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<Deployment>> {
    Ok(
        sqlx::query_as::<_, Deployment>("SELECT * FROM deployments WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Create a deployment.
pub async fn create(
    pool: &SqlitePool,
    site_id: &str,
    repo_url: &str,
    branch: &str,
    target_subdir: &str,
    build_command: &str,
    deploy_user: &str,
) -> Result<Deployment> {
    if repo_url.trim().is_empty() {
        bail!("the repository url is empty");
    }
    if !repo_url.starts_with("http://")
        && !repo_url.starts_with("https://")
        && !repo_url.starts_with("git@")
        && !repo_url.starts_with("ssh://")
        && !repo_url.starts_with("file://")
    {
        bail!("the repository url must be http(s), ssh, git@ or file");
    }
    let now = chrono::Utc::now().to_rfc3339();
    let deployment = Deployment {
        id: uuid::Uuid::new_v4().to_string(),
        site_id: site_id.to_string(),
        repo_url: repo_url.trim().to_string(),
        branch: if branch.trim().is_empty() {
            "main".to_string()
        } else {
            branch.trim().to_string()
        },
        target_subdir: target_subdir.trim().to_string(),
        build_command: build_command.trim().to_string(),
        deploy_user: if deploy_user.trim().is_empty() {
            "www-data".to_string()
        } else {
            deploy_user.trim().to_string()
        },
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO deployments
         (id, site_id, repo_url, branch, target_subdir, build_command, deploy_user,
          enabled, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&deployment.id)
    .bind(&deployment.site_id)
    .bind(&deployment.repo_url)
    .bind(&deployment.branch)
    .bind(&deployment.target_subdir)
    .bind(&deployment.build_command)
    .bind(&deployment.deploy_user)
    .bind(deployment.enabled)
    .bind(&deployment.created_at)
    .bind(&deployment.updated_at)
    .execute(pool)
    .await?;
    Ok(deployment)
}

/// Update a deployment.
pub async fn update(pool: &SqlitePool, id: &str, patch: DeploymentPatch) -> Result<Deployment> {
    let mut deployment = get(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the deployment `{id}` does not exist"))?;
    if let Some(value) = patch.repo_url {
        deployment.repo_url = value.trim().to_string();
    }
    if let Some(value) = patch.branch {
        deployment.branch = value.trim().to_string();
    }
    if let Some(value) = patch.target_subdir {
        deployment.target_subdir = value.trim().to_string();
    }
    if let Some(value) = patch.build_command {
        deployment.build_command = value.trim().to_string();
    }
    if let Some(value) = patch.deploy_user {
        deployment.deploy_user = value.trim().to_string();
    }
    if let Some(value) = patch.enabled {
        deployment.enabled = value;
    }
    deployment.updated_at = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE deployments
         SET repo_url = ?, branch = ?, target_subdir = ?, build_command = ?,
             deploy_user = ?, enabled = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&deployment.repo_url)
    .bind(&deployment.branch)
    .bind(&deployment.target_subdir)
    .bind(&deployment.build_command)
    .bind(&deployment.deploy_user)
    .bind(deployment.enabled)
    .bind(&deployment.updated_at)
    .bind(&deployment.id)
    .execute(pool)
    .await?;
    Ok(deployment)
}

/// Delete a deployment.
pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool> {
    let deleted = sqlx::query("DELETE FROM deployments WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0;
    if deleted {
        sqlx::query("DELETE FROM deploy_runs WHERE deployment_id = ?")
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(deleted)
}

/// List the run history of a deployment.
pub async fn list_runs(
    pool: &SqlitePool,
    deployment_id: &str,
    limit: i64,
) -> Result<Vec<DeployRun>> {
    Ok(sqlx::query_as::<_, DeployRun>(
        "SELECT * FROM deploy_runs WHERE deployment_id = ? ORDER BY started_at DESC LIMIT ?",
    )
    .bind(deployment_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?)
}

/// Where a deployment keeps its working files.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// The base directory for all deployment workspaces.
    pub base_dir: PathBuf,
}

/// Run a deployment and publish it to the document root.
pub async fn run(
    pool: &SqlitePool,
    deployment: &Deployment,
    document_root: &str,
    workspace: &Workspace,
) -> Result<DeployRun> {
    let started_at = chrono::Utc::now().to_rfc3339();
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut output = String::new();

    let result = execute(deployment, document_root, workspace, &mut output).await;

    let (status, message, commit) = match result {
        Ok(commit) => (
            "ok".to_string(),
            "the deployment succeeded".to_string(),
            commit,
        ),
        Err(error) => {
            warn!(deployment = %deployment.id, error = %error, "the deployment failed");
            ("failed".to_string(), error.to_string(), String::new())
        }
    };

    let run = DeployRun {
        id: run_id,
        deployment_id: deployment.id.clone(),
        site_id: deployment.site_id.clone(),
        commit_sha: commit,
        branch: deployment.branch.clone(),
        status: status.clone(),
        message: message.clone(),
        output: output.clone(),
        started_at,
        finished_at: Some(chrono::Utc::now().to_rfc3339()),
    };

    sqlx::query(
        "INSERT INTO deploy_runs
         (id, deployment_id, site_id, commit_sha, branch, status, message, output,
          started_at, finished_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&run.id)
    .bind(&run.deployment_id)
    .bind(&run.site_id)
    .bind(&run.commit_sha)
    .bind(&run.branch)
    .bind(&run.status)
    .bind(&run.message)
    .bind(&run.output)
    .bind(&run.started_at)
    .bind(&run.finished_at)
    .execute(pool)
    .await?;

    Ok(run)
}

/// The deployment steps.
async fn execute(
    deployment: &Deployment,
    document_root: &str,
    workspace: &Workspace,
    output: &mut String,
) -> Result<String> {
    let root = workspace.base_dir.join(&deployment.id);
    let checkout = root.join("checkout");
    let releases = root.join("releases");
    let previous = root.join("previous");
    tokio::fs::create_dir_all(&releases).await?;
    tokio::fs::create_dir_all(&previous).await?;

    // 1. Fetch the code.
    let commit = if checkout.join(".git").exists() {
        let fetch = git(Some(&checkout), &["fetch", "--prune", "origin"]).await;
        match fetch {
            Ok(_) => {
                let checkout_branch = git(
                    Some(&checkout),
                    &["checkout", "--force", &deployment.branch],
                )
                .await;
                if let Err(error) = checkout_branch {
                    output.push_str(&format!("{error}\n"));
                }
                let reset = git(
                    Some(&checkout),
                    &["reset", "--hard", &format!("origin/{}", deployment.branch)],
                )
                .await;
                if let Err(error) = reset {
                    // Fall back to whatever FETCH_HEAD points at.
                    output.push_str(&format!("{error}\n"));
                    git(Some(&checkout), &["reset", "--hard", "FETCH_HEAD"]).await?;
                }
            }
            Err(error) => {
                output.push_str(&format!("{error}\n"));
                bail!("the repository could not be fetched: {error}");
            }
        }
        git(Some(&checkout), &["rev-parse", "HEAD"]).await?
    } else {
        let _ = tokio::fs::remove_dir_all(&checkout).await;
        if let Some(parent) = checkout.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let clone = git(
            None,
            &[
                "clone",
                "--branch",
                &deployment.branch,
                "--single-branch",
                &deployment.repo_url,
                &checkout.to_string_lossy(),
            ],
        )
        .await;
        if let Err(error) = clone {
            output.push_str(&format!("{error}\n"));
            // The branch may not exist in a fresh repository; clone everything
            // and check the branch out afterwards.
            let _ = tokio::fs::remove_dir_all(&checkout).await;
            git(
                None,
                &["clone", &deployment.repo_url, &checkout.to_string_lossy()],
            )
            .await?;
            git(
                Some(&checkout),
                &["checkout", "--force", &deployment.branch],
            )
            .await?;
        }
        git(Some(&checkout), &["rev-parse", "HEAD"]).await?
    };
    output.push_str(&format!("commit {commit}\n"));

    // 2. Build.
    let source = if deployment.target_subdir.is_empty() {
        checkout.clone()
    } else {
        checkout.join(&deployment.target_subdir)
    };
    if !source.exists() {
        bail!("the source directory {} does not exist", source.display());
    }
    if !deployment.build_command.is_empty() {
        let result = run_as_user(&deployment.deploy_user, &source, &deployment.build_command).await;
        match result {
            Ok(text) => output.push_str(&text),
            Err(error) => {
                output.push_str(&format!("{error}\n"));
                bail!("the build command failed: {error}");
            }
        }
    }

    // 3. Publish.
    let release = releases.join(commit.trim());
    let _ = tokio::fs::remove_dir_all(&release).await;
    copy_dir(&source, &release).await?;
    let release = if deployment.target_subdir.is_empty() {
        release
    } else {
        release.join(&deployment.target_subdir)
    };

    let document_root = PathBuf::from(document_root);
    tokio::fs::create_dir_all(&document_root).await?;
    let _ = tokio::fs::remove_dir_all(&previous).await;
    if document_root.exists() {
        copy_dir(&document_root, &previous).await?;
    }

    if let Err(error) = publish(&release, &document_root).await {
        // Restore the previous release.
        if previous.exists() {
            let _ = publish(&previous, &document_root).await;
        }
        bail!("the release could not be published: {error}");
    }

    // 4. Hand the files to the deploy user.
    if !deployment.deploy_user.is_empty() && deployment.deploy_user != "root" {
        let _ = chown_tree(&document_root, &deployment.deploy_user).await;
    }

    info!(deployment = %deployment.id, commit = %commit, "deployment published");
    Ok(commit)
}

/// Replace the contents of `target` with the contents of `source`.
async fn publish(source: &Path, target: &Path) -> Result<()> {
    if !source.exists() {
        bail!("the release {} does not exist", source.display());
    }
    let staging = target.with_extension("staging");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    copy_dir(source, &staging).await?;
    // Swap the directories so the live root is never half-written.
    let old = target.with_extension("old");
    let _ = tokio::fs::remove_dir_all(&old).await;
    if target.exists() {
        tokio::fs::rename(target, &old).await?;
    }
    if let Err(error) = tokio::fs::rename(&staging, target).await {
        // Put the old root back.
        if old.exists() {
            let _ = tokio::fs::rename(&old, target).await;
        }
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(error.into());
    }
    let _ = tokio::fs::remove_dir_all(&old).await;
    Ok(())
}

/// Copy a directory tree, skipping `.git`.
pub async fn copy_dir(source: &Path, target: &Path) -> Result<()> {
    tokio::fs::create_dir_all(target).await?;
    let mut entries = tokio::fs::read_dir(source).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let from = entry.path();
        let to = target.join(&name);
        let kind = entry.file_type().await?;
        if kind.is_symlink() {
            let target_link = tokio::fs::read_link(&from).await?;
            let _ = tokio::fs::remove_file(&to).await;
            std::os::unix::fs::symlink(target_link, &to)?;
        } else if kind.is_dir() {
            Box::pin(copy_dir(&from, &to)).await?;
        } else {
            tokio::fs::copy(&from, &to).await?;
        }
    }
    Ok(())
}

/// Change the owner of a tree.
async fn chown_tree(path: &Path, user: &str) -> Result<()> {
    let output = tokio::process::Command::new("chown")
        .arg("-R")
        .arg(user)
        .arg(path)
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "cannot change the owner to `{user}`: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Run a shell command, as `user` when possible.
async fn run_as_user(user: &str, dir: &Path, script: &str) -> Result<String> {
    let runuser = which("runuser").is_some();
    let mut process = if runuser && !user.is_empty() && user != "root" {
        let mut process = tokio::process::Command::new("runuser");
        process
            .arg("-u")
            .arg(user)
            .arg("--")
            .arg("sh")
            .arg("-c")
            .arg(script);
        process
    } else {
        let mut process = tokio::process::Command::new("sh");
        process.arg("-c").arg(script);
        process
    };
    let output = process
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .await?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() {
        bail!("the command exited with {}", output.status);
    }
    Ok(text)
}

/// Run a git command.
async fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut command = tokio::process::Command::new("git");
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    command.args(args);
    let output = command
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .context("cannot run git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Find a binary on PATH.
fn which(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(binary);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use sqlx::SqlitePool;

    async fn test_pool() -> SqlitePool {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-deploy-{}.db", uuid::Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn deployments_roundtrip() {
        let pool = test_pool().await;
        let deployment = create(
            &pool,
            "site-1",
            "https://example.com/app.git",
            "main",
            "public",
            "npm run build",
            "www-data",
        )
        .await
        .unwrap();
        assert_eq!(deployment.branch, "main");
        assert_eq!(deployment.deploy_user, "www-data");

        assert_eq!(list(&pool, Some("site-1")).await.unwrap().len(), 1);
        assert_eq!(list(&pool, Some("other")).await.unwrap().len(), 0);

        let updated = update(
            &pool,
            &deployment.id,
            DeploymentPatch {
                branch: Some("develop".into()),
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.branch, "develop");
        assert!(!updated.enabled);

        assert!(delete(&pool, &deployment.id).await.unwrap());
        assert!(get(&pool, &deployment.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rejects_an_invalid_repository_url() {
        let pool = test_pool().await;
        assert!(
            create(
                &pool,
                "site-1",
                "ftp://example.com",
                "main",
                "",
                "",
                "www-data"
            )
            .await
            .is_err()
        );
        assert!(
            create(&pool, "site-1", "  ", "main", "", "", "www-data")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_failed_run_is_recorded() {
        let pool = test_pool().await;
        let deployment = create(
            &pool,
            "site-1",
            "file:///nonexistent/repository.git",
            "main",
            "",
            "",
            "www-data",
        )
        .await
        .unwrap();
        let dir = std::env::temp_dir().join(format!("sp-deploy-{}", uuid::Uuid::new_v4()));
        let workspace = Workspace {
            base_dir: dir.clone(),
        };
        let run = run(
            &pool,
            &deployment,
            dir.join("public").to_string_lossy().as_ref(),
            &workspace,
        )
        .await
        .unwrap();
        assert_eq!(run.status, "failed");
        assert!(!run.message.is_empty());
        let runs = list_runs(&pool, &deployment.id, 10).await.unwrap();
        assert_eq!(runs.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn copies_a_tree_and_skips_git() {
        let dir = std::env::temp_dir().join(format!("sp-deploy-{}", uuid::Uuid::new_v4()));
        let source = dir.join("source");
        let target = dir.join("target");
        tokio::fs::create_dir_all(source.join(".git"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(source.join("public"))
            .await
            .unwrap();
        tokio::fs::write(source.join(".git/config"), "secret")
            .await
            .unwrap();
        tokio::fs::write(source.join("public/index.php"), "<?php")
            .await
            .unwrap();
        copy_dir(&source, &target).await.unwrap();
        assert!(target.join("public/index.php").exists());
        assert!(!target.join(".git").exists());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn publishing_swaps_the_content() {
        let dir = std::env::temp_dir().join(format!("sp-deploy-{}", uuid::Uuid::new_v4()));
        let release = dir.join("release");
        let target = dir.join("public");
        tokio::fs::create_dir_all(&release).await.unwrap();
        tokio::fs::write(release.join("index.php"), "new")
            .await
            .unwrap();
        tokio::fs::create_dir_all(&target).await.unwrap();
        tokio::fs::write(target.join("old.txt"), "old")
            .await
            .unwrap();
        publish(&release, &target).await.unwrap();
        assert!(target.join("index.php").exists());
        assert!(!target.join("old.txt").exists());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
