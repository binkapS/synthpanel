//! External / remote database server definitions.
//!
//! An operator can register a database that is not managed by the
//! panel itself — an RDS or Aurora instance, a managed PostgreSQL server, or
//! any other remote MySQL/Postgres endpoint. This module owns those connection
//! definitions. It deliberately does **not** open connections: it only stores
//! and validates the configuration and audits nothing (the IPC/web layers do).
//!
//! # Security
//!
//! Passwords are encrypted at rest with ChaCha20-Poly1305 before they touch
//! SQLite. The key is the persistent `secret.key` that `synthpanel-core`
//! creates at startup (a 32-byte hex string, mode 0600); if the file is ever
//! missing this module creates it with the same permissions instead of
//! falling back to plaintext storage. Each record id is used as AEAD
//! associated data, so a ciphertext cannot be moved between records. Nothing
//! in this module logs the key or the password, and the input type's `Debug`
//! implementation redacts the password.
//!
//! The cleartext password is only available during `add()`, in memory, long
//! enough to encrypt it. `list()` never selects the encrypted column.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use std::fmt;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Default port for MySQL-compatible engines.
pub const DEFAULT_MYSQL_PORT: u16 = 3306;
/// Default port for PostgreSQL-compatible engines.
pub const DEFAULT_POSTGRES_PORT: u16 = 5432;

/// A remote server as returned to callers. No credential is ever included.
#[derive(Clone, Serialize)]
pub struct RemoteDatabaseRecord {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub host: String,
    pub port: u16,
    #[serde(rename = "database")]
    pub database: String,
    pub username: String,
    pub tls: bool,
}

/// Input for `DATABASES_REMOTE_ADD`.
///
/// `port` may be omitted, in which case it is derived from `engine`, and
/// `tls` defaults to `false`. `Debug` redacts the password on purpose.
#[derive(Deserialize)]
pub struct AddRemoteDatabase {
    pub name: String,
    pub engine: String,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub database: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub tls: bool,
}

impl fmt::Debug for AddRemoteDatabase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AddRemoteDatabase")
            .field("name", &self.name)
            .field("engine", &self.engine)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("tls", &self.tls)
            .finish()
    }
}

impl AddRemoteDatabase {
    /// Parse an IPC parameter object. This is the convenient entry point for
    /// the core dispatcher: it accepts the exact `params` value of an
    /// envelope and performs no I/O.
    pub fn from_params(params: &Value) -> Result<Self> {
        serde_json::from_value::<AddRemoteDatabase>(params.clone()).context(
            "invalid remote database parameters (name, engine, host, port, database, username, password and tls are expected)",
        )
    }

    /// Validate all fields, normalise the engine name and resolve the default
    /// port. The returned value is the only form `add()` accepts.
    pub fn validate(&self) -> Result<ValidatedRemoteDatabase> {
        let name = self.name.trim();
        if name.is_empty() || name.len() > 128 {
            bail!("name must be between 1 and 128 characters");
        }
        if name.chars().any(char::is_control) {
            bail!("name must not contain control characters");
        }

        let engine = self.engine.trim().to_ascii_lowercase();
        let engine = match engine.as_str() {
            "mysql" | "mariadb" | "aurora-mysql" | "aurora-postgresql" => engine,
            "postgres" | "postgresql" => "postgres".to_string(),
            _ => bail!(
                "unsupported engine `{}`; expected mysql, mariadb, postgres, postgresql, aurora-mysql or aurora-postgresql",
                self.engine
            ),
        };

        let host = self.host.trim();
        if host.is_empty() || host.len() > 255 {
            bail!("host must be between 1 and 255 characters");
        }
        if !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'))
        {
            bail!(
                "host must be a hostname, IP address or RDS endpoint (no scheme, path or credentials)"
            );
        }

        let port = match self.port {
            Some(port) => port,
            None if engine.contains("postgres") => DEFAULT_POSTGRES_PORT,
            None => DEFAULT_MYSQL_PORT,
        };
        if port == 0 {
            bail!("port must be between 1 and 65535");
        }

        let database = field("database", &self.database, 128)?;
        let username = field("username", &self.username, 128)?;
        if self.password.is_empty()
            || self.password.len() > 1024
            || self.password.chars().any(char::is_control)
        {
            bail!(
                "password must be between 1 and 1024 characters and contain no control characters"
            );
        }

        Ok(ValidatedRemoteDatabase {
            name: name.to_string(),
            engine,
            host: host.to_string(),
            port,
            database,
            username,
            password: self.password.clone(),
            tls: self.tls,
        })
    }
}

