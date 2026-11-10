//! System metrics collection and storage.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use std::time::Duration;
use tracing::warn;

const RAW_RETENTION_SECS: i64 = 24 * 3600;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Snapshot {
    pub ts: i64,
    pub host: String,
    pub cpu: f32,
    pub load1: f32,
    pub mem_used: i64,
    pub mem_total: i64,
    pub swap_used: i64,
    pub swap_total: i64,
    pub disk_used: f64,
    pub disk_total: i64,
    pub net_rx: i64,
    pub net_tx: i64,
}

pub struct Collector {
    sys: sysinfo::System,
    disks: sysinfo::Disks,
    networks: sysinfo::Networks,
    pool: SqlitePool,
    host: String,
}

impl Collector {
    pub fn new(pool: SqlitePool) -> Self {
        let host = sysinfo::System::host_name().unwrap_or_else(|| "localhost".to_string());
        Self {
            sys: sysinfo::System::new_all(),
            disks: sysinfo::Disks::new_with_refreshed_list(),
            networks: sysinfo::Networks::new_with_refreshed_list(),
            pool,
            host,
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// Sample the current system state.
    pub fn sample(&mut self) -> Snapshot {
        self.sys.refresh_all();
        self.disks.refresh_list();
        self.networks.refresh_list();
        let cpu = self.sys.global_cpu_usage();
        let load1 = sysinfo::System::load_average().one as f32;
        let mem_used = self.sys.used_memory() as i64;
        let mem_total = self.sys.total_memory() as i64;
        let swap_used = self.sys.used_swap() as i64;
        let swap_total = self.sys.total_swap() as i64;
        let (disk_used, disk_total) = disk_usage(&self.disks);
        let (net_rx, net_tx) = network_totals(&self.networks);

        Snapshot {
            ts: Utc::now().timestamp(),
            host: self.host.clone(),
            cpu,
            load1,
            mem_used,
            mem_total,
            swap_used,
            swap_total,
            disk_used,
            disk_total,
            net_rx,
            net_tx,
        }
    }

    /// Run the periodic collector until the process exits.
    pub async fn run(mut self) {
        let mut interval = tokio::time::interval(SAMPLE_INTERVAL);
        loop {
            interval.tick().await;
            let snapshot = self.sample();
            if let Err(error) = store(&self.pool, &snapshot).await {
                warn!(%error, "failed to store metrics sample");
            }
        }
    }

    /// Prune raw samples older than the retention window.
    pub async fn prune(&self) -> Result<u64> {
        prune(&self.pool, &self.host).await
    }
}

fn disk_usage(disks: &sysinfo::Disks) -> (f64, i64) {
    let mut total = 0u64;
    let mut available = 0u64;
    for disk in disks.list() {
        total += disk.total_space();
        available += disk.available_space();
    }
    if total == 0 {
        (0.0, 0)
    } else {
        let used = total.saturating_sub(available);
        ((used as f64 / total as f64) * 100.0, total as i64)
    }
}

fn network_totals(networks: &sysinfo::Networks) -> (i64, i64) {
    let mut rx = 0i64;
    let mut tx = 0i64;
    for data in networks.list().values() {
        rx += data.total_received() as i64;
        tx += data.total_transmitted() as i64;
    }
    (rx, tx)
}

pub async fn store(pool: &SqlitePool, snapshot: &Snapshot) -> Result<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO metrics_raw
            (ts, host, cpu, load1, mem_used, mem_total, swap_used, swap_total, disk_used,
             disk_total, net_rx, net_tx)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(snapshot.ts)
    .bind(&snapshot.host)
    .bind(snapshot.cpu)
    .bind(snapshot.load1)
    .bind(snapshot.mem_used)
    .bind(snapshot.mem_total)
    .bind(snapshot.swap_used)
    .bind(snapshot.swap_total)
    .bind(snapshot.disk_used)
    .bind(snapshot.disk_total)
    .bind(snapshot.net_rx)
    .bind(snapshot.net_tx)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn latest(pool: &SqlitePool, host: &str) -> Result<Option<Snapshot>> {
    let row = sqlx::query_as::<_, Snapshot>(
        "SELECT * FROM metrics_raw WHERE host = ? ORDER BY ts DESC LIMIT 1",
    )
    .bind(host)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn range(pool: &SqlitePool, host: &str, since_ts: i64) -> Result<Vec<Snapshot>> {
    let rows = sqlx::query_as::<_, Snapshot>(
        "SELECT * FROM metrics_raw WHERE host = ? AND ts >= ? ORDER BY ts ASC",
    )
    .bind(host)
    .bind(since_ts)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn prune(pool: &SqlitePool, host: &str) -> Result<u64> {
    let cutoff = Utc::now().timestamp() - RAW_RETENTION_SECS;
    let result = sqlx::query("DELETE FROM metrics_raw WHERE host = ? AND ts < ?")
        .bind(host)
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Parse a human range such as `30m`, `24h` or `7d` into a unix timestamp.
pub fn since_from_range(range: &str) -> Option<i64> {
    let range = range.trim();
    if range.is_empty() {
        return None;
    }
    let (num, unit) = range.split_at(range.len() - 1);
    let num: i64 = num.parse().ok()?;
    let secs = match unit {
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        "w" => num * 604_800,
        _ => return None,
    };
    Some(Utc::now().timestamp() - secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn parses_supported_ranges() {
        let now = Utc::now().timestamp();
        let ts = since_from_range("24h").unwrap();
        assert!((now - ts - 86400).abs() <= 2);
        assert!(since_from_range("bogus").is_none());
        assert!(since_from_range("").is_none());
    }

    #[tokio::test]
    async fn store_and_read_back() {
        let mut path = std::env::temp_dir();
        path.push(format!("sp-metrics-{}.db", uuid::Uuid::new_v4()));
        let pool = db::connect(&path).await.unwrap();
        db::migrate(&pool).await.unwrap();

        let snap = Snapshot {
            ts: Utc::now().timestamp(),
            host: "testhost".into(),
            cpu: 12.5,
            load1: 0.75,
            mem_used: 1024,
            mem_total: 4096,
            swap_used: 0,
            swap_total: 0,
            disk_used: 42.0,
            disk_total: 1000,
            net_rx: 100,
            net_tx: 200,
        };
        store(&pool, &snap).await.unwrap();
        let latest = latest(&pool, "testhost").await.unwrap().unwrap();
        assert_eq!(latest.cpu, 12.5);
        assert_eq!(latest.mem_used, 1024);
        assert_eq!(latest.mem_total, 4096);
        assert_eq!(latest.disk_used, 42.0);

        let rows = range(&pool, "testhost", snap.ts - 60).await.unwrap();
        assert_eq!(rows.len(), 1);
    }
}
