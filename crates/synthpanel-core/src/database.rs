//! Database and database-user lifecycle.
//!
//! The panel talks to the database servers through their own clients
//! (`mysql`, `psql`, `redis-cli`) instead of embedding a driver: the servers
//! are already configured by the distribution packages, and the panel only
//! needs to create databases and users with well-defined privileges.

use anyhow::{Context, Result, anyhow, bail};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};

/// The engines the panel manages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Mysql,
    Postgres,
    Redis,
}

impl Engine {
    pub fn as_str(&self) -> &'static str {
        match self {
            Engine::Mysql => "mysql",
            Engine::Postgres => "postgres",
            Engine::Redis => "redis",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "mysql" | "mariadb" => Some(Engine::Mysql),
            "postgres" | "postgresql" | "pgsql" => Some(Engine::Postgres),
            "redis" => Some(Engine::Redis),
            _ => None,
        }
    }

    /// Whether the engine has the concept of separate databases and users.
    pub fn supports_users(&self) -> bool {
        matches!(self, Engine::Mysql | Engine::Postgres)
    }
}

/// A database user with its privileges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseUser {
    pub username: String,
    pub password: String,
    pub privileges: Vec<String>,
}

/// Connection details for a database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub engine: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
}

impl ConnectionInfo {
    /// A ready-to-paste connection string.
    pub fn connection_string(&self) -> String {
        match self.engine.as_str() {
            "mysql" => format!(
                "mysql://{}:{}@{}:{}/{}",
                self.username, self.password, self.host, self.port, self.database
            ),
            "postgres" => format!(
                "postgresql://{}:{}@{}:{}/{}",
                self.username, self.password, self.host, self.port, self.database
            ),
            "redis" => format!(
                "redis://{}@{}:{}/{}",
                self.username, self.host, self.port, self.database
            ),
            _ => String::new(),
        }
    }

    /// The equivalent `.env` snippet.
    pub fn env_snippet(&self) -> String {
        match self.engine.as_str() {
            "mysql" => format!(
                "DB_CONNECTION=mysql\nDB_HOST={}\nDB_PORT={}\nDB_DATABASE={}\nDB_USERNAME={}\nDB_PASSWORD={}\n",
                self.host, self.port, self.database, self.username, self.password
            ),
            "postgres" => format!(
                "DB_CONNECTION=pgsql\nDB_HOST={}\nDB_PORT={}\nDB_DATABASE={}\nDB_USERNAME={}\nDB_PASSWORD={}\n",
                self.host, self.port, self.database, self.username, self.password
            ),
            "redis" => format!(
                "REDIS_HOST={}\nREDIS_PORT={}\nREDIS_PASSWORD={}\n",
                self.host, self.port, self.password
            ),
            _ => String::new(),
        }
    }
}

/// Validate a database name.
///
/// Identifiers are restricted to a conservative subset so they can never
/// break out of a quoted SQL context.
pub fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("the name must not be empty");
    }
    if name.len() > 64 {
        bail!("the name must be at most 64 characters long");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("the name may only contain letters, digits, underscores and dashes");
    }
    if name.starts_with('-') {
        bail!("the name must not start with a dash");
    }
    Ok(name.to_string())
}

/// Validate a username.
pub fn validate_username(username: &str) -> Result<String> {
    let username = username.trim();
    if username.is_empty() {
        bail!("the username must not be empty");
    }
    if username.len() > 32 {
        bail!("the username must be at most 32 characters long");
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("the username may only contain letters, digits, underscores and dashes");
    }
    Ok(username.to_string())
}

/// Generate a strong random password.
pub fn generate_password(length: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#%^_-";
    let mut rng = rand::thread_rng();
    let mut password = String::with_capacity(length);
    for _ in 0..length {
        let index = rng.gen_range(0..ALPHABET.len());
        password.push(ALPHABET[index] as char);
    }
    password
}

