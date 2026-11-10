//! Unix-socket IPC server.
//!
//! The core never listens on TCP. Clients (web, CLI) connect to
//! `/run/synthpanel/core.sock`, authenticate with the shared token from
//! `/etc/synthpanel/ipc-token`, and issue typed JSON requests.

use crate::apply::ApplyPipeline;
use crate::audit;
use crate::config::Paths;
use crate::metrics::{self, Snapshot};
use crate::settings::Settings;
use crate::sites::{self, CreateSite, SitePatch};
use crate::users::{self, CreateUser, UserPatch};
use serde_json::{Value, json};
use std::time::Instant;
use subtle::ConstantTimeEq;
use synthpanel_ipc::{Envelope, Error, MAX_LINE, Response, codes, methods};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufStream};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, warn};

pub struct IpcServer {
    paths: Paths,
    settings: Settings,
    users: users::Users,
    sites: sites::Sites,
    apply: ApplyPipeline,
    pool: sqlx::SqlitePool,
    host: String,
    started_at: Instant,
}

impl Clone for IpcServer {
    fn clone(&self) -> Self {
        Self {
            paths: self.paths.clone(),
            settings: self.settings.clone(),
            users: self.users.clone(),
            sites: self.sites.clone(),
            apply: ApplyPipeline::new(),
            pool: self.pool.clone(),
            host: self.host.clone(),
            started_at: self.started_at,
        }
    }
}

impl IpcServer {
    pub fn new(
        paths: Paths,
        pool: sqlx::SqlitePool,
        settings: Settings,
        users: users::Users,
        sites: sites::Sites,
        apply: ApplyPipeline,
        host: String,
    ) -> Self {
        Self {
            paths,
            settings,
            users,
            sites,
            apply,
            pool,
            host,
            started_at: Instant::now(),
        }
    }

    /// Bind the socket and serve connections forever.
    pub async fn serve(&self, listener: UnixListener) -> anyhow::Result<()> {
        loop {
            let (stream, _addr) = listener.accept().await?;
            let server = self.clone();
            tokio::spawn(async move {
                if let Err(error) = server.handle_connection(stream).await {
                    debug!(%error, "ipc connection closed with error");
                }
            });
        }
    }

    async fn handle_connection(&self, stream: UnixStream) -> anyhow::Result<()> {
        let mut stream = BufStream::new(stream);
        loop {
            let mut line = String::new();
            let read = stream.read_line(&mut line).await?;
            if read == 0 {
                return Ok(());
            }
            if read > MAX_LINE {
                warn!("ipc line too long; dropping connection");
                return Ok(());
            }
            let envelope: Envelope = match serde_json::from_str(line.trim_end()) {
                Ok(envelope) => envelope,
                Err(error) => {
                    let response = Response::failure(
                        0,
                        Error::new(codes::BAD_REQUEST, format!("invalid request: {error}")),
                    );
                    write_response(&mut stream, &response).await?;
                    continue;
                }
            };

            if envelope.v != synthpanel_ipc::PROTOCOL_VERSION {
                let response = Response::failure(
                    envelope.id,
                    Error::new(
                        codes::UNSUPPORTED,
                        format!("unsupported protocol version {}", envelope.v),
                    ),
                );
                write_response(&mut stream, &response).await?;
                continue;
            }

            if !self.authorized(&envelope.token).await {
                let response = Response::failure(
                    envelope.id,
                    Error::new(codes::UNAUTHORIZED, "invalid token"),
                );
                write_response(&mut stream, &response).await?;
                continue;
            }

            // Authorization happens inside `dispatch` so that unit tests can
            // exercise the rejection path against a live server.
            let result = self.dispatch(&envelope).await;
            let response = match result {
                Ok(value) => Response::success(envelope.id, value),
                Err(error) => Response::failure(envelope.id, error),
            };
            write_response(&mut stream, &response).await?;
        }
    }

    async fn authorized(&self, token: &str) -> bool {
        let expected = match tokio::fs::read_to_string(&self.paths.ipc_token_path).await {
            Ok(contents) => contents.trim().to_string(),
            Err(_) => return false,
        };
        if expected.is_empty() {
            return false;
        }
        expected.as_bytes().ct_eq(token.as_bytes()).into()
    }

