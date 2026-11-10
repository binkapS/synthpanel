//! Shared IPC protocol for SynthPanel.
//!
//! `synthpanel-core` listens on a Unix domain socket and speaks a
//! line-delimited JSON protocol. Every request carries the shared secret
//! token; the core verifies it before dispatching to a typed handler.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::path::Path;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufStream};
use tokio::net::UnixStream;

/// Protocol version implemented by this crate.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum size of a single protocol line (1 MiB).
pub const MAX_LINE: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    pub id: u64,
    pub token: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Envelope {
    pub fn new(
        id: u64,
        token: impl Into<String>,
        method: impl Into<String>,
        params: Value,
    ) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            token: token.into(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
}

impl Response {
    pub fn success(id: u64, result: Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: u64, error: Error) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{message}")]
pub struct Error {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl Error {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Well-known method names. Core must implement each of these.
pub mod methods {
    pub const HEALTH: &str = "health";
    pub const SETTINGS_GET: &str = "settings.get";
    pub const SETTINGS_SET: &str = "settings.set";
    pub const SETTINGS_LIST: &str = "settings.list";
    pub const USERS_GET_BY_EMAIL: &str = "users.get_by_email";
    pub const USERS_GET: &str = "users.get";
    pub const USERS_CREATE: &str = "users.create";
    pub const USERS_UPDATE: &str = "users.update";
    pub const USERS_LIST: &str = "users.list";
    pub const AUDIT_LIST: &str = "audit.list";
    pub const AUDIT_RECORD: &str = "audit.record";
    pub const METRICS_SNAPSHOT: &str = "metrics.snapshot";
    pub const METRICS_RANGE: &str = "metrics.range";
    pub const SERVICES_LIST: &str = "services.list";
    pub const SERVICES_CONTROL: &str = "services.control";
    pub const SITES_LIST: &str = "sites.list";
    pub const SITES_GET: &str = "sites.get";
    pub const SITES_CREATE: &str = "sites.create";
    pub const SITES_UPDATE: &str = "sites.update";
    pub const SITES_DELETE: &str = "sites.delete";
    pub const SITES_SSL_ISSUE: &str = "sites.ssl.issue";
    pub const SITES_SSL_STATUS: &str = "sites.ssl.status";
    pub const PHP_VERSIONS: &str = "php.versions";
    pub const DATABASES_LIST: &str = "databases.list";
    pub const DATABASES_GET: &str = "databases.get";
    pub const DATABASES_CREATE: &str = "databases.create";
    pub const DATABASES_DELETE: &str = "databases.delete";
    pub const DATABASES_ADD_USER: &str = "databases.add_user";
    pub const DATABASES_USERS: &str = "databases.users";
    pub const DATABASES_DELETE_USER: &str = "databases.delete_user";
    pub const DATABASES_ENGINES: &str = "databases.engines";
    pub const SECURITY_FIREWALL: &str = "security.firewall";
    pub const SECURITY_FIREWALL_APPLY: &str = "security.firewall.apply";
    pub const SECURITY_FAIL2BAN: &str = "security.fail2ban.jails";
    pub const SECURITY_FAIL2BAN_UNBAN: &str = "security.fail2ban.unban";
    pub const SECURITY_SSH_KEYS: &str = "security.ssh_keys";
    pub const SECURITY_SSH_KEY_ADD: &str = "security.ssh_key.add";
    pub const SECURITY_SSH_KEY_REMOVE: &str = "security.ssh_key.remove";

    pub const BACKUPS_LIST: &str = "backups.list";
    pub const BACKUPS_CREATE: &str = "backups.create";
    pub const BACKUPS_DELETE: &str = "backups.delete";
    pub const BACKUPS_VERIFY: &str = "backups.verify";
    pub const BACKUPS_RESTORE: &str = "backups.restore";
    pub const BACKUPS_DOWNLOAD: &str = "backups.download";
    pub const BACKUPS_SCHEDULES: &str = "backups.schedules";
    pub const BACKUPS_SCHEDULE_CREATE: &str = "backups.schedule.create";
    pub const BACKUPS_SCHEDULE_DELETE: &str = "backups.schedule.delete";

    pub const CRON_LIST: &str = "cron.list";
    pub const CRON_CREATE: &str = "cron.create";
    pub const CRON_UPDATE: &str = "cron.update";
    pub const CRON_DELETE: &str = "cron.delete";
    pub const CRON_RUN: &str = "cron.run";
    pub const CRON_RUNS: &str = "cron.runs";
    pub const CRON_DESCRIBE: &str = "cron.describe";

    pub const LOGS_SOURCES: &str = "logs.sources";
    pub const LOGS_READ: &str = "logs.read";
    pub const LOGS_TAIL: &str = "logs.tail";

    pub const FILES_LIST: &str = "files.list";
    pub const FILES_READ: &str = "files.read";
    pub const FILES_WRITE: &str = "files.write";
    pub const FILES_MKDIR: &str = "files.mkdir";
    pub const FILES_RENAME: &str = "files.rename";
    pub const FILES_DELETE: &str = "files.delete";
    pub const FILES_CHMOD: &str = "files.chmod";
    pub const FILES_ARCHIVE: &str = "files.archive";
    pub const FILES_EXTRACT: &str = "files.extract";

    pub const API_KEYS_LIST: &str = "api_keys.list";
    pub const API_KEYS_CREATE: &str = "api_keys.create";
    pub const API_KEYS_DELETE: &str = "api_keys.delete";
    pub const API_KEYS_VERIFY: &str = "api_keys.verify";
}

/// Well-known error codes.
pub mod codes {
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const BAD_REQUEST: &str = "bad_request";
    pub const NOT_FOUND: &str = "not_found";
    pub const CONFLICT: &str = "conflict";
    pub const INTERNAL: &str = "internal";
    pub const UNSUPPORTED: &str = "unsupported";
    pub const RATE_LIMITED: &str = "rate_limited";
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(#[from] Error),
    #[error("malformed response: {0}")]
    Malformed(String),
}

/// A connected IPC client. Cheap to clone per task is not supported; use one
/// per connection or wrap in a mutex.
pub struct Client {
    stream: BufStream<UnixStream>,
    next_id: u64,
    token: String,
}

impl Client {
    pub async fn connect(
        path: impl AsRef<Path>,
        token: impl Into<String>,
    ) -> std::io::Result<Self> {
        let stream = UnixStream::connect(path).await?;
        Ok(Self {
            stream: BufStream::new(stream),
            next_id: 1,
            token: token.into(),
        })
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Perform a request/response round trip.
    pub async fn call<T: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<T, ClientError> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let envelope = Envelope::new(id, self.token.clone(), method, params);
        self.send(envelope).await?;
        let response = self.recv().await?;
        if response.id != id {
            return Err(ClientError::Malformed(format!(
                "response id mismatch: expected {id}, got {}",
                response.id
            )));
        }
        if response.ok {
            let value = response.result.unwrap_or(Value::Null);
            serde_json::from_value(value)
                .map_err(|e| ClientError::Malformed(format!("cannot decode result: {e}")))
        } else {
            Err(ClientError::Protocol(response.error.unwrap_or_else(|| {
                Error::new(codes::INTERNAL, "missing error details")
            })))
        }
    }

    pub async fn call_unit(&mut self, method: &str, params: Value) -> Result<(), ClientError> {
        self.call::<Value>(method, params).await.map(|_| ())
    }

    async fn send(&mut self, envelope: Envelope) -> Result<(), ClientError> {
        let mut line = serde_json::to_vec(&envelope)
            .map_err(|e| ClientError::Malformed(format!("cannot encode request: {e}")))?;
        line.push(b'\n');
        self.stream.write_all(&line).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<Response, ClientError> {
        let mut reader = BufReader::new(&mut self.stream);
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(ClientError::Malformed("connection closed".into()));
        }
        if n > MAX_LINE {
            return Err(ClientError::Malformed("line too long".into()));
        }
        serde_json::from_str(line.trim_end())
            .map_err(|e| ClientError::Malformed(format!("cannot decode response: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let env = Envelope::new(7, "secret", methods::HEALTH, serde_json::json!({}));
        let s = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, 7);
        assert_eq!(back.token, "secret");
        assert_eq!(back.method, methods::HEALTH);
    }

    #[test]
    fn response_success_serializes_without_error() {
        let resp = Response::success(1, serde_json::json!({"ok": true}));
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"ok\":true"));
        assert!(!s.contains("\"error\""));
    }
}