/// Escape a value for a single-quoted MySQL string.
fn mysql_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Escape an identifier for MySQL backtick quoting.
fn mysql_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

/// Escape a value for a single-quoted PostgreSQL string.
fn postgres_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Escape an identifier for PostgreSQL double-quote quoting.
fn postgres_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// Build the SQL that creates a MySQL database and user.
pub fn mysql_create_sql(database: &str, username: &str, password: &str) -> String {
    format!(
        "CREATE DATABASE IF NOT EXISTS {db} CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\
         CREATE USER IF NOT EXISTS {user}@'localhost' IDENTIFIED BY {password};\
         GRANT ALL PRIVILEGES ON {db}.* TO {user}@'localhost';\
         FLUSH PRIVILEGES;",
        db = mysql_identifier(database),
        user = mysql_identifier(username),
        password = mysql_quote(password),
    )
}

/// Build the SQL that creates a PostgreSQL database and user.
pub fn postgres_create_sql(database: &str, username: &str, password: &str) -> String {
    format!(
        "CREATE USER {user} WITH PASSWORD {password};\
         CREATE DATABASE {db} OWNER {user};\
         GRANT ALL PRIVILEGES ON DATABASE {db} TO {user};",
        db = postgres_identifier(database),
        user = postgres_identifier(username),
        password = postgres_quote(password),
    )
}

/// Build the SQL that drops a MySQL database and user.
pub fn mysql_drop_sql(database: &str, username: Option<&str>) -> String {
    let mut sql = format!("DROP DATABASE IF EXISTS {};", mysql_identifier(database));
    if let Some(username) = username {
        sql.push_str(&format!(
            "DROP USER IF EXISTS {}@'localhost';",
            mysql_identifier(username)
        ));
        sql.push_str("FLUSH PRIVILEGES;");
    }
    sql
}

/// Build the SQL that drops a PostgreSQL database and user.
pub fn postgres_drop_sql(database: &str, username: Option<&str>) -> String {
    let mut sql = format!("DROP DATABASE IF EXISTS {};", postgres_identifier(database));
    if let Some(username) = username {
        sql.push_str(&format!(
            "DROP USER IF EXISTS {};",
            postgres_identifier(username)
        ));
    }
    sql
}

/// The admin credentials used to reach the local database server.
#[derive(Debug, Clone)]
pub struct AdminCredentials {
    pub mysql_root_password: Option<String>,
    pub postgres_user: String,
    pub postgres_password: Option<String>,
}

impl Default for AdminCredentials {
    fn default() -> Self {
        Self {
            mysql_root_password: None,
            postgres_user: "postgres".to_string(),
            postgres_password: None,
        }
    }
}