    async fn dispatch(&self, envelope: &Envelope) -> Result<Value, Error> {
        if !self.authorized(&envelope.token).await {
            return Err(Error::new(codes::UNAUTHORIZED, "invalid token"));
        }
        let params = &envelope.params;
        match envelope.method.as_str() {
            methods::HEALTH => Ok(json!({
                "status": "ok",
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_secs": self.started_at.elapsed().as_secs(),
                "host": self.host,
            })),

            methods::SETTINGS_GET => {
                let key = param_str(params, "key")?;
                Ok(self
                    .settings
                    .get(key)
                    .await
                    .map_err(internal)?
                    .unwrap_or(Value::Null))
            }
            methods::SETTINGS_SET => {
                let key = param_str(params, "key")?;
                let value = params.get("value").cloned().unwrap_or(Value::Null);
                self.settings.set(key, &value).await.map_err(internal)?;
                Ok(Value::Null)
            }
            methods::SETTINGS_LIST => {
                let map = self.settings.map().await.map_err(internal)?;
                Ok(serde_json::to_value(map).map_err(internal)?)
            }

            methods::USERS_GET => {
                let id = param_str(params, "id")?;
                let user = self.users.get(id).await.map_err(internal)?;
                Ok(serialize_opt(user)?)
            }
            methods::USERS_GET_BY_EMAIL => {
                let email = param_str(params, "email")?;
                let user = self.users.get_by_email(email).await.map_err(internal)?;
                Ok(serialize_opt(user)?)
            }
            methods::USERS_CREATE => {
                let email = param_str(params, "email")?.to_string();
                let name = param_str(params, "name")?.to_string();
                // Either a pre-computed hash or a plaintext password that the
                // core hashes itself. The core is the single authority for the
                // hashing parameters.
                let password_hash = match params.get("password_hash").and_then(Value::as_str) {
                    Some(hash) => hash.to_string(),
                    None => {
                        let password = param_str(params, "password")?;
                        crate::auth::hash_password(password).map_err(internal)?
                    }
                };
                let role = params
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("admin")
                    .to_string();
                let quota_sites = param_i64(params, "quota_sites", 10)?;
                let quota_dbs = param_i64(params, "quota_dbs", 10)?;
                let user = self
                    .users
                    .create(CreateUser {
                        email,
                        name,
                        password_hash,
                        role,
                        quota_sites,
                        quota_dbs,
                    })
                    .await
                    .map_err(internal)?;
                Ok(serde_json::to_value(user).map_err(internal)?)
            }
            methods::USERS_UPDATE => {
                let id = param_str(params, "id")?.to_string();
                let patch = parse_user_patch(params)?;
                let user = self
                    .users
                    .update(id.as_str(), patch)
                    .await
                    .map_err(internal)?;
                Ok(serialize_opt(user)?)
            }
            methods::USERS_LIST => {
                let users = self.users.list().await.map_err(internal)?;
                Ok(serde_json::to_value(users).map_err(internal)?)
            }

            methods::AUDIT_RECORD => {
                let actor_id = opt_str(params, "actor_id");
                let actor_name = opt_str(params, "actor_name");
                let action = param_str(params, "action")?;
                let target = opt_str(params, "target");
                let metadata = params.get("metadata").cloned().unwrap_or(json!({}));
                let ip = opt_str(params, "ip");
                let id = audit::record(
                    &self.pool, actor_id, actor_name, action, target, &metadata, ip,
                )
                .await
                .map_err(internal)?;
                Ok(json!({ "id": id }))
            }
            methods::AUDIT_LIST => {
                let limit = param_i64(params, "limit", 50)?.clamp(1, 500);
                let entries = audit::list(&self.pool, limit).await.map_err(internal)?;
                Ok(serde_json::to_value(entries).map_err(internal)?)
            }

            methods::METRICS_SNAPSHOT => {
                let snapshot = metrics::latest(&self.pool, &self.host)
                    .await
                    .map_err(internal)?;
                match snapshot {
                    Some(snapshot) => Ok(serde_json::to_value(snapshot).map_err(internal)?),
                    None => Ok(Value::Null),
                }
            }
            methods::METRICS_RANGE => {
                let range = params.get("range").and_then(Value::as_str).unwrap_or("24h");
                let since = metrics::since_from_range(range).ok_or_else(|| {
                    Error::new(codes::BAD_REQUEST, format!("invalid range `{range}`"))
                })?;
                let rows = metrics::range(&self.pool, &self.host, since)
                    .await
                    .map_err(internal)?;
                Ok(serde_json::to_value(rows).map_err(internal)?)
            }

            methods::SERVICES_LIST => Ok(self.services_list().await),
            methods::SERVICES_CONTROL => {
                let unit = param_str(params, "unit")?;
                let action = param_str(params, "action")?;
                self.services_control(unit, action).await?;
                Ok(json!({ "unit": unit, "action": action, "status": "ok" }))
            }

            methods::SITES_LIST => {
                let list = self.sites.list().await.map_err(internal)?;
                Ok(serde_json::to_value(list).map_err(internal)?)
            }
            methods::SITES_GET => {
                let id = param_str(params, "id")?;
                let extras = self.sites.get_with_extras(id).await.map_err(internal)?;
                Ok(serde_json::to_value(extras).map_err(internal)?)
            }
            methods::SITES_CREATE => {
                let domain = param_str(params, "domain")?.to_string();
                let aliases = params
                    .get("aliases")
                    .and_then(|v| {
                        v.as_array().map(|arr| {
                            arr.iter()
                                .filter_map(|item| item.as_str().map(String::from))
                                .collect::<Vec<_>>()
                        })
                    })
                    .unwrap_or_default();
                let document_root = params
                    .get("document_root")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("/var/www/sp_{domain}"));
                let template = params
                    .get("template")
                    .and_then(Value::as_str)
                    .unwrap_or("php")
                    .to_string();
                let php_version = params
                    .get("php_version")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let ssl_mode = params
                    .get("ssl_mode")
                    .and_then(Value::as_str)
                    .unwrap_or("none")
                    .to_string();
                let owner_id = params
                    .get("owner_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let directives = params
                    .get("directives")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_default();
                let input = CreateSite {
                    domain: domain.clone(),
                    aliases: aliases.clone(),
                    document_root,
                    template,
                    php_version,
                    ssl_mode,
                    owner_id,
                    directives,
                };
                let site = self.sites.create(input).await.map_err(internal)?;
                let extras = self
                    .sites
                    .get_with_extras(&site.id)
                    .await
                    .map_err(internal)?
                    .ok_or_else(|| Error::new(codes::INTERNAL, "site vanished after insert"))?;
                let vhost = crate::nginx::render_vhost(&extras, &self.host);
                let pool = crate::php::render_pool(&extras);
                let result = self
                    .apply
                    .apply_site(&extras, &vhost, &pool)
                    .await
                    .map_err(internal)?;
                let _ = audit::record(
                    &self.pool,
                    None,
                    None,
                    "sites.create",
                    Some(site.domain.as_str()),
                    &json!({ "id": site.id, "aliases": aliases }),
                    None,
                )
                .await;
                Ok(json!({ "site": site, "applied": result }))
            }
            methods::SITES_UPDATE => {
                let id = param_str(params, "id")?.to_string();
                let mut patch = SitePatch::default();
                if let Some(value) = params.get("domain") {
                    patch.domain = Some(value.as_str().map(str::to_string).ok_or_else(|| {
                        Error::new(codes::BAD_REQUEST, "domain must be a string")
                    })?);
                }
                if let Some(value) = params.get("aliases") {
                    patch.aliases = Some(
                        value
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|i| i.as_str().map(String::from))
                                    .collect()
                            })
                            .ok_or_else(|| {
                                Error::new(codes::BAD_REQUEST, "aliases must be an array")
                            })?,
                    );
                }
                if let Some(value) = params.get("document_root") {
                    patch.document_root =
                        Some(value.as_str().map(str::to_string).ok_or_else(|| {
                            Error::new(codes::BAD_REQUEST, "document_root must be a string")
                        })?);
                }
                if let Some(value) = params.get("template") {
                    patch.template = Some(value.as_str().map(str::to_string).ok_or_else(|| {
                        Error::new(codes::BAD_REQUEST, "template must be a string")
                    })?);
                }
                if let Some(value) = params.get("ssl_mode") {
                    patch.ssl_mode = Some(value.as_str().map(str::to_string).ok_or_else(|| {
                        Error::new(codes::BAD_REQUEST, "ssl_mode must be a string")
                    })?);
                }
                if let Some(value) = params.get("status") {
                    patch.status = Some(value.as_str().map(str::to_string).ok_or_else(|| {
                        Error::new(codes::BAD_REQUEST, "status must be a string")
                    })?);
                }
                if let Some(value) = params.get("php_version") {
                    if value.is_null() {
                        patch.php_version = Some(None);
                    } else {
                        patch.php_version =
                            Some(Some(value.as_str().map(str::to_string).ok_or_else(
                                || Error::new(codes::BAD_REQUEST, "php_version must be a string"),
                            )?));
                    }
                }
                if let Some(value) = params.get("directives") {
                    patch.directives =
                        Some(value.as_str().map(str::to_string).ok_or_else(|| {
                            Error::new(codes::BAD_REQUEST, "directives must be a string")
                        })?);
                }
                if let Some(value) = params.get("overrides_json") {
                    patch.overrides_json =
                        Some(value.as_str().map(str::to_string).ok_or_else(|| {
                            Error::new(codes::BAD_REQUEST, "overrides_json must be a string")
                        })?);
                }
                let updated = self
                    .sites
                    .update(id.as_str(), patch)
                    .await
                    .map_err(internal)?;
                match updated {
                    Some(site) => {
                        let extras = self
                            .sites
                            .get_with_extras(&site.id)
                            .await
                            .map_err(internal)?
                            .ok_or_else(|| Error::new(codes::NOT_FOUND, "site not found"))?;
                        let vhost = crate::nginx::render_vhost(&extras, &self.host);
                        let pool = crate::php::render_pool(&extras);
                        let result = self
                            .apply
                            .apply_site(&extras, &vhost, &pool)
                            .await
                            .map_err(internal)?;
                        let _ = audit::record(
                            &self.pool,
                            None,
                            None,
                            "sites.update",
                            Some(site.domain.as_str()),
                            &json!({ "id": site.id }),
                            None,
                        )
                        .await;
                        Ok(json!({ "site": site, "applied": result }))
                    }
                    None => Err(Error::new(codes::NOT_FOUND, "site not found")),
                }
            }
            methods::SITES_SSL_ISSUE => {
                let id = param_str(params, "id")?.to_string();
                let site = self.sites.get(&id).await.map_err(internal)?;
                let Some(site) = site else {
                    return Err(Error::new(codes::NOT_FOUND, "site not found"));
                };
                let domain = site.domain.clone();
                let email = opt_str(params, "email").map(str::to_string);
                let acme = crate::ssl::AcmePaths::from_env();
                let webroot = self.paths.www_dir.join(&id);
                let result = crate::ssl::issue_http01(&acme, &domain, &webroot, email.as_deref())
                    .await
                    .map_err(internal)?;
                if result.issued {
                    // Switch the site to Let's Encrypt and re-apply the vhost.
                    let _ = self
                        .sites
                        .update(
                            &id,
                            SitePatch {
                                ssl_mode: Some("le".into()),
                                ..Default::default()
                            },
                        )
                        .await;
                    if let Some(extras) = self.sites.get_with_extras(&id).await.map_err(internal)? {
                        let vhost = crate::nginx::render_vhost(&extras, &self.host);
                        let pool = crate::php::render_pool(&extras);
                        let applied = self
                            .apply
                            .apply_site(&extras, &vhost, &pool)
                            .await
                            .map_err(internal)?;
                        let _ = audit::record(
                            &self.pool,
                            None,
                            None,
                            "sites.ssl.issue",
                            Some(domain.as_str()),
                            &json!({ "id": id, "expires_at": result.expires_at }),
                            None,
                        )
                        .await;
                        return Ok(json!({ "result": result, "applied": applied }));
                    }
                }
                let _ = audit::record(
                    &self.pool,
                    None,
                    None,
                    "sites.ssl.issue",
                    Some(domain.as_str()),
                    &json!({ "id": id, "issued": false }),
                    None,
                )
                .await;
                Ok(json!({ "result": result }))
            }
            methods::SITES_SSL_STATUS => {
                let id = param_str(params, "id")?.to_string();
                let acme = crate::ssl::AcmePaths::from_env();
                let site = self.sites.get(&id).await.map_err(internal)?;
                let Some(site) = site else {
                    return Err(Error::new(codes::NOT_FOUND, "site not found"));
                };
                let domain_paths = acme.domain_paths(&site.domain);
                let (live_fullchain, _) = domain_paths.live_paths();
                let expires_at = crate::ssl::read_expiry(&live_fullchain);
                Ok(json!({
                    "domain": site.domain,
                    "ssl_mode": site.ssl_mode,
                    "expires_at": expires_at,
                    "expiring_soon": crate::ssl::expires_within(&live_fullchain, 30),
                }))
            }
            methods::PHP_VERSIONS => {
                let versions: Vec<&str> = crate::php::PHP_VERSIONS.to_vec();
                Ok(serde_json::to_value(versions).map_err(internal)?)
            }
            methods::SITES_DELETE => {
                let id = param_str(params, "id")?.to_string();
                let removed = self.sites.delete(&id).await.map_err(internal)?;
                let fs = self.apply.remove_site(&id).await.map_err(internal)?;
                let _ = audit::record(
                    &self.pool,
                    None,
                    None,
                    "sites.delete",
                    Some(id.as_str()),
                    &json!({ "removed": removed }),
                    None,
                )
                .await;
                Ok(json!({ "removed": removed, "fs": fs }))
            }

            other => Err(Error::new(
                codes::UNSUPPORTED,
                format!("unknown method `{other}`"),
            )),
        }
    }

    async fn services_list(&self) -> Value {
        let units = [
            "nginx",
            "php8.3-fpm",
            "mariadb",
            "postgresql",
            "redis-server",
            "synthpanel-core",
            "synthpanel-web",
        ];
        let mut out = Vec::new();
        for unit in units {
            let status = systemctl_is_active(unit).await;
            out.push(json!({ "unit": unit, "status": status }));
        }
        Value::Array(out)
    }

    async fn services_control(&self, unit: &str, action: &str) -> Result<(), Error> {
        if !is_allowed_unit(unit) {
            return Err(Error::new(
                codes::BAD_REQUEST,
                format!("unit `{unit}` is not managed"),
            ));
        }
        if !matches!(action, "start" | "stop" | "restart" | "reload") {
            return Err(Error::new(
                codes::BAD_REQUEST,
                format!("invalid action `{action}`"),
            ));
        }
        let output = tokio::process::Command::new("systemctl")
            .arg(action)
            .arg(unit)
            .output()
            .await
            .map_err(internal)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::new(
                codes::INTERNAL,
                format!("systemctl {action} {unit} failed: {stderr}"),
            ));
        }
        Ok(())
    }
}

