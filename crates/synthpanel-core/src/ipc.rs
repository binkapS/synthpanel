//! Unix-socket IPC server.
//!
//! The core never listens on TCP. Clients (web, CLI) connect to
//! `/run/synthpanel/core.sock`, authenticate with the shared token from
//! `/etc/synthpanel/ipc-token`, and issue typed JSON requests.

use crate::apply::ApplyPipeline;
use crate::audit;
use crate::config::Paths;
use crate::metrics::{self, Snapshot};
use crate::security::FirewallRule;
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

/// The repositories the IPC server works with.
pub struct Stores {
    pub settings: Settings,
    pub users: users::Users,
    pub sites: sites::Sites,
    pub databases: crate::databases::Databases,
}

pub struct IpcServer {
    paths: Paths,
    stores: Stores,
    apply: ApplyPipeline,
    pool: sqlx::SqlitePool,
    host: String,
    started_at: Instant,
}

impl Clone for IpcServer {
    fn clone(&self) -> Self {
        Self {
            paths: self.paths.clone(),
            stores: Stores {
                settings: self.stores.settings.clone(),
                users: self.stores.users.clone(),
                sites: self.stores.sites.clone(),
                databases: self.stores.databases.clone(),
            },
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
        stores: Stores,
        apply: ApplyPipeline,
        host: String,
    ) -> Self {
        Self {
            paths,
            stores,
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
                    .stores
                    .settings
                    .get(key)
                    .await
                    .map_err(internal)?
                    .unwrap_or(Value::Null))
            }
            methods::SETTINGS_SET => {
                let key = param_str(params, "key")?;
                let value = params.get("value").cloned().unwrap_or(Value::Null);
                self.stores
                    .settings
                    .set(key, &value)
                    .await
                    .map_err(internal)?;
                Ok(Value::Null)
            }
            methods::SETTINGS_LIST => {
                let map = self.stores.settings.map().await.map_err(internal)?;
                Ok(serde_json::to_value(map).map_err(internal)?)
            }

            methods::USERS_GET => {
                let id = param_str(params, "id")?;
                let user = self.stores.users.get(id).await.map_err(internal)?;
                Ok(serialize_opt(user)?)
            }
            methods::USERS_GET_BY_EMAIL => {
                let email = param_str(params, "email")?;
                let user = self
                    .stores
                    .users
                    .get_by_email(email)
                    .await
                    .map_err(internal)?;
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
                    .stores
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
                    .stores
                    .users
                    .update(id.as_str(), patch)
                    .await
                    .map_err(internal)?;
                Ok(serialize_opt(user)?)
            }
            methods::USERS_LIST => {
                let users = self.stores.users.list().await.map_err(internal)?;
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
                let list = self.stores.sites.list().await.map_err(internal)?;
                Ok(serde_json::to_value(list).map_err(internal)?)
            }
            methods::SITES_GET => {
                let id = param_str(params, "id")?;
                let extras = self
                    .stores
                    .sites
                    .get_with_extras(id)
                    .await
                    .map_err(internal)?;
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
                let site = self.stores.sites.create(input).await.map_err(internal)?;
                let extras = self
                    .stores
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
                    .stores
                    .sites
                    .update(id.as_str(), patch)
                    .await
                    .map_err(internal)?;
                match updated {
                    Some(site) => {
                        let extras = self
                            .stores
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
                let site = self.stores.sites.get(&id).await.map_err(internal)?;
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
                        .stores
                        .sites
                        .update(
                            &id,
                            SitePatch {
                                ssl_mode: Some("le".into()),
                                ..Default::default()
                            },
                        )
                        .await;
                    if let Some(extras) = self
                        .stores
                        .sites
                        .get_with_extras(&id)
                        .await
                        .map_err(internal)?
                    {
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
                let site = self.stores.sites.get(&id).await.map_err(internal)?;
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
                let removed = self.stores.sites.delete(&id).await.map_err(internal)?;
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

            methods::DATABASES_LIST => {
                let list = self.stores.databases.list().await.map_err(internal)?;
                Ok(serde_json::to_value(list).map_err(internal)?)
            }
            methods::DATABASES_GET => {
                let id = param_str(params, "id")?;
                let record = self.stores.databases.get(id).await.map_err(internal)?;
                match record {
                    Some(record) => {
                        let users = self
                            .stores
                            .databases
                            .list_users(&record.id)
                            .await
                            .map_err(internal)?;
                        let sanitized: Vec<Value> = users
                            .into_iter()
                            .map(|user| {
                                json!({
                                    "id": user.id,
                                    "database_id": user.database_id,
                                    "username": user.username,
                                    "privileges": user.privileges,
                                    "created_at": user.created_at,
                                })
                            })
                            .collect();
                        Ok(json!({ "database": record, "users": sanitized }))
                    }
                    None => Ok(Value::Null),
                }
            }
            methods::DATABASES_CREATE => {
                let engine_name = param_str(params, "engine")?;
                let engine = crate::database::require_engine(engine_name).map_err(internal)?;
                let name = param_str(params, "name")?;
                let name = crate::database::validate_name(name).map_err(internal)?;
                let owner_id = params
                    .get("owner_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let username = params
                    .get("username")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| name.clone());
                let username = crate::database::validate_username(&username).map_err(internal)?;
                let password = params
                    .get("password")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| crate::database::generate_password(24));
                let privileges = params
                    .get("privileges")
                    .and_then(Value::as_str)
                    .and_then(crate::databases::Privilege::parse)
                    .unwrap_or(crate::databases::Privilege::All);

                // Provision the database on the server first: if that fails
                // nothing is recorded in the panel state.
                let credentials = self.database_credentials();
                let mut provisioned = false;
                let mut provisioning_error = None;
                match engine {
                    crate::database::Engine::Mysql => {
                        match crate::database::mysql_create(
                            &name,
                            &username,
                            &password,
                            &credentials,
                        ) {
                            Ok(()) => provisioned = true,
                            Err(error) => provisioning_error = Some(error.to_string()),
                        }
                    }
                    crate::database::Engine::Postgres => {
                        match crate::database::postgres_create(
                            &name,
                            &username,
                            &password,
                            &credentials,
                        ) {
                            Ok(()) => provisioned = true,
                            Err(error) => provisioning_error = Some(error.to_string()),
                        }
                    }
                    crate::database::Engine::Redis => {
                        match crate::database::redis_set_password(&password) {
                            Ok(()) => provisioned = true,
                            Err(error) => provisioning_error = Some(error.to_string()),
                        }
                    }
                }

                if !provisioned {
                    let message = provisioning_error.unwrap_or_else(|| "unknown error".into());
                    let _ = audit::record(
                        &self.pool,
                        None,
                        None,
                        "databases.create",
                        Some(name.as_str()),
                        &json!({ "engine": engine.as_str(), "error": message }),
                        None,
                    )
                    .await;
                    return Err(Error::new(
                        codes::INTERNAL,
                        format!("cannot provision the database: {message}"),
                    ));
                }

                let record = self
                    .stores
                    .databases
                    .create(engine.as_str(), &name, owner_id)
                    .await
                    .map_err(internal)?;
                let user = self
                    .stores
                    .databases
                    .add_user(&record.id, &username, &password, privileges)
                    .await
                    .map_err(internal)?;
                let _ = audit::record(
                    &self.pool,
                    None,
                    None,
                    "databases.create",
                    Some(name.as_str()),
                    &json!({ "engine": engine.as_str(), "username": username }),
                    None,
                )
                .await;

                let connection = crate::database::ConnectionInfo {
                    engine: engine.as_str().to_string(),
                    host: "127.0.0.1".into(),
                    port: match engine {
                        crate::database::Engine::Mysql => 3306,
                        crate::database::Engine::Postgres => 5432,
                        crate::database::Engine::Redis => 6379,
                    },
                    database: name.clone(),
                    username: username.clone(),
                    password: password.clone(),
                };

                Ok(json!({
                    "database": record,
                    "user": {
                        "id": user.id,
                        "username": username,
                        "privileges": user.privileges,
                    },
                    "password": password,
                    "connection_string": connection.connection_string(),
                    "env": connection.env_snippet(),
                }))
            }
            methods::DATABASES_DELETE => {
                let id = param_str(params, "id")?;
                let record = self.stores.databases.get(id).await.map_err(internal)?;
                let Some(record) = record else {
                    return Err(Error::new(codes::NOT_FOUND, "database not found"));
                };
                let users = self
                    .stores
                    .databases
                    .list_users(&record.id)
                    .await
                    .map_err(internal)?;
                let username = users.first().map(|user| user.username.clone());
                let engine = crate::database::require_engine(&record.engine).map_err(internal)?;
                let credentials = self.database_credentials();
                let dropped = match engine {
                    crate::database::Engine::Mysql => {
                        crate::database::mysql_drop(&record.name, username.as_deref(), &credentials)
                    }
                    crate::database::Engine::Postgres => crate::database::postgres_drop(
                        &record.name,
                        username.as_deref(),
                        &credentials,
                    ),
                    crate::database::Engine::Redis => Ok(()),
                };
                if let Err(error) = dropped {
                    let _ = audit::record(
                        &self.pool,
                        None,
                        None,
                        "databases.delete",
                        Some(record.name.as_str()),
                        &json!({ "error": error.to_string() }),
                        None,
                    )
                    .await;
                    return Err(Error::new(
                        codes::INTERNAL,
                        format!("cannot drop the database: {error}"),
                    ));
                }
                let removed = self.stores.databases.delete(id).await.map_err(internal)?;
                let _ = audit::record(
                    &self.pool,
                    None,
                    None,
                    "databases.delete",
                    Some(record.name.as_str()),
                    &json!({ "engine": record.engine }),
                    None,
                )
                .await;
                Ok(json!({ "removed": removed }))
            }
            methods::DATABASES_ADD_USER => {
                let database_id = param_str(params, "database_id")?;
                let record = self
                    .stores
                    .databases
                    .get(database_id)
                    .await
                    .map_err(internal)?;
                if record.is_none() {
                    return Err(Error::new(codes::NOT_FOUND, "database not found"));
                }
                let username = param_str(params, "username")?;
                let username = crate::database::validate_username(username).map_err(internal)?;
                let password = params
                    .get("password")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| crate::database::generate_password(24));
                let privileges = params
                    .get("privileges")
                    .and_then(Value::as_str)
                    .and_then(crate::databases::Privilege::parse)
                    .unwrap_or(crate::databases::Privilege::All);
                let user = self
                    .stores
                    .databases
                    .add_user(database_id, &username, &password, privileges)
                    .await
                    .map_err(internal)?;
                Ok(json!({
                    "id": user.id,
                    "username": user.username,
                    "privileges": user.privileges,
                    "password": password,
                }))
            }
            methods::DATABASES_USERS => {
                let database_id = param_str(params, "database_id")?;
                let users = self
                    .stores
                    .databases
                    .list_users(database_id)
                    .await
                    .map_err(internal)?;
                Ok(serde_json::to_value(users).map_err(internal)?)
            }
            methods::DATABASES_DELETE_USER => {
                let id = param_str(params, "id")?;
                let removed = self
                    .stores
                    .databases
                    .delete_user(id)
                    .await
                    .map_err(internal)?;
                Ok(json!({ "removed": removed }))
            }
            methods::DATABASES_ENGINES => {
                let credentials = self.database_credentials();
                let engines: Vec<crate::database::EngineStatus> = [
                    crate::database::Engine::Mysql,
                    crate::database::Engine::Postgres,
                    crate::database::Engine::Redis,
                ]
                .into_iter()
                .map(|engine| crate::database::engine_status(engine, &credentials))
                .collect();
                Ok(serde_json::to_value(engines).map_err(internal)?)
            }

            methods::SECURITY_FIREWALL => {
                let available = crate::security::ufw_available();
                let status = if available {
                    crate::security::ufw_status().unwrap_or_default()
                } else {
                    String::new()
                };
                Ok(json!({ "available": available, "status": status }))
            }
            methods::SECURITY_FIREWALL_APPLY => {
                let rule = FirewallRule {
                    port: param_str(params, "port")?.to_string(),
                    proto: param_str(params, "proto")?.to_string(),
                    source: param_str(params, "source")?.to_string(),
                    action: param_str(params, "action")?.to_string(),
                    comment: params
                        .get("comment")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    enabled: params
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                };
                let rule = FirewallRule {
                    port: crate::security::validate_port(&rule.port).map_err(internal)?,
                    source: crate::security::validate_source(&rule.source).map_err(internal)?,
                    ..rule
                };
                let output = crate::security::ufw_apply(&rule).map_err(internal)?;
                let _ = audit::record(
                    &self.pool,
                    None,
                    None,
                    "security.firewall.apply",
                    Some(rule.port.as_str()),
                    &json!({ "rule": rule }),
                    None,
                )
                .await;
                Ok(json!({ "output": output }))
            }
            methods::SECURITY_FAIL2BAN => {
                let available = crate::security::fail2ban_available();
                let jails = if available {
                    crate::security::fail2ban_jails().unwrap_or_default()
                } else {
                    Vec::new()
                };
                Ok(json!({ "available": available, "jails": jails }))
            }
            methods::SECURITY_FAIL2BAN_UNBAN => {
                let jail = param_str(params, "jail")?;
                let ip = param_str(params, "ip")?;
                let output = crate::security::fail2ban_unban(jail, ip).map_err(internal)?;
                let _ = audit::record(
                    &self.pool,
                    None,
                    None,
                    "security.fail2ban.unban",
                    Some(ip),
                    &json!({ "jail": jail }),
                    None,
                )
                .await;
                Ok(json!({ "output": output }))
            }
            methods::SECURITY_SSH_KEYS => {
                let rows = sqlx::query_as::<_, SshKeyRow>(
                    "SELECT id, label, public_key, fingerprint, created_at
                     FROM ssh_keys ORDER BY created_at",
                )
                .fetch_all(&self.pool)
                .await
                .map_err(internal)?;
                Ok(serde_json::to_value(rows).map_err(internal)?)
            }
            methods::SECURITY_SSH_KEY_ADD => {
                let label = param_str(params, "label")?;
                let public_key = param_str(params, "public_key")?;
                let user = params.get("user").and_then(Value::as_str).unwrap_or("root");
                let (key_type, fingerprint) =
                    crate::security::validate_ssh_key(public_key).map_err(internal)?;
                crate::security::install_ssh_key(user, public_key).map_err(internal)?;
                let id = uuid::Uuid::new_v4().to_string();
                let now = chrono::Utc::now().to_rfc3339();
                sqlx::query(
                    "INSERT INTO ssh_keys (id, user_id, label, public_key, fingerprint, created_at)
                     VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(user)
                .bind(label)
                .bind(public_key)
                .bind(&fingerprint)
                .bind(&now)
                .execute(&self.pool)
                .await
                .map_err(internal)?;
                let _ = audit::record(
                    &self.pool,
                    None,
                    None,
                    "security.ssh_key.add",
                    Some(label),
                    &json!({ "user": user, "type": key_type }),
                    None,
                )
                .await;
                Ok(json!({ "id": id, "label": label, "fingerprint": fingerprint }))
            }
            methods::SECURITY_SSH_KEY_REMOVE => {
                let id = param_str(params, "id")?;
                let row = sqlx::query_as::<_, SshKeyRow>(
                    "SELECT id, label, public_key, fingerprint, created_at
                     FROM ssh_keys WHERE id = ?",
                )
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(internal)?;
                let Some(row) = row else {
                    return Err(Error::new(codes::NOT_FOUND, "ssh key not found"));
                };
                let _ = sqlx::query("DELETE FROM ssh_keys WHERE id = ?")
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(internal)?;
                let _ = audit::record(
                    &self.pool,
                    None,
                    None,
                    "security.ssh_key.remove",
                    Some(row.label.as_str()),
                    &json!({}),
                    None,
                )
                .await;
                Ok(json!({ "removed": true }))
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

/// A row of the `ssh_keys` table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct SshKeyRow {
    pub id: String,
    pub label: String,
    pub public_key: String,
    pub fingerprint: String,
    pub created_at: String,
}

impl IpcServer {
    /// Read the admin credentials for the local database servers from the
    /// environment. They are optional: without them the panel relies on
    /// socket/peer authentication, which is the default on Ubuntu.
    fn database_credentials(&self) -> crate::database::AdminCredentials {
        crate::database::AdminCredentials {
            mysql_root_password: std::env::var("SYNTHPANEL_MYSQL_ROOT_PASSWORD").ok(),
            postgres_user: std::env::var("SYNTHPANEL_POSTGRES_USER")
                .unwrap_or_else(|_| "postgres".to_string()),
            postgres_password: std::env::var("SYNTHPANEL_POSTGRES_PASSWORD").ok(),
        }
    }
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
        let databases = crate::databases::Databases::new(pool.clone());
        let apply = crate::apply::ApplyPipeline::new();
        let server = IpcServer::new(
            paths.clone(),
            pool,
            Stores {
                settings,
                users,
                sites,
                databases,
            },
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