/// A validated, normalised remote server definition. Kept private to the
/// crate so callers cannot bypass `AddRemoteDatabase::validate()`.
#[derive(Clone)]
pub struct ValidatedRemoteDatabase {
    pub(crate) name: String,
    pub(crate) engine: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) database: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) tls: bool,
}

fn field(label: &str, value: &str, max: usize) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > max {
        bail!("{label} must be between 1 and {max} characters");
    }
    if value.chars().any(char::is_control) {
        bail!("{label} must not contain control characters");
    }
    Ok(value.to_string())
}

/// A convenient repository handle. The free functions below are equivalent;
/// the IPC dispatcher can use either without adding anything to `Stores`.
#[derive(Clone)]
pub struct RemoteDatabases {
    pool: SqlitePool,
    secret_key_path: PathBuf,
}

impl RemoteDatabases {
    pub fn new(pool: SqlitePool, secret_key_path: impl Into<PathBuf>) -> Self {
        Self {
            pool,
            secret_key_path: secret_key_path.into(),
        }
    }

    pub fn from_paths(pool: SqlitePool, paths: &crate::config::Paths) -> Self {
        Self::new(pool, paths.secret_key_path.clone())
    }

    pub async fn list(&self) -> Result<Vec<RemoteDatabaseRecord>> {
        list(&self.pool).await
    }

    pub async fn add(&self, input: &AddRemoteDatabase) -> Result<RemoteDatabaseRecord> {
        add(&self.pool, &self.secret_key_path, input).await
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        delete(&self.pool, id).await
    }
}

/// Internal row shape. `password_encrypted` is intentionally not selected on
/// the read path, so a list operation cannot leak a credential by accident.
#[derive(FromRow)]
struct RemoteDbRow {
    id: String,
    name: String,
    engine: String,
    host: String,
    port: i64,
    database: String,
    username: String,
    tls: i64,
}

impl From<RemoteDbRow> for RemoteDatabaseRecord {
    fn from(row: RemoteDbRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            engine: row.engine,
            host: row.host,
            port: row.port as u16,
            database: row.database,
            username: row.username,
            tls: row.tls != 0,
        }
    }
}

/// List registered remote servers. Passwords are never selected or returned.
pub async fn list(pool: &SqlitePool) -> Result<Vec<RemoteDatabaseRecord>> {
    let rows = sqlx::query_as::<_, RemoteDbRow>(
        "SELECT id, name, engine, host, port, database_name AS \"database\", username, tls
         FROM remote_databases ORDER BY name COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Validate and store a remote server. The password is encrypted before the
/// `INSERT`, using the persistent secret key and the generated id as AAD.
pub async fn add(
    pool: &SqlitePool,
    secret_key_path: impl AsRef<Path>,
    input: &AddRemoteDatabase,
) -> Result<RemoteDatabaseRecord> {
    let validated = input.validate()?;

    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM remote_databases WHERE name = ? COLLATE NOCASE")
            .bind(&validated.name)
            .fetch_optional(pool)
            .await?;
    if existing.is_some() {
        bail!(
            "a remote database named `{}` already exists",
            validated.name
        );
    }

    let id = Uuid::new_v4().to_string();
    let key = load_or_create_key(secret_key_path.as_ref())?;
    let password_encrypted = encrypt(&key, &validated.password, id.as_bytes())?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO remote_databases
            (id, name, engine, host, port, database_name, username,
             password_encrypted, tls, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&validated.name)
    .bind(&validated.engine)
    .bind(&validated.host)
    .bind(validated.port as i64)
    .bind(&validated.database)
    .bind(&validated.username)
    .bind(&password_encrypted)
    .bind(if validated.tls { 1i64 } else { 0i64 })
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(RemoteDatabaseRecord {
        id,
        name: validated.name,
        engine: validated.engine,
        host: validated.host,
        port: validated.port,
        database: validated.database,
        username: validated.username,
        tls: validated.tls,
    })
}

/// Delete a remote server by id. Returns `false` when nothing was removed.
pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool> {
    let result = sqlx::query("DELETE FROM remote_databases WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Secret key handling and AEAD
// ---------------------------------------------------------------------------

const ENCRYPTED_PREFIX: &str = "v1:";

/// Read the persistent secret key, creating it (0600) if it is missing.
/// A 32-byte hex string is used as-is; any other content is hashed to 32
/// bytes so a legacy/custom key file still works.
fn load_or_create_key(path: &Path) -> Result<[u8; 32]> {
    match std::fs::read_to_string(path) {
        Ok(contents) => key_from_text(&contents),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("cannot create {}", parent.display()))?;
            }
            let mut bytes = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut bytes);
            let hex = hex::encode(bytes);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(path) {
                Ok(mut file) => {
                    use std::io::Write;
                    file.write_all(hex.as_bytes())
                        .with_context(|| format!("cannot write {}", path.display()))?;
                    file.flush()?;
                    Ok(bytes)
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    // Another process won the race; read what it wrote.
                    let contents = std::fs::read_to_string(path)?;
                    key_from_text(&contents)
                }
                Err(error) => Err(error)
                    .with_context(|| format!("cannot create the secret key at {}", path.display())),
            }
        }
        Err(error) => {
            Err(error).with_context(|| format!("cannot read the secret key at {}", path.display()))
        }
    }
}