fn is_allowed_unit(unit: &str) -> bool {
    matches!(
        unit,
        "nginx"
            | "php8.1-fpm"
            | "php8.2-fpm"
            | "php8.3-fpm"
            | "php8.4-fpm"
            | "mariadb"
            | "mysql"
            | "postgresql"
            | "redis-server"
            | "synthpanel-core"
            | "synthpanel-web"
    )
}

async fn systemctl_is_active(unit: &str) -> String {
    match tokio::process::Command::new("systemctl")
        .arg("is-active")
        .arg(unit)
        .output()
        .await
    {
        Ok(output) => {
            let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if status.is_empty() {
                "unknown".to_string()
            } else {
                status
            }
        }
        Err(_) => "unknown".to_string(),
    }
}

async fn write_response(
    stream: &mut BufStream<UnixStream>,
    response: &Response,
) -> anyhow::Result<()> {
    let mut bytes = serde_json::to_vec(response)?;
    bytes.push(b'\n');
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

fn internal(error: impl std::fmt::Display) -> Error {
    Error::new(codes::INTERNAL, error.to_string())
}

fn param_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, Error> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new(codes::BAD_REQUEST, format!("missing string field `{key}`")))
}

fn opt_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

fn param_i64(params: &Value, key: &str, default: i64) -> Result<i64, Error> {
    match params.get(key) {
        Some(Value::Null) | None => Ok(default),
        Some(value) => value.as_i64().ok_or_else(|| {
            Error::new(
                codes::BAD_REQUEST,
                format!("field `{key}` must be an integer"),
            )
        }),
    }
}