/// Run a command and capture its output.
fn run_capture(program: &str, args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut command = Command::new(program);
    command.args(args);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("cannot spawn `{program}`; is it installed?"))?;
    if let Some(input) = stdin {
        use std::io::Write;
        if let Some(mut pipe) = child.stdin.take() {
            pipe.write_all(input.as_bytes())?;
        }
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!("{program} failed: {}{}", stderr.trim(), stdout.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Whether a client binary is available.
pub fn client_available(program: &str) -> bool {
    Path::new("/usr/bin").join(program).exists() || which(program).is_some()
}

fn which(program: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.exists())
}

/// Create a MySQL database with its user.
pub fn mysql_create(
    database: &str,
    username: &str,
    password: &str,
    credentials: &AdminCredentials,
) -> Result<()> {
    let sql = mysql_create_sql(database, username, password);
    let mut args: Vec<String> = vec!["-u".into(), "root".into()];
    if let Some(root_password) = &credentials.mysql_root_password {
        args.push(format!("-p{root_password}"));
    }
    args.push("-e".into());
    args.push(sql);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    run_capture("mysql", &borrowed, None)?;
    Ok(())
}

/// Drop a MySQL database and optionally its user.
pub fn mysql_drop(
    database: &str,
    username: Option<&str>,
    credentials: &AdminCredentials,
) -> Result<()> {
    let sql = mysql_drop_sql(database, username);
    let mut args: Vec<String> = vec!["-u".into(), "root".into()];
    if let Some(root_password) = &credentials.mysql_root_password {
        args.push(format!("-p{root_password}"));
    }
    args.push("-e".into());
    args.push(sql);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    run_capture("mysql", &borrowed, None)?;
    Ok(())
}

/// Create a PostgreSQL database with its user.
pub fn postgres_create(
    database: &str,
    username: &str,
    password: &str,
    credentials: &AdminCredentials,
) -> Result<()> {
    let sql = postgres_create_sql(database, username, password);
    let mut args: Vec<String> = vec!["-U".into(), credentials.postgres_user.clone()];
    if let Some(pg_password) = &credentials.postgres_password {
        args.push(format!("-p{pg_password}"));
    }
    args.push("-c".into());
    args.push(sql);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    run_capture("psql", &borrowed, None)?;
    Ok(())
}

/// Drop a PostgreSQL database and optionally its user.
pub fn postgres_drop(
    database: &str,
    username: Option<&str>,
    credentials: &AdminCredentials,
) -> Result<()> {
    let sql = postgres_drop_sql(database, username);
    let mut args: Vec<String> = vec!["-U".into(), credentials.postgres_user.clone()];
    if let Some(pg_password) = &credentials.postgres_password {
        args.push(format!("-p{pg_password}"));
    }
    args.push("-c".into());
    args.push(sql);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    run_capture("psql", &borrowed, None)?;
    Ok(())
}

/// Set a Redis password and restart the service.
pub fn redis_set_password(password: &str) -> Result<()> {
    let config = Path::new("/etc/redis/redis.conf");
    if !config.exists() {
        bail!("/etc/redis/redis.conf is missing; is redis-server installed?");
    }
    let contents = std::fs::read_to_string(config)?;
    let mut updated = String::new();
    let mut found = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("requirepass ") {
            updated.push_str(&format!("requirepass {password}\n"));
            found = true;
        } else {
            updated.push_str(line);
            updated.push('\n');
        }
    }
    if !found {
        updated.push_str(&format!("\nrequirepass {password}\n"));
    }
    std::fs::write(config, updated)?;
    let _ = Command::new("systemctl")
        .arg("restart")
        .arg("redis-server")
        .output();
    Ok(())
}

/// Verify that a Redis password works.
pub fn redis_verify(password: &str) -> Result<bool> {
    let output = Command::new("redis-cli")
        .arg("-a")
        .arg(password)
        .arg("PING")
        .output();
    match output {
        Ok(output) => {
            Ok(output.status.success() && String::from_utf8_lossy(&output.stdout).contains("PONG"))
        }
        Err(_) => Ok(false),
    }
}

/// Check that a MySQL server answers.
pub fn mysql_ping(credentials: &AdminCredentials) -> Result<bool> {
    let mut args: Vec<String> = vec!["-u".into(), "root".into()];
    if let Some(root_password) = &credentials.mysql_root_password {
        args.push(format!("-p{root_password}"));
    }
    args.push("-e".into());
    args.push("SELECT 1".into());
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    match run_capture("mysql", &borrowed, None) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Check that a PostgreSQL server answers.
pub fn postgres_ping(credentials: &AdminCredentials) -> Result<bool> {
    let mut args: Vec<String> = vec!["-U".into(), credentials.postgres_user.clone()];
    if let Some(pg_password) = &credentials.postgres_password {
        args.push(format!("-p{pg_password}"));
    }
    args.push("-c".into());
    args.push("SELECT 1".into());
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    match run_capture("psql", &borrowed, None) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Everything the panel needs to know about one engine's health.
#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    pub engine: String,
    pub client_available: bool,
    pub server_reachable: bool,
}

pub fn engine_status(engine: Engine, credentials: &AdminCredentials) -> EngineStatus {
    let (client, reachable) = match engine {
        Engine::Mysql => (
            client_available("mysql"),
            mysql_ping(credentials).unwrap_or(false),
        ),
        Engine::Postgres => (
            client_available("psql"),
            postgres_ping(credentials).unwrap_or(false),
        ),
        Engine::Redis => (
            client_available("redis-cli"),
            redis_verify("").unwrap_or(false)
                || Command::new("redis-cli")
                    .arg("PING")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false),
        ),
    };
    EngineStatus {
        engine: engine.as_str().to_string(),
        client_available: client,
        server_reachable: reachable,
    }
}

