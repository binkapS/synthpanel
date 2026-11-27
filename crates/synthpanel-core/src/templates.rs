//! Framework and application templates.
//!
//! `templates.list` exposes a small catalogue of ready-to-install application
//! templates. `templates.install` provisions one of them into an existing
//! site, updates the site's template/document root, and returns the id of the
//! installed template.
//!
//! Security properties of this module:
//! - package managers are invoked through `Command` with discrete arguments;
//!   there is no shell, no `sh -c` and no string interpolation into a shell;
//! - caller-supplied versions, binaries and URLs are validated before they
//!   reach a command line;
//! - a local package directory can be supplied with
//!   `SYNTHPANEL_TEMPLATE_SOURCE_DIR` (or the per-template
//!   `SYNTHPANEL_TEMPLATE_SOURCE_<ID>`), which makes installs fully offline
//!   and testable without a registry or a package manager;
//! - WordPress is *not* reimplemented here: the `wordpress` id delegates to
//!   [`crate::wordpress`], including its `SYNTHPANEL_WORDPRESS_ARCHIVE`
//!   fixture override and its archive validation.

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};
use uuid::Uuid;

/// The WordPress catalogue id. The installer for this id is owned by
/// [`crate::wordpress`] — this module only routes to it.
pub const WORDPRESS_ID: &str = "wordpress";

/// `SYNTHPANEL_TEMPLATE_SOURCE_DIR/<id>` holds a prepared package tree for an
/// offline install.
pub const ENV_SOURCE_DIR: &str = "SYNTHPANEL_TEMPLATE_SOURCE_DIR";
pub const ENV_COMPOSER_BIN: &str = "SYNTHPANEL_COMPOSER_BIN";
pub const ENV_NPM_BIN: &str = "SYNTHPANEL_NPM_BIN";
pub const ENV_CURL_BIN: &str = "SYNTHPANEL_CURL_BIN";
pub const ENV_TAR_BIN: &str = "SYNTHPANEL_TAR_BIN";
pub const ENV_HUGO_BIN: &str = "SYNTHPANEL_HUGO_BIN";
pub const ENV_ZOLA_BIN: &str = "SYNTHPANEL_ZOLA_BIN";
pub const ENV_MKDIR_BIN: &str = "SYNTHPANEL_MKDIR_BIN";
pub const ENV_NEXTCLOUD_URL: &str = "SYNTHPANEL_NEXTCLOUD_URL";
pub const ENV_WP_ADMIN_USER: &str = "SYNTHPANEL_TEMPLATE_WORDPRESS_ADMIN_USER";
pub const ENV_WP_ADMIN_PASSWORD: &str = "SYNTHPANEL_TEMPLATE_WORDPRESS_ADMIN_PASSWORD";
pub const ENV_WP_ADMIN_EMAIL: &str = "SYNTHPANEL_TEMPLATE_WORDPRESS_ADMIN_EMAIL";

/// A row of the `templates.list` response.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct TemplateInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
}

/// How a template's files are obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupKind {
    /// `composer create-project`
    Composer,
    /// `npm create ...`
    Npm,
    /// Download a release archive and unpack it into the site root.
    Archive,
    /// Run a site-generator binary (`hugo new site`, `zola init`).
    Generator,
    /// Delegated to the dedicated WordPress installer.
    WordPress,
}

/// Per-template setup description. `source` is always passed as a *single*
/// command argument (a Composer package, an npm package, a generator name or
/// a base URL); it is never parsed by a shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateSetup {
    pub info: TemplateInfo,
    pub kind: SetupKind,
    pub source: &'static str,
    /// Sub-directory of the install root that nginx should serve.
    pub docroot_subdir: &'static str,
}