fn serialize_opt<T: serde::Serialize>(value: Option<T>) -> Result<Value, Error> {
    match value {
        Some(value) => Ok(serde_json::to_value(value).map_err(internal)?),
        None => Ok(Value::Null),
    }
}

fn parse_user_patch(params: &Value) -> Result<UserPatch, Error> {
    let mut patch = UserPatch::default();
    if let Some(value) = params.get("email") {
        patch.email = Some(
            value
                .as_str()
                .ok_or_else(|| Error::new(codes::BAD_REQUEST, "email must be a string"))?
                .to_string(),
        );
    }
    if let Some(value) = params.get("name") {
        patch.name = Some(
            value
                .as_str()
                .ok_or_else(|| Error::new(codes::BAD_REQUEST, "name must be a string"))?
                .to_string(),
        );
    }
    if let Some(value) = params.get("password_hash") {
        patch.password_hash = Some(
            value
                .as_str()
                .ok_or_else(|| Error::new(codes::BAD_REQUEST, "password_hash must be a string"))?
                .to_string(),
        );
    }
    if let Some(value) = params.get("role") {
        patch.role = Some(
            value
                .as_str()
                .ok_or_else(|| Error::new(codes::BAD_REQUEST, "role must be a string"))?
                .to_string(),
        );
    }
    if let Some(value) = params.get("status") {
        patch.status = Some(
            value
                .as_str()
                .ok_or_else(|| Error::new(codes::BAD_REQUEST, "status must be a string"))?
                .to_string(),
        );
    }
    if let Some(value) = params.get("quota_sites") {
        if !value.is_null() {
            patch.quota_sites =
                Some(value.as_i64().ok_or_else(|| {
                    Error::new(codes::BAD_REQUEST, "quota_sites must be an integer")
                })?);
        }
    }
    if let Some(value) = params.get("quota_dbs") {
        if !value.is_null() {
            patch.quota_dbs =
                Some(value.as_i64().ok_or_else(|| {
                    Error::new(codes::BAD_REQUEST, "quota_dbs must be an integer")
                })?);
        }
    }
    if let Some(value) = params.get("totp_secret") {
        if value.is_null() {
            patch.totp_secret = Some(None);
        } else {
            patch.totp_secret = Some(Some(
                value
                    .as_str()
                    .ok_or_else(|| Error::new(codes::BAD_REQUEST, "totp_secret must be a string"))?
                    .to_string(),
            ));
        }
    }
    if let Some(value) = params.get("totp_enabled") {
        if !value.is_null() {
            patch.totp_enabled =
                Some(value.as_bool().ok_or_else(|| {
                    Error::new(codes::BAD_REQUEST, "totp_enabled must be a boolean")
                })?);
        }
    }
    if let Some(value) = params.get("last_login_at") {
        if value.is_null() {
            patch.last_login_at = Some(None);
        } else {
            patch.last_login_at = Some(Some(
                value
                    .as_str()
                    .ok_or_else(|| {
                        Error::new(codes::BAD_REQUEST, "last_login_at must be a string")
                    })?
                    .to_string(),
            ));
        }
    }
    Ok(patch)
}

