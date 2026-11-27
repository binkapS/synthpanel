//! On-demand SQL database dumps and imports.
//!
//! The core never embeds a database driver: it shells out to the distribution
//! clients (`mysqldump`/`mysql`, `pg_dump`/`psql`) with an explicit argument
//! vector, so user-controlled values are passed as individual arguments and
//! never interpreted by a shell. Dumps are also archived under the configured
//! backup directory with a sanitised filename before they are returned over
//! IPC, so an operator can recover a dump even if the panel database is lost.

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config::Paths;
use crate::database::{AdminCredentials, Engine as DbEngine};
use crate::databases::DatabaseRecord;

/// Upper bound for a single dump, both on disk and in the IPC response.
///
/// The IPC protocol uses newline-delimited JSON with a 1 MiB line limit, so a
/// base64 payload must stay well below that. Larger databases should be
/// exported through the scheduled backup system instead.
pub const MAX_DUMP_BYTES: usize = 512 * 1024;

/// The binary locations used for dump/import operations.
///
/// Each binary can be overridden with a `SYNTHPANEL_*_BIN` environment
/// variable, matching the `AcmePaths::from_env` style used for acme.sh. The
/// defaults are plain command names, which `Command` resolves via `PATH`.
#[derive(Debug, Clone)]
pub struct DatabaseBinaries {
    pub mysqldump: PathBuf,
    pub mysql: PathBuf,
    pub pg_dump: PathBuf,
    pub psql: PathBuf,
}

impl Default for DatabaseBinaries {
    fn default() -> Self {
        Self {
            mysqldump: PathBuf::from("mysqldump"),
            mysql: PathBuf::from("mysql"),
            pg_dump: PathBuf::from("pg_dump"),
            psql: PathBuf::from("psql"),
        }
    }
}

impl DatabaseBinaries {
    pub fn from_env() -> Self {
        let configured = |key: &str, default: &str| -> PathBuf {
            std::env::var_os(key)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(default))
        };
        Self {
            mysqldump: configured("SYNTHPANEL_MYSQLDUMP_BIN", "mysqldump"),
            mysql: configured("SYNTHPANEL_MYSQL_BIN", "mysql"),
            pg_dump: configured("SYNTHPANEL_PG_DUMP_BIN", "pg_dump"),
            psql: configured("SYNTHPANEL_PSQL_BIN", "psql"),
        }
    }
}

/// The IPC payload of `databases.dump`.
#[derive(Debug, Clone, Serialize)]
pub struct DumpResult {
    /// The managed database name.
    pub name: String,
    /// Base64-encoded SQL dump (UTF-8 safe for JSON transport).
    pub data: String,
}

/// Read the server credentials with the same environment variables the rest
/// of the core uses for database provisioning.
fn credentials() -> AdminCredentials {
    AdminCredentials {
        mysql_root_password: std::env::var("SYNTHPANEL_MYSQL_ROOT_PASSWORD").ok(),
        postgres_user: std::env::var("SYNTHPANEL_POSTGRES_USER")
            .unwrap_or_else(|_| "postgres".to_string()),
        postgres_password: std::env::var("SYNTHPANEL_POSTGRES_PASSWORD").ok(),
    }
}

/// Map a stored engine string to a dumpable engine.
fn dump_engine(engine: &str) -> Result<DbEngine> {
    match DbEngine::parse(engine) {
        Some(engine @ (DbEngine::Mysql | DbEngine::Postgres)) => Ok(engine),
        Some(DbEngine::Redis) => bail!("redis databases do not support SQL dumps"),
        None => bail!("unsupported database engine `{engine}`"),
    }
}

/// Reduce a value to a single, filesystem-safe path component.
///
/// Only ASCII letters, digits, `_` and `-` survive; everything else becomes
/// `_`. This makes it impossible for a database name to introduce a path
/// separator or a `..` component in an archive filename.
pub fn safe_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// Filename used for an archived dump in `<backup_dir>/databases`.
fn dump_file_name(prefix: &str, engine: &str, name: &str) -> String {
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    format!(
        "{prefix}_{}_{}_{stamp}.sql",
        safe_component(engine),
        safe_component(name)
    )
}

/// Decode the `data` field of a dump request.
///
/// Dumps produced by this module are base64 encoded. Raw SQL text is also
/// accepted so an operator can paste a dump directly into the API.
pub fn decode_dump(data: &str) -> Result<Vec<u8>> {
    let trimmed = data.trim();
    if trimmed.is_empty() {
        bail!("the dump data is empty");
    }
    match BASE64.decode(trimmed) {
        Ok(bytes) if !bytes.is_empty() => Ok(bytes),
        _ => Ok(trimmed.as_bytes().to_vec()),
    }
}

/// Run a client command, feeding it optional stdin and capturing stdout.
fn run_capture(command: &mut Command, stdin: Option<&[u8]>) -> Result<Vec<u8>> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let program = command.get_program().to_string_lossy().to_string();
    let mut child = command
        .spawn()
        .with_context(|| format!("cannot spawn `{program}`; is it installed?"))?;
    if let Some(input) = stdin {
        if let Some(mut pipe) = child.stdin.take() {
            pipe.write_all(input)?;
        }
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{program} failed: {}", stderr.trim());
    }
    Ok(output.stdout)
}

/// Add the common MySQL invocation arguments and credentials.
fn mysql_args(command: &mut Command, credentials: &AdminCredentials) {
    command.arg("--user=root");
    if let Some(password) = &credentials.mysql_root_password {
        command.env("MYSQL_PWD", password);
    }
}