/// The catalogue, in the order shown to users.
pub const TEMPLATES: &[TemplateSetup] = &[
    TemplateSetup {
        info: TemplateInfo {
            id: "laravel",
            label: "Laravel",
            category: "php",
        },
        kind: SetupKind::Composer,
        source: "laravel/laravel",
        docroot_subdir: "public",
    },
    TemplateSetup {
        info: TemplateInfo {
            id: "symfony",
            label: "Symfony",
            category: "php",
        },
        kind: SetupKind::Composer,
        source: "symfony/skeleton",
        docroot_subdir: "public",
    },
    TemplateSetup {
        info: TemplateInfo {
            id: "magento",
            label: "Magento",
            category: "php",
        },
        kind: SetupKind::Composer,
        source: "magento/project-community-edition",
        docroot_subdir: "pub",
    },
    TemplateSetup {
        info: TemplateInfo {
            id: "nextcloud",
            label: "Nextcloud",
            category: "php",
        },
        kind: SetupKind::Archive,
        source: "https://download.nextcloud.com/server/releases",
        docroot_subdir: "",
    },
    TemplateSetup {
        info: TemplateInfo {
            id: "nextjs",
            label: "Next.js",
            category: "node",
        },
        kind: SetupKind::Npm,
        source: "next-app",
        docroot_subdir: "",
    },
    TemplateSetup {
        info: TemplateInfo {
            id: "strapi",
            label: "Strapi",
            category: "node",
        },
        kind: SetupKind::Npm,
        source: "strapi-app",
        docroot_subdir: "",
    },
    TemplateSetup {
        info: TemplateInfo {
            id: "hugo",
            label: "Hugo",
            category: "static",
        },
        kind: SetupKind::Generator,
        source: "hugo",
        docroot_subdir: "public",
    },
    TemplateSetup {
        info: TemplateInfo {
            id: "zola",
            label: "Zola",
            category: "static",
        },
        kind: SetupKind::Generator,
        source: "zola",
        docroot_subdir: "public",
    },
    TemplateSetup {
        info: TemplateInfo {
            id: WORDPRESS_ID,
            label: "WordPress",
            category: "cms",
        },
        kind: SetupKind::WordPress,
        source: WORDPRESS_ID,
        docroot_subdir: "",
    },
];

/// The payload of `templates.list`.
pub fn list() -> Vec<TemplateInfo> {
    TEMPLATES.iter().map(|setup| setup.info).collect()
}

/// Look up a template by its (case-insensitive) id.
pub fn by_id(id: &str) -> Option<&'static TemplateSetup> {
    let id = id.trim().to_ascii_lowercase();
    TEMPLATES.iter().find(|setup| setup.info.id == id)
}

/// Arguments of the `templates.install` IPC method.
#[derive(Debug, Clone, Copy)]
pub struct InstallRequest<'a> {
    pub site_id: &'a str,
    pub template: &'a str,
    pub version: Option<&'a str>,
}

/// Result of the `templates.install` IPC method.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallResult {
    pub ok: bool,
    pub template: String,
}

/// One external program invocation. `args` contains only validated,
/// shell-free values; `binary_env` names the env var that may replace
/// `binary` (for tests and for distributions with non-standard paths).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub binary_env: &'static str,
    pub binary: &'static str,
    pub args: Vec<String>,
}

impl CommandSpec {
    /// Resolve the binary, honouring the env override.
    pub fn resolve(&self) -> Result<String> {
        resolve_binary(self.binary_env, self.binary)
    }
}

/// A complete, purely computed setup plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupPlan {
    pub steps: Vec<CommandSpec>,
    /// Temporary files created by a step; removed after the plan runs.
    pub cleanup: Vec<PathBuf>,
}

/// Accept a version pin while rejecting anything that could be read as a
/// shell fragment or a path. `None` means "use the package's latest release".
pub fn validate_version(version: &str) -> Result<String> {
    let value = version.trim();
    if value.is_empty() {
        bail!("the template version is empty");
    }
    if value.len() > 64 {
        bail!("the template version is longer than 64 characters");
    }
    if !value.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        bail!("the template version must start with a digit");
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
    {
        bail!("the template version contains unsupported characters");
    }
    if value.contains("..") {
        bail!("the template version contains `..`");
    }
    Ok(value.to_string())
}

/// Reject shell metacharacters in an overridden binary name or path.
pub fn validate_binary(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("the binary override is empty");
    }
    if value.len() > 256 {
        bail!("the binary override is longer than 256 characters");
    }
    if !value.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '/' | '.' | '-' | '_' | '+' | ':' | '@' | '=' | ',' | '~')
    }) {
        bail!("the binary override contains unsupported characters: {value}");
    }
    Ok(value.to_string())
}

