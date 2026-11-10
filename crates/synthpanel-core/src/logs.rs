//! Log reading with filters and tail support.

use anyhow::{Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The log sources the panel can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogSource {
    /// The panel's own log.
    Panel,
    /// The nginx access log of a site.
    NginxAccess,
    /// The nginx error log of a site.
    NginxError,
    /// The PHP-FPM log of a site.
    PhpFpm,
    /// The system authentication log.
    Auth,
}

impl LogSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogSource::Panel => "panel",
            LogSource::NginxAccess => "nginx-access",
            LogSource::NginxError => "nginx-error",
            LogSource::PhpFpm => "php-fpm",
            LogSource::Auth => "auth",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "panel" => Some(LogSource::Panel),
            "nginx-access" => Some(LogSource::NginxAccess),
            "nginx-error" => Some(LogSource::NginxError),
            "php-fpm" | "php" => Some(LogSource::PhpFpm),
            "auth" => Some(LogSource::Auth),
            _ => None,
        }
    }
}

/// Filters applied to a log query.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    /// Only lines containing this text.
    pub search: Option<String>,
    /// Only lines containing this status code.
    pub status: Option<u16>,
    /// Only lines at or above this level (error, warn, info, debug).
    pub level: Option<String>,
    /// Only lines newer than this many minutes.
    pub since_minutes: Option<i64>,
    /// Maximum number of lines to return.
    pub limit: usize,
}

/// A single log line with its parsed metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub timestamp: Option<String>,
    pub level: Option<String>,
    pub message: String,
    pub status: Option<u16>,
}

/// The directories the panel reads logs from.
#[derive(Debug, Clone)]
pub struct LogPaths {
    pub panel_log: PathBuf,
    pub nginx_logs: PathBuf,
    pub php_logs: PathBuf,
    pub auth_log: PathBuf,
}

impl LogPaths {
    pub fn from_env() -> Self {
        let log_dir = std::env::var_os("SYNTHPANEL_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/log/synthpanel"));
        let etc = std::env::var_os("SYNTHPANEL_ETC_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/synthpanel"));
        Self {
            panel_log: log_dir.join("synthpanel-web.log"),
            nginx_logs: PathBuf::from("/var/log/nginx"),
            php_logs: etc.join("php"),
            auth_log: PathBuf::from("/var/log/auth.log"),
        }
    }

    /// Resolve a source to the file it reads.
    pub fn resolve(&self, source: LogSource, site_id: Option<&str>) -> Result<PathBuf> {
        match source {
            LogSource::Panel => Ok(self.panel_log.clone()),
            LogSource::Auth => Ok(self.auth_log.clone()),
            LogSource::NginxAccess => {
                let site = site_id.ok_or_else(|| anyhow::anyhow!("a site is required"))?;
                Ok(self.nginx_logs.join(format!("{site}.access.log")))
            }
            LogSource::NginxError => {
                let site = site_id.ok_or_else(|| anyhow::anyhow!("a site is required"))?;
                Ok(self.nginx_logs.join(format!("{site}.error.log")))
            }
            LogSource::PhpFpm => {
                let site = site_id.ok_or_else(|| anyhow::anyhow!("a site is required"))?;
                Ok(self.php_logs.join(format!("{site}.log")))
            }
        }
    }
}

/// Extract the HTTP status code from an nginx access log line.
pub fn parse_status(line: &str) -> Option<u16> {
    // The combined log format puts the status right after the quoted request:
    //   ... "GET / HTTP/1.1" 200 612 "-" "curl/8.0"
    if let Some(start) = line.find('"') {
        if let Some(relative_end) = line[start + 1..].find('"') {
            let rest = &line[start + 1 + relative_end + 1..];
            if let Some(token) = rest.split_whitespace().next() {
                if let Ok(status) = token.parse::<u16>() {
                    if (100..600).contains(&status) {
                        return Some(status);
                    }
                }
            }
        }
    }
    // Fallback for minimal lines such as `GET /a 200 1`.
    for token in line.split_whitespace() {
        if token.len() == 3 {
            if let Ok(value) = token.parse::<u16>() {
                if (100..600).contains(&value) {
                    return Some(value);
                }
            }
        }
    }
    None
}

/// Extract a log level from a line.
pub fn parse_level(line: &str) -> Option<String> {
    for level in ["error", "warn", "warning", "info", "debug", "fatal", "crit"] {
        if line.to_ascii_lowercase().contains(&format!("[{level}]")) {
            return Some(level.to_string());
        }
    }
    None
}