/// Ensure the engine name is supported.
pub fn require_engine(name: &str) -> Result<Engine> {
    Engine::parse(name).ok_or_else(|| anyhow!("unsupported database engine `{name}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_engines() {
        assert_eq!(Engine::parse("mysql"), Some(Engine::Mysql));
        assert_eq!(Engine::parse("MariaDB"), Some(Engine::Mysql));
        assert_eq!(Engine::parse("postgresql"), Some(Engine::Postgres));
        assert_eq!(Engine::parse("redis"), Some(Engine::Redis));
        assert_eq!(Engine::parse("oracle"), None);
    }

    #[test]
    fn validates_names() {
        assert_eq!(validate_name("shop_db").unwrap(), "shop_db");
        assert!(validate_name("").is_err());
        assert!(validate_name("drop; table").is_err());
        assert!(validate_name(&"a".repeat(65)).is_err());
        assert!(validate_name("-leading").is_err());
    }

    #[test]
    fn validates_usernames() {
        assert_eq!(validate_username("shop_user").unwrap(), "shop_user");
        assert!(validate_username("").is_err());
        assert!(validate_username("bad user").is_err());
        assert!(validate_username(&"x".repeat(33)).is_err());
    }

    #[test]
    fn generates_distinct_passwords() {
        let a = generate_password(24);
        let b = generate_password(24);
        assert_eq!(a.len(), 24);
        assert_ne!(a, b);
    }

    #[test]
    fn mysql_sql_is_quoted() {
        let sql = mysql_create_sql("shop", "user", "pa'ss");
        assert!(sql.contains("CREATE DATABASE IF NOT EXISTS `shop`"));
        assert!(sql.contains("IDENTIFIED BY 'pa\\'ss'"));
        assert!(!sql.contains("'; DROP"));
    }

    #[test]
    fn postgres_sql_is_quoted() {
        let sql = postgres_create_sql("shop", "user", "pw");
        assert!(sql.contains("CREATE DATABASE \"shop\" OWNER \"user\""));
        assert!(sql.contains("PASSWORD 'pw'"));
    }

    #[test]
    fn drop_sql_removes_the_user() {
        let sql = mysql_drop_sql("shop", Some("user"));
        assert!(sql.contains("DROP DATABASE IF EXISTS `shop`"));
        assert!(sql.contains("DROP USER IF EXISTS `user`@'localhost'"));
    }

    #[test]
    fn connection_strings_are_well_formed() {
        let info = ConnectionInfo {
            engine: "mysql".into(),
            host: "127.0.0.1".into(),
            port: 3306,
            database: "shop".into(),
            username: "user".into(),
            password: "pw".into(),
        };
        assert_eq!(
            info.connection_string(),
            "mysql://user:pw@127.0.0.1:3306/shop"
        );
        assert!(info.env_snippet().contains("DB_DATABASE=shop"));
    }

    #[test]
    fn postgres_connection_string() {
        let info = ConnectionInfo {
            engine: "postgres".into(),
            host: "127.0.0.1".into(),
            port: 5432,
            database: "shop".into(),
            username: "user".into(),
            password: "pw".into(),
        };
        assert_eq!(
            info.connection_string(),
            "postgresql://user:pw@127.0.0.1:5432/shop"
        );
    }
}