/// Reject URLs that could inject another argument or a shell command.
pub fn validate_url(value: &str) -> Result<String> {
    let value = value.trim();
    if !value.starts_with("https://") {
        bail!("the template source URL must use https://");
    }
    if value.len() > 512
        || value.chars().any(|c| {
            c.is_ascii_whitespace()
                || c.is_control()
                || matches!(
                    c,
                    ';' | '$' | '`' | '"' | '\'' | '\\' | '<' | '>' | '{' | '}'
                )
        })
    {
        bail!("the template source URL contains unsupported characters");
    }
    Ok(value.to_string())
}

fn resolve_binary(env_key: &str, default: &str) -> Result<String> {
    match std::env::var(env_key) {
        Ok(value) if !value.trim().is_empty() => validate_binary(&value)
            .with_context(|| format!("invalid {env_key}; expected a single binary name or path")),
        _ => Ok(default.to_string()),
    }
}

fn spec(binary_env: &'static str, binary: &'static str, args: Vec<String>) -> CommandSpec {
    CommandSpec {
        binary_env,
        binary,
        args,
    }
}

/// Build the shell-free command plan for a template.
///
/// `base` is the directory the application is installed into, `site_id` is
/// used for a deterministic download staging path, and `domain` is reserved
/// for future template-specific arguments (it is never interpolated into a
/// command string).
pub fn plan(
    setup: &TemplateSetup,
    version: Option<&str>,
    base: &Path,
    site_id: &str,
    domain: &str,
) -> Result<SetupPlan> {
    let _ = domain;
    let site_id = validate_binary(site_id)?;
    let base_arg = base.display().to_string();
    let version = match version {
        Some(value) => Some(validate_version(value)?),
        None => None,
    };

    let mut plan = SetupPlan::default();
    match setup.kind {
        SetupKind::Composer => {
            let mut args = vec!["create-project".to_string()];
            if setup.info.id == "magento" {
                args.push("--repository-url=https://repo.magento.com/".to_string());
            }
            args.push(setup.source.to_string());
            args.push(base_arg);
            if let Some(version) = version {
                args.push(version);
            }
            args.push("--no-interaction".to_string());
            args.push("--prefer-dist".to_string());
            plan.steps.push(spec(ENV_COMPOSER_BIN, "composer", args));
        }
        SetupKind::Npm => {
            let release = version.unwrap_or_else(|| "latest".to_string());
            let package = format!("{}@{release}", setup.source);
            let mut args = vec!["create".to_string(), package, base_arg, "--yes".to_string()];
            if setup.info.id == "strapi" {
                args.push("--no-run".to_string());
            }
            plan.steps.push(spec(ENV_NPM_BIN, "npm", args));
        }
        SetupKind::Archive => {
            if setup.info.id != "nextcloud" {
                bail!("no archive plan for template `{}`", setup.info.id);
            }
            let url = match std::env::var(ENV_NEXTCLOUD_URL) {
                Ok(value) if !value.trim().is_empty() => validate_url(&value)?,
                _ => {
                    let path = match &version {
                        Some(version) => format!("nextcloud-{version}.tar.bz2"),
                        None => "latest.tar.bz2".to_string(),
                    };
                    validate_url(&format!("{}/{}", setup.source, path))?
                }
            };
            let archive = std::env::temp_dir().join(format!(
                "synthpanel-nextcloud-{site_id}-{}.tar.bz2",
                Uuid::new_v4()
            ));
            let archive_arg = archive.display().to_string();
            plan.steps.push(spec(
                ENV_CURL_BIN,
                "curl",
                vec![
                    "--fail".to_string(),
                    "--location".to_string(),
                    "--silent".to_string(),
                    "--show-error".to_string(),
                    "--output".to_string(),
                    archive_arg.clone(),
                    url,
                ],
            ));
            plan.steps.push(spec(
                ENV_TAR_BIN,
                "tar",
                vec![
                    "--extract".to_string(),
                    "--bzip2".to_string(),
                    "--strip-components=1".to_string(),
                    "--file".to_string(),
                    archive_arg,
                    "--directory".to_string(),
                    base_arg,
                ],
            ));
            plan.cleanup.push(archive);
        }
        SetupKind::Generator => {
            let (binary_env, binary, args): (&str, &str, Vec<String>) = match setup.info.id {
                "hugo" => (
                    ENV_HUGO_BIN,
                    "hugo",
                    vec!["new".to_string(), "site".to_string(), base_arg.clone()],
                ),
                "zola" => (
                    ENV_ZOLA_BIN,
                    "zola",
                    vec!["init".to_string(), base_arg.clone()],
                ),
                other => bail!("no generator plan for template `{other}`"),
            };
            plan.steps.push(spec(binary_env, binary, args));
            if !setup.docroot_subdir.is_empty() {
                plan.steps.push(spec(
                    ENV_MKDIR_BIN,
                    "mkdir",
                    vec![
                        "-p".to_string(),
                        base.join(setup.docroot_subdir).display().to_string(),
                    ],
                ));
            }
        }
        SetupKind::WordPress => {
            // No commands here: `install` routes this id to
            // `crate::wordpress::install` so WordPress setup is not duplicated.
        }
    }
    Ok(plan)
}