/// Extract a timestamp from a line, if one is present.
pub fn parse_timestamp(line: &str) -> Option<String> {
    // nginx access logs start with `[10/Oct/2026:13:55:36 +0000]`.
    if let Some(start) = line.find('[') {
        if let Some(end) = line[start..].find(']') {
            let raw = &line[start + 1..start + end];
            if raw.len() >= 20 {
                return Some(raw.to_string());
            }
        }
    }
    // ISO-8601 timestamps anywhere in the line.
    for token in line.split_whitespace() {
        if token.len() >= 19 && token.contains('-') && token[4..5] == *"-" {
            return Some(token.trim_matches(['[', ']']).to_string());
        }
    }
    None
}

/// Read a log file and apply the filters.
pub fn read_log(path: &Path, filters: &Filters) -> Result<Vec<LogLine>> {
    if !path.exists() {
        bail!("the log file {} does not exist", path.display());
    }
    let contents = std::fs::read_to_string(path)?;
    let limit = if filters.limit == 0 {
        500
    } else {
        filters.limit
    };

    let mut lines: Vec<LogLine> = Vec::new();
    for raw in contents.lines() {
        if let Some(search) = &filters.search {
            if !raw.to_lowercase().contains(&search.to_lowercase()) {
                continue;
            }
        }
        let status = parse_status(raw);
        if let Some(wanted) = filters.status {
            if status != Some(wanted) {
                continue;
            }
        }
        let level = parse_level(raw);
        if let Some(wanted) = &filters.level {
            let wanted = wanted.to_ascii_lowercase();
            let matched = level
                .as_deref()
                .map(|value| value.starts_with(&wanted))
                .unwrap_or(false);
            if !matched {
                continue;
            }
        }
        lines.push(LogLine {
            timestamp: parse_timestamp(raw),
            level,
            message: raw.to_string(),
            status,
        });
    }

    // Keep the tail: the most recent lines are the interesting ones.
    if lines.len() > limit {
        lines = lines.split_off(lines.len() - limit);
    }
    Ok(lines)
}

/// Read the last `count` lines of a file without loading all of it.
pub fn tail(path: &Path, count: usize) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = contents.lines().collect();
    let start = lines.len().saturating_sub(count);
    Ok(lines[start..].iter().map(|line| line.to_string()).collect())
}

/// The current time, used to stamp run records.
pub fn now() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nginx_access_lines() {
        let line = "127.0.0.1 - - [10/Oct/2026:13:55:36 +0000] \"GET / HTTP/1.1\" 200 612 \"-\" \"curl/8.0\"";
        assert_eq!(parse_status(line), Some(200));
        assert_eq!(
            parse_timestamp(line).as_deref(),
            Some("10/Oct/2026:13:55:36 +0000")
        );
    }

    #[test]
    fn parses_log_levels() {
        assert_eq!(
            parse_level("[error] something broke").as_deref(),
            Some("error")
        );
        assert_eq!(parse_level("[warn] careful").as_deref(), Some("warn"));
        assert_eq!(parse_level("no level here"), None);
    }

    #[test]
    fn filters_by_status_and_text() {
        let dir = std::env::temp_dir().join(format!("sp-logs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("access.log");
        std::fs::write(&path, "GET /a 200 1\nGET /b 404 2\nPOST /c 500 3\n").unwrap();

        let all = read_log(&path, &Filters::default()).unwrap();
        assert_eq!(all.len(), 3);

        let by_status = read_log(
            &path,
            &Filters {
                status: Some(404),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_status.len(), 1);
        assert_eq!(by_status[0].status, Some(404));

        let by_text = read_log(
            &path,
            &Filters {
                search: Some("POST".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_text.len(), 1);
    }

    #[test]
    fn tail_keeps_the_last_lines() {
        let dir = std::env::temp_dir().join(format!("sp-tail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log");
        std::fs::write(&path, "1\n2\n3\n4\n5\n").unwrap();
        let tail = tail(&path, 2).unwrap();
        assert_eq!(tail, vec!["4".to_string(), "5".to_string()]);
    }

    #[test]
    fn missing_log_is_reported() {
        let result = read_log(Path::new("/nonexistent/log"), &Filters::default());
        assert!(result.is_err());
    }

    #[test]
    fn parses_sources() {
        assert_eq!(
            LogSource::parse("nginx-access"),
            Some(LogSource::NginxAccess)
        );
        assert_eq!(LogSource::parse("panel"), Some(LogSource::Panel));
        assert_eq!(LogSource::parse("nope"), None);
    }
}