/// Collect a one-off snapshot. Exposed for tests and for callers that need a
/// value before the collector has produced its first sample.
pub async fn sample_now(pool: &sqlx::SqlitePool, host: &str) -> anyhow::Result<Snapshot> {
    let mut collector = metrics::Collector::new(pool.clone());
    let mut snapshot = collector.sample();
    snapshot.host = host.to_string();
    metrics::store(pool, &snapshot).await?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use crate::db;
    use serde_json::json;
    use std::time::Duration;

    async fn test_server() -> (IpcServer, Paths) {
        let mut base = std::env::temp_dir();
        base.push(format!("sp-ipc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let paths = Paths {
            etc_dir: base.join("etc"),
            lib_dir: base.join("lib"),
            log_dir: base.join("log"),
            run_dir: base.join("run"),
            www_dir: base.join("www"),
            backup_dir: base.join("backups"),
            db_path: base.join("lib/synthpanel.db"),
            socket_path: base.join("run/core.sock"),
            ipc_token_path: base.join("etc/ipc-token"),
            tls_dir: base.join("etc/tls"),
            secret_key_path: base.join("etc/secret.key"),
        };
        paths.ensure_dirs().unwrap();
        std::fs::write(&paths.ipc_token_path, "test-token").unwrap();

        let pool = db::connect(&paths.db_path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        let settings = Settings::new(pool.clone());
        let users = users::Users::new(pool.clone());
        let sites = sites::Sites::new(pool.clone());
        let apply = crate::apply::ApplyPipeline::new();
        let server = IpcServer::new(
            paths.clone(),
            pool,
            settings,
            users,
            sites,
            apply,
            "testhost".into(),
        );
        (server, paths)
    }

    #[tokio::test]
    async fn health_and_settings_roundtrip() {
        let (server, paths) = test_server().await;
        let health = server
            .dispatch(&Envelope::new(1, "test-token", methods::HEALTH, json!({})))
            .await
            .unwrap();
        assert_eq!(health["status"], "ok");

        // Rotate the token on disk; the server must read the fresh value.
        std::fs::write(&paths.ipc_token_path, "the-correct-token").unwrap();
        let bad = server
            .dispatch(&Envelope::new(2, "test-token", methods::HEALTH, json!({})))
            .await;
        assert!(bad.is_err(), "stale token must be rejected");
        std::fs::write(&paths.ipc_token_path, "test-token").unwrap();

        server
            .dispatch(&Envelope::new(
                3,
                "test-token",
                methods::SETTINGS_SET,
                json!({"key": "hostname", "value": "panel.example.com"}),
            ))
            .await
            .unwrap();
        let value = server
            .dispatch(&Envelope::new(
                4,
                "test-token",
                methods::SETTINGS_GET,
                json!({"key": "hostname"}),
            ))
            .await
            .unwrap();
        assert_eq!(value, json!("panel.example.com"));
    }

    #[tokio::test]
    async fn user_crud_and_metrics() {
        let (server, _paths) = test_server().await;
        let created = server
            .dispatch(&Envelope::new(
                1,
                "test-token",
                methods::USERS_CREATE,
                json!({"email":"a@example.com","name":"A","password_hash":"hash","role":"admin"}),
            ))
            .await
            .unwrap();
        let id = created["id"].as_str().unwrap().to_string();

        let fetched = server
            .dispatch(&Envelope::new(
                2,
                "test-token",
                methods::USERS_GET_BY_EMAIL,
                json!({"email":"a@example.com"}),
            ))
            .await
            .unwrap();
        assert_eq!(fetched["id"], created["id"]);

        let updated = server
            .dispatch(&Envelope::new(
                3,
                "test-token",
                methods::USERS_UPDATE,
                json!({"id": id, "role": "operator"}),
            ))
            .await
            .unwrap();
        assert_eq!(updated["role"], "operator");

        let snapshot = sample_now(&server.pool, "testhost").await.unwrap();
        let range = server
            .dispatch(&Envelope::new(
                4,
                "test-token",
                methods::METRICS_RANGE,
                json!({"range":"1h"}),
            ))
            .await
            .unwrap();
        assert_eq!(range.as_array().unwrap().len(), 1);
        assert!(snapshot.mem_used >= 0);
    }

    #[tokio::test]
    async fn unknown_method_is_rejected() {
        let (server, _paths) = test_server().await;
        let result = server
            .dispatch(&Envelope::new(1, "test-token", "bogus.method", json!({})))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn audit_entries_are_recorded() {
        let (server, _paths) = test_server().await;
        server
            .dispatch(&Envelope::new(
                1,
                "test-token",
                methods::AUDIT_RECORD,
                json!({"action":"test.action","target":"x","metadata":{"k":1}}),
            ))
            .await
            .unwrap();
        let entries = server
            .dispatch(&Envelope::new(
                2,
                "test-token",
                methods::AUDIT_LIST,
                json!({"limit":10}),
            ))
            .await
            .unwrap();
        assert_eq!(entries.as_array().unwrap().len(), 1);
        assert_eq!(entries[0]["action"], "test.action");
    }

    #[tokio::test]
    async fn server_survives_a_bad_line() {
        let (server, paths) = test_server().await;
        let listener = UnixListener::bind(&paths.socket_path).unwrap();
        let handle = tokio::spawn(async move { server.serve(listener).await });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut client = synthpanel_ipc::Client::connect(&paths.socket_path, "test-token")
            .await
            .unwrap();
        let health: Value = client.call(methods::HEALTH, json!({})).await.unwrap();
        assert_eq!(health["status"], "ok");
        handle.abort();
    }
}