/// Add the common PostgreSQL invocation arguments and credentials.
fn postgres_args(command: &mut Command, credentials: &AdminCredentials) {
    command.arg(format!("--username={}", credentials.postgres_user));
    if let Some(password) = &credentials.postgres_password {
        command.env("PGPASSWORD", password);
    }
}

/// Archive dump bytes under the backup directory and return the file path.
fn archive_dump(
    paths: &Paths,
    prefix: &str,
    engine: &str,
    name: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    let directory = paths.backup_dir.join("databases");
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("cannot create {}", directory.display()))?;
    let target = directory.join(dump_file_name(prefix, engine, name));
    std::fs::write(&target, bytes).with_context(|| format!("cannot write {}", target.display()))?;
    Ok(target)
}

/// Dump a single managed database.
pub fn dump(record: &DatabaseRecord, paths: &Paths) -> Result<DumpResult> {
    let engine = dump_engine(&record.engine)?;
    let name = crate::database::validate_name(&record.name)?;
    let binaries = DatabaseBinaries::from_env();
    let credentials = credentials();

    let stdout = match engine {
        DbEngine::Mysql => {
            let mut command = Command::new(&binaries.mysqldump);
            mysql_args(&mut command, &credentials);
            command
                .arg("--single-transaction")
                .arg("--no-tablespaces")
                .arg(&name);
            run_capture(&mut command, None)?
        }
        DbEngine::Postgres => {
            let mut command = Command::new(&binaries.pg_dump);
            postgres_args(&mut command, &credentials);
            command
                .arg("--format=plain")
                .arg("--no-owner")
                .arg("--no-privileges")
                .arg(&name);
            run_capture(&mut command, None)?
        }
        DbEngine::Redis => unreachable!("validated above"),
    };

    if stdout.is_empty() {
        bail!("the database dump is empty");
    }
    if stdout.len() > MAX_DUMP_BYTES {
        bail!(
            "the database dump is too large ({} bytes, limit {} bytes); use scheduled backups instead",
            stdout.len(),
            MAX_DUMP_BYTES
        );
    }

    let archive = archive_dump(paths, "dump", engine.as_str(), &name, &stdout)?;
    tracing::info!(path = %archive.display(), bytes = stdout.len(), "archived database dump");
    Ok(DumpResult {
        name: record.name.clone(),
        data: BASE64.encode(&stdout),
    })
}

/// Import SQL into a single managed database.
pub fn import(record: &DatabaseRecord, data: &str, paths: &Paths) -> Result<()> {
    let engine = dump_engine(&record.engine)?;
    let name = crate::database::validate_name(&record.name)?;
    let bytes = decode_dump(data)?;
    if bytes.len() > MAX_DUMP_BYTES {
        bail!(
            "the import data is too large ({} bytes, limit {} bytes)",
            bytes.len(),
            MAX_DUMP_BYTES
        );
    }

    // Keep a copy of what was imported for the audit trail.
    let archive = archive_dump(paths, "import", engine.as_str(), &name, &bytes)?;
    let binaries = DatabaseBinaries::from_env();
    let credentials = credentials();

    match engine {
        DbEngine::Mysql => {
            let mut command = Command::new(&binaries.mysql);
            mysql_args(&mut command, &credentials);
            command.arg(&name);
            run_capture(&mut command, Some(&bytes))?;
        }
        DbEngine::Postgres => {
            let mut command = Command::new(&binaries.psql);
            postgres_args(&mut command, &credentials);
            command
                .arg("--dbname")
                .arg(&name)
                .arg("-v")
                .arg("ON_ERROR_STOP=1");
            run_capture(&mut command, Some(&bytes))?;
        }
        DbEngine::Redis => unreachable!("validated above"),
    }

    tracing::info!(path = %archive.display(), bytes = bytes.len(), "imported database dump");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_component_blocks_path_traversal() {
        assert_eq!(safe_component("shop"), "shop");
        let hostile = safe_component("../../etc/passwd");
        assert!(!hostile.contains('/'));
        assert!(!hostile.contains('\\'));
        assert!(!hostile.contains(".."));
        assert_eq!(safe_component(""), "_");
        assert_eq!(safe_component("a b"), "a_b");
    }

    #[test]
    fn dump_file_name_is_flat_and_sql() {
        let name = dump_file_name("dump", "mysql", "../../etc/passwd");
        assert_eq!(name.matches('.').count(), 1, "only the .sql extension");
        assert!(name.ends_with(".sql"));
        assert!(name.starts_with("dump_mysql_"));
        assert!(!name.contains('/'));
    }

    #[test]
    fn decode_dump_accepts_base64_and_plain_sql() {
        let sql = b"CREATE TABLE t (id INT);\n";
        assert_eq!(decode_dump(&BASE64.encode(sql)).unwrap(), sql);
        assert_eq!(
            decode_dump("CREATE TABLE t (id INT);").unwrap(),
            b"CREATE TABLE t (id INT);".to_vec()
        );
        assert!(decode_dump("   ").is_err());
    }

    #[test]
    fn dump_engine_supports_sql_engines_only() {
        assert!(dump_engine("mysql").is_ok());
        assert!(dump_engine("mariadb").is_ok());
        assert!(dump_engine("postgresql").is_ok());
        assert!(dump_engine("redis").is_err());
        assert!(dump_engine("nonsense").is_err());
    }

    #[test]
    fn binary_defaults_use_command_names() {
        let binaries = DatabaseBinaries::default();
        assert_eq!(binaries.mysqldump, PathBuf::from("mysqldump"));
        assert_eq!(binaries.mysql, PathBuf::from("mysql"));
        assert_eq!(binaries.pg_dump, PathBuf::from("pg_dump"));
        assert_eq!(binaries.psql, PathBuf::from("psql"));
    }
}