fn key_from_text(contents: &str) -> Result<[u8; 32]> {
    let text = contents.trim();
    if let Ok(bytes) = hex::decode(text) {
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
    }
    // Not a 32-byte hex key: derive a stable key from the file contents.
    let digest = Sha256::digest(text.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest[..32]);
    Ok(key)
}

fn encrypt(key: &[u8; 32], plaintext: &str, aad: &[u8]) -> Result<String> {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let payload = Payload {
        msg: plaintext.as_bytes(),
        aad,
    };
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), payload)
        .map_err(|_| anyhow!("cannot encrypt the remote database password"))?;
    let mut blob = Vec::with_capacity(nonce_bytes.len() + ciphertext.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);
    Ok(format!("{ENCRYPTED_PREFIX}{}", BASE64.encode(blob)))
}

#[allow(dead_code)]
fn decrypt(key: &[u8; 32], stored: &str, aad: &[u8]) -> Result<String> {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

    let encoded = stored
        .strip_prefix(ENCRYPTED_PREFIX)
        .ok_or_else(|| anyhow!("the stored password is not encrypted"))?;
    let blob = BASE64
        .decode(encoded)
        .map_err(|_| anyhow!("the stored password is malformed"))?;
    if blob.len() < 12 {
        bail!("the stored password is truncated");
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let payload = Payload {
        msg: &blob[12..],
        aad,
    };
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&blob[..12]), payload)
        .map_err(|_| anyhow!("cannot decrypt the remote database password"))?;
    String::from_utf8(plaintext).map_err(|_| anyhow!("the stored password is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn sample() -> AddRemoteDatabase {
        AddRemoteDatabase {
            name: "analytics".into(),
            engine: "aurora-postgresql".into(),
            host: "analytics.cluster-abc123.us-east-1.rds.amazonaws.com".into(),
            port: None,
            database: "reports".into(),
            username: "reader".into(),
            password: "s3cr3t-p@ss".into(),
            tls: true,
        }
    }

    async fn test_pool() -> SqlitePool {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-remote-db-{}.db", Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();
        pool
    }

    fn test_key_path() -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-remote-key-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path.join("secret.key")
    }

    #[test]
    fn validates_fields_and_defaults_ports() {
        let validated = sample().validate().unwrap();
        assert_eq!(validated.engine, "aurora-postgresql");
        assert_eq!(validated.port, DEFAULT_POSTGRES_PORT);
        assert!(validated.tls);

        let mut mysql = sample();
        mysql.engine = "MySQL".into();
        mysql.port = Some(3307);
        let validated = mysql.validate().unwrap();
        assert_eq!(validated.engine, "mysql");
        assert_eq!(validated.port, 3307);

        let mut postgres = sample();
        postgres.engine = "postgresql".into();
        assert_eq!(postgres.validate().unwrap().engine, "postgres");
        assert_eq!(postgres.validate().unwrap().port, DEFAULT_POSTGRES_PORT);
    }

    #[test]
    fn rejects_invalid_input() {
        let mut input = sample();
        input.engine = "oracle".into();
        assert!(input.validate().is_err());

        let mut input = sample();
        input.host = "https://example.com/db".into();
        assert!(input.validate().is_err());

        let mut input = sample();
        input.port = Some(0);
        assert!(input.validate().is_err());

        let mut input = sample();
        input.name = String::new();
        assert!(input.validate().is_err());

        let mut input = sample();
        input.password = String::new();
        assert!(input.validate().is_err());
    }

    #[test]
    fn debug_never_reveals_the_password() {
        let input = sample();
        let rendered = format!("{input:?}");
        assert!(!rendered.contains("s3cr3t-p@ss"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn key_file_is_created_and_reused() {
        let path = test_key_path();
        let first = load_or_create_key(&path).unwrap();
        let second = load_or_create_key(&path).unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read_to_string(&path).unwrap().len(), 64);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn encryption_roundtrips_and_binds_to_the_id() {
        let key = [7u8; 32];
        let id = b"record-id";
        let stored = encrypt(&key, "hunter2", id).unwrap();
        assert!(!stored.contains("hunter2"));
        assert!(stored.starts_with(ENCRYPTED_PREFIX));
        assert_eq!(decrypt(&key, &stored, id).unwrap(), "hunter2");
        assert!(decrypt(&key, &stored, b"other-id").is_err());
        assert!(decrypt(&[8u8; 32], &stored, id).is_err());
    }

    #[tokio::test]
    async fn passwords_are_encrypted_at_rest_and_list_is_credential_free() {
        let pool = test_pool().await;
        let key_path = test_key_path();
        let created = add(&pool, &key_path, &sample()).await.unwrap();

        let stored: String =
            sqlx::query_scalar("SELECT password_encrypted FROM remote_databases WHERE id = ?")
                .bind(&created.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(stored.starts_with(ENCRYPTED_PREFIX));
        assert!(!stored.contains("s3cr3t-p@ss"));
        let key = load_or_create_key(&key_path).unwrap();
        assert_eq!(
            decrypt(&key, &stored, created.id.as_bytes()).unwrap(),
            "s3cr3t-p@ss"
        );

        let rows = list(&pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        let value = serde_json::to_value(&rows[0]).unwrap();
        let object = value.as_object().unwrap();
        for expected in [
            "id", "name", "engine", "host", "port", "database", "username", "tls",
        ] {
            assert!(object.contains_key(expected), "missing `{expected}`");
        }
        assert_eq!(object.len(), 8, "list must not expose extra fields");
        assert_eq!(object["database"], "reports");
        assert_eq!(object["port"], 5432);
        assert_eq!(object["tls"], true);
    }

    #[tokio::test]
    async fn add_rejects_duplicate_names_and_delete_removes_records() {
        let pool = test_pool().await;
        let key_path = test_key_path();
        let created = add(&pool, &key_path, &sample()).await.unwrap();
        let mut duplicate = sample();
        duplicate.name = "ANALYTICS".into();
        assert!(add(&pool, &key_path, &duplicate).await.is_err());

        assert!(delete(&pool, &created.id).await.unwrap());
        assert!(!delete(&pool, &created.id).await.unwrap());
        assert!(list(&pool).await.unwrap().is_empty());
    }

    #[test]
    fn parses_ipc_params() {
        let params = serde_json::json!({
            "name": "prod",
            "engine": "mysql",
            "host": "db.example.com",
            "port": 3306,
            "database": "app",
            "username": "app",
            "password": "pw",
            "tls": false,
        });
        let input = AddRemoteDatabase::from_params(&params).unwrap();
        assert_eq!(input.validate().unwrap().port, 3306);
        assert!(AddRemoteDatabase::from_params(&serde_json::json!({})).is_err());
    }
}