/// Where a framework is installed. When the site's document root already
/// ends in the template's public sub-directory, its parent is the install
/// root, which makes repeated installs idempotent.
pub fn install_base(document_root: &str, subdir: &str) -> Result<PathBuf> {
    let root = Path::new(document_root.trim());
    if !root.is_absolute() || root == Path::new("/") {
        bail!("the site's document root must be an absolute directory path");
    }
    if root
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        bail!("the site's document root contains `..`");
    }
    let subdir = subdir.trim();
    if subdir.is_empty() {
        return Ok(root.to_path_buf());
    }
    let sub = Path::new(subdir);
    if sub.is_absolute()
        || sub.components().count() != 1
        || !sub
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        bail!("the template docroot sub-directory is invalid: {subdir}");
    }
    if root.file_name() == Some(OsStr::new(subdir)) {
        // The document root already includes the public directory: install
        // into its parent so re-running an install never nests directories.
        Ok(root
            .parent()
            .filter(|parent| *parent != Path::new("/"))
            .unwrap_or(root)
            .to_path_buf())
    } else {
        // A generic site root is the application base; the template's
        // published directory becomes the new document root afterwards.
        Ok(root.to_path_buf())
    }
}

/// The document root that serves an installed template.
pub fn effective_docroot(base: &Path, subdir: &str) -> PathBuf {
    if subdir.trim().is_empty() {
        base.to_path_buf()
    } else {
        base.join(subdir.trim())
    }
}

fn safe_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && !name.chars().any(char::is_control)
}

/// Copy a local, prebuilt package tree into the install root.
///
/// Symlinks and special files are refused so a fixture cannot write outside
/// the site. This is the offline path used by `SYNTHPANEL_TEMPLATE_SOURCE_*`.
pub fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let source = source
        .canonicalize()
        .with_context(|| format!("cannot read template source {}", source.display()))?;
    if !source.is_dir() {
        bail!(
            "the template source is not a directory: {}",
            source.display()
        );
    }
    fs::create_dir_all(target)?;
    let mut pending = vec![(source, target.to_path_buf())];
    let mut visited = 0usize;
    while let Some((from, to)) = pending.pop() {
        for entry in fs::read_dir(&from)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| anyhow!("a template source name is not UTF-8"))?;
            if !safe_component(name) {
                bail!("unsafe name in template source: {name}");
            }
            visited += 1;
            if visited > 100_000 {
                bail!("the template source has too many files");
            }
            let child_from = entry.path();
            let child_to = to.join(name);
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                bail!(
                    "template source contains a symlink: {}",
                    child_from.display()
                );
            }
            if kind.is_dir() {
                fs::create_dir_all(&child_to)?;
                pending.push((child_from, child_to));
            } else if kind.is_file() {
                fs::copy(&child_from, &child_to)
                    .with_context(|| format!("cannot copy {}", child_from.display()))?;
            } else {
                bail!(
                    "unsupported file in template source: {}",
                    child_from.display()
                );
            }
        }
    }
    Ok(())
}

/// Resolve the offline source override for a template, if one is configured.
fn source_override(setup: &TemplateSetup) -> Result<Option<PathBuf>> {
    // WordPress owns its own archive override (`SYNTHPANEL_WORDPRESS_ARCHIVE`).
    if setup.kind == SetupKind::WordPress {
        return Ok(None);
    }
    let specific = format!(
        "SYNTHPANEL_TEMPLATE_SOURCE_{}",
        setup.info.id.to_ascii_uppercase()
    );
    let candidates = [
        std::env::var(&specific).ok(),
        std::env::var(ENV_SOURCE_DIR)
            .ok()
            .map(|root| Path::new(&root).join(setup.info.id).display().to_string()),
    ];
    for value in candidates.into_iter().flatten() {
        if value.trim().is_empty() {
            continue;
        }
        let path = PathBuf::from(value.trim());
        if path.is_dir() {
            return Ok(Some(path));
        }
        warn!(
            template = setup.info.id,
            path = %path.display(),
            "the configured template source is not a directory"
        );
    }
    Ok(None)
}

fn run_step(step: &CommandSpec, cwd: &Path) -> Result<()> {
    let binary = step.resolve()?;
    let output = Command::new(&binary)
        .args(&step.args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("cannot run `{binary}`; set {}", step.binary_env))?;
    if !output.status.success() {
        bail!(
            "{binary} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn own_files(base: &Path, owner: &str) -> Result<()> {
    let args = crate::wordpress::chown_args(owner, base, true)?;
    let binary = resolve_binary("SYNTHPANEL_CHOWN_BIN", "chown")?;
    let output = Command::new(&binary)
        .args(&args)
        .output()
        .with_context(|| format!("cannot run `{binary}`; set SYNTHPANEL_CHOWN_BIN"))?;
    if !output.status.success() {
        bail!(
            "{binary} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

async fn install_wordpress(pool: &SqlitePool, site_id: &str, domain: &str) -> Result<()> {
    let user = std::env::var(ENV_WP_ADMIN_USER).unwrap_or_else(|_| "admin".to_string());
    let email = std::env::var(ENV_WP_ADMIN_EMAIL).unwrap_or_else(|_| format!("admin@{domain}"));
    let password = match std::env::var(ENV_WP_ADMIN_PASSWORD) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            warn!(
                "no WordPress admin password configured; generating one (set {ENV_WP_ADMIN_PASSWORD} to choose it)"
            );
            crate::database::generate_password(24)
        }
    };
    crate::wordpress::validate_admin(&user, &password, &email)?;
    crate::wordpress::install(
        pool,
        crate::wordpress::InstallRequest {
            site_id,
            admin_user: &user,
            admin_password: &password,
            admin_email: &email,
        },
    )
    .await?;
    Ok(())
}

/// Install a template into an existing site.
pub async fn install(pool: &SqlitePool, request: InstallRequest<'_>) -> Result<InstallResult> {
    let site_id = request.site_id.trim();
    if site_id.is_empty() {
        bail!("site_id must not be empty");
    }
    let setup = by_id(request.template)
        .ok_or_else(|| anyhow!("unknown template `{}`", request.template.trim()))?;

    let site: crate::sites::SiteRecord = sqlx::query_as("SELECT * FROM sites WHERE id = ?")
        .bind(site_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow!("site not found"))?;

    let base = install_base(&site.document_root, setup.docroot_subdir)?;
    fs::create_dir_all(&base).with_context(|| format!("cannot create {}", base.display()))?;

    match source_override(setup)? {
        Some(source) => {
            info!(
                template = setup.info.id,
                source = %source.display(),
                "using a local template source fixture"
            );
            copy_tree(&source, &base)?;
        }
        None if setup.kind == SetupKind::WordPress => {
            install_wordpress(pool, site_id, &site.domain).await?;
        }
        None => {
            let plan = plan(setup, request.version, &base, site_id, &site.domain)?;
            for step in &plan.steps {
                run_step(step, &base)?;
            }
            for path in &plan.cleanup {
                let _ = fs::remove_file(path);
            }
        }
    }

    // WordPress applies its own ownership while writing wp-config.php.
    if setup.kind != SetupKind::WordPress {
        own_files(&base, &site.pool_user)?;
    }

    let docroot = effective_docroot(&base, setup.docroot_subdir);
    crate::sites::Sites::new(pool.clone())
        .update(
            site_id,
            crate::sites::SitePatch {
                template: Some(setup.info.id.to_string()),
                document_root: Some(docroot.display().to_string()),
                ..Default::default()
            },
        )
        .await?;

    Ok(InstallResult {
        ok: true,
        template: setup.info.id.to_string(),
    })
}

/// Convenience wrapper for callers that already hold the three fields.
pub async fn install_site(
    pool: &SqlitePool,
    site_id: &str,
    template: &str,
    version: Option<&str>,
) -> Result<InstallResult> {
    install(
        pool,
        InstallRequest {
            site_id,
            template,
            version,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_covers_all_nine_templates() {
        let rows = list();
        assert_eq!(rows.len(), 9);
        let ids: Vec<&str> = rows.iter().map(|row| row.id).collect();
        assert_eq!(
            ids,
            vec![
                "laravel",
                "symfony",
                "magento",
                "nextcloud",
                "nextjs",
                "strapi",
                "hugo",
                "zola",
                WORDPRESS_ID
            ]
        );
        assert!(rows.iter().all(|row| !row.label.is_empty()));
        assert!(rows.iter().all(|row| !row.category.is_empty()));
        let json = serde_json::to_value(&rows).unwrap();
        assert_eq!(json[0]["id"], "laravel");
        assert_eq!(json[0]["label"], "Laravel");
        assert!(json[0].get("category").is_some());
    }

    #[test]
    fn wordpress_id_routes_to_the_wordpress_module() {
        let setup = by_id(WORDPRESS_ID).unwrap();
        assert_eq!(setup.kind, SetupKind::WordPress);
        assert_eq!(setup.info.id, "wordpress");
        // The dedicated installer's validation is reused rather than copied.
        assert!(crate::wordpress::validate_admin("admin", "s3cret!", "admin@example.com").is_ok());
        assert!(
            plan(setup, None, Path::new("/srv/site"), "site-1", "example.com")
                .unwrap()
                .steps
                .is_empty()
        );
    }

    #[test]
    fn lookups_are_case_insensitive_and_reject_unknown_ids() {
        assert_eq!(by_id(" Laravel ").unwrap().info.id, "laravel");
        assert!(by_id("drupal").is_none());
    }

    #[test]
    fn versions_are_strictly_validated() {
        assert_eq!(validate_version(" 10.4.2 ").unwrap(), "10.4.2");
        assert_eq!(validate_version("29.0.0-rc1").unwrap(), "29.0.0-rc1");
        for bad in [
            "",
            "latest",
            "1.0; rm -rf /",
            "$(id)",
            "../1.0",
            "1.0 `id`",
            "1.0\nid",
            &"9".repeat(65),
        ] {
            assert!(validate_version(bad).is_err(), "accepted version: {bad}");
        }
    }

    #[test]
    fn binaries_and_urls_cannot_hide_shell_fragments() {
        assert_eq!(
            validate_binary("/usr/bin/composer").unwrap(),
            "/usr/bin/composer"
        );
        assert_eq!(validate_binary("npm").unwrap(), "npm");
        for bad in ["sh -c", "npm; id", "$(id)", "cmd\nid", "a'b", "a&b"] {
            assert!(validate_binary(bad).is_err(), "accepted binary: {bad}");
        }
        assert_eq!(
            validate_url("https://download.nextcloud.com/server/releases/latest.tar.bz2").unwrap(),
            "https://download.nextcloud.com/server/releases/latest.tar.bz2"
        );
        assert!(validate_url("http://example.com/a.tar.bz2").is_err());
        assert!(validate_url("https://a;b").is_err());
    }

    #[test]
    fn composer_plans_are_shell_free_and_include_the_version() {
        let setup = by_id("laravel").unwrap();
        let laravel_plan = plan(
            setup,
            Some("11.4.0"),
            Path::new("/var/www/sp_1"),
            "site-1",
            "example.com",
        )
        .unwrap();
        assert_eq!(laravel_plan.steps.len(), 1);
        let step = &laravel_plan.steps[0];
        assert_eq!(step.binary_env, ENV_COMPOSER_BIN);
        assert_eq!(step.binary, "composer");
        assert_eq!(
            step.args,
            vec![
                "create-project",
                "laravel/laravel",
                "/var/www/sp_1",
                "11.4.0",
                "--no-interaction",
                "--prefer-dist"
            ]
        );
        assert!(
            step.args
                .iter()
                .all(|arg| !arg.contains(';') && !arg.contains('$'))
        );

        let magento = plan(
            by_id("magento").unwrap(),
            None,
            Path::new("/var/www/sp_2"),
            "site-2",
            "shop.example.com",
        )
        .unwrap();
        assert!(magento.steps[0].args[1].starts_with("--repository-url="));
        assert!(!magento.steps[0].args.iter().any(|arg| arg == "pub"));
    }

    #[test]
    fn npm_plans_pin_the_package_without_a_shell() {
        let nextjs_plan = plan(
            by_id("nextjs").unwrap(),
            None,
            Path::new("/srv/next"),
            "site-1",
            "app.example.com",
        )
        .unwrap();
        assert_eq!(nextjs_plan.steps[0].binary, "npm");
        assert_eq!(nextjs_plan.steps[0].args[0], "create");
        assert_eq!(nextjs_plan.steps[0].args[1], "next-app@latest");

        let strapi = plan(
            by_id("strapi").unwrap(),
            Some("5.12.1"),
            Path::new("/srv/cms"),
            "site-1",
            "cms.example.com",
        )
        .unwrap();
        assert_eq!(strapi.steps[0].args[1], "strapi-app@5.12.1");
        assert!(strapi.steps[0].args.iter().any(|arg| arg == "--no-run"));
    }

    #[test]
    fn archive_and_generator_plans_are_complete() {
        let nextcloud_plan = plan(
            by_id("nextcloud").unwrap(),
            Some("29.0.2"),
            Path::new("/srv/cloud"),
            "site-1",
            "cloud.example.com",
        )
        .unwrap();
        assert_eq!(nextcloud_plan.steps.len(), 2);
        assert_eq!(nextcloud_plan.steps[0].binary, "curl");
        assert!(
            nextcloud_plan.steps[0]
                .args
                .iter()
                .any(|arg| arg.ends_with("/nextcloud-29.0.2.tar.bz2"))
        );
        assert_eq!(nextcloud_plan.steps[1].binary, "tar");
        assert!(
            nextcloud_plan.steps[1]
                .args
                .iter()
                .any(|arg| arg == "--strip-components=1")
        );
        assert_eq!(nextcloud_plan.cleanup.len(), 1);

        for id in ["hugo", "zola"] {
            let setup = by_id(id).unwrap();
            let plan = plan(setup, None, Path::new("/srv/site"), "site-1", "example.com").unwrap();
            assert_eq!(plan.steps.len(), 2);
            assert!(
                plan.steps[1]
                    .args
                    .iter()
                    .any(|arg| arg.ends_with("/public"))
            );
        }
    }

    #[test]
    fn install_root_and_docroot_are_confined() {
        let base = install_base("/var/www/sp_1/public", "public").unwrap();
        assert_eq!(base, Path::new("/var/www/sp_1"));
        assert_eq!(
            effective_docroot(&base, "public"),
            Path::new("/var/www/sp_1/public")
        );
        let base = install_base("/var/www/sp_2", "public").unwrap();
        assert_eq!(base, Path::new("/var/www/sp_2"));
        assert_eq!(
            effective_docroot(&base, "public"),
            Path::new("/var/www/sp_2/public")
        );
        assert_eq!(
            install_base("/var/www/sp_3", "").unwrap(),
            Path::new("/var/www/sp_3")
        );
        for bad in [
            "/",
            "relative/path",
            "/var/www/../etc",
            "/var/www/sp_4/../../etc",
        ] {
            assert!(install_base(bad, "public").is_err(), "accepted root: {bad}");
        }
        assert!(install_base("/srv/site", "../public").is_err());
    }

    #[test]
    fn local_source_tree_is_copied_without_symlinks() {
        let dir = std::env::temp_dir().join(format!("sp-template-src-{}", Uuid::new_v4()));
        let dest = std::env::temp_dir().join(format!("sp-template-dst-{}", Uuid::new_v4()));
        fs::create_dir_all(dir.join("app")).unwrap();
        fs::write(dir.join("index.php"), "hello").unwrap();
        fs::write(dir.join("app/page.php"), "page").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", dir.join("bad-link")).unwrap();

        assert!(copy_tree(&dir, &dest).is_err());
        fs::remove_file(dir.join("bad-link")).unwrap();
        copy_tree(&dir, &dest).unwrap();
        assert_eq!(fs::read_to_string(dest.join("index.php")).unwrap(), "hello");
        assert_eq!(
            fs::read_to_string(dest.join("app/page.php")).unwrap(),
            "page"
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dest);
    }
}
