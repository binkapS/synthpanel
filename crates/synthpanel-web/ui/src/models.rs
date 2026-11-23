// Copyright (C) 2026 Binkap S
// SPDX-License-Identifier: AGPL-3.0-or-later

//! JSON → Slint struct conversion helpers for every page.

use crate::{
    ApiKeyRow, AlertRow, AlertRuleRow, AuditEntry, BackupRow, ChannelRow, CronRow, CronRunRow,
    DatabaseRow, DeployRunRow, DeploymentRow, EngineRow, FileRow, JailRow,
    LogLine, LogSource, MetricSnapshot, PanelCertificate, PhpLink, PhpVersionRow, ScheduleRow,
    Service, Session, SiteDetail, SiteRow, SshKeyRow, TerminalLine, UserRow,
};
use serde_json::Value;

fn str_(value: &Value, key: &str) -> slint::SharedString {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
        .into()
}

fn opt_str(value: &Value, key: &str) -> slint::SharedString {
    str_(value, key)
}

/// Wrap a vector in the `ModelRc` used by generated Slint structs.
fn model<T: Clone + 'static>(items: Vec<T>) -> slint::ModelRc<T> {
    slint::ModelRc::new(slint::VecModel::from(items))
}

fn num(value: &Value, key: &str) -> f32 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

fn int(value: &Value, key: &str) -> i32 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0) as i32
}

fn bool_(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// Human readable byte size for the backup/file tables.
pub fn format_bytes(bytes: f32) -> String {
    if bytes <= 0.0 {
        return "—".to_string();
    }
    let value = bytes as f64;
    let (scaled, unit) = if value >= 1_073_741_824.0 {
        (value / 1_073_741_824.0, "GB")
    } else if value >= 1_048_576.0 {
        (value / 1_048_576.0, "MB")
    } else if value >= 1024.0 {
        (value / 1024.0, "KB")
    } else {
        (value, "B")
    };
    format!("{scaled:.1} {unit}")
}

/// Build an SVG path (300×100 viewbox) for the dashboard sparkline.
pub fn sparkline_path(points: &[f32]) -> String {
    if points.len() < 2 {
        return String::new();
    }
    let mut min = points[0];
    let mut max = points[0];
    for value in points {
        min = min.min(*value);
        max = max.max(*value);
    }
    if (max - min).abs() < f32::EPSILON {
        return "M0,50 L300,50".to_string();
    }
    let span = (points.len() - 1) as f32;
    let mut path = String::from("M0,50");
    for (index, value) in points.iter().enumerate() {
        let x = index as f32 * 300.0 / span;
        let y = 100.0 - (*value - min) * 100.0 / (max - min);
        path.push_str(&format!(" L{x:.1},{y:.1}"));
    }
    path
}

fn str_array(value: &Value, key: &str) -> Vec<slint::SharedString> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(|text| slint::SharedString::from(text)))
                .collect()
        })
        .unwrap_or_default()
}

fn objects<'a>(value: &'a Value, key: &str) -> Vec<&'a Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter(|item| item.is_object()).collect())
        .unwrap_or_default()
}

pub fn user_session(user: &Value) -> Session {
    Session {
        name: opt_str(user, "name"),
        email: opt_str(user, "email"),
        role: opt_str(user, "role"),
        totp_enabled: bool_(user, "totp_enabled"),
    }
}

/// Convert a metrics snapshot (the REST envelope is a bare object; the
/// WebSocket frame carries it under `"metrics"`).
///
/// Units as produced by the core: memory in bytes, disk used as a 0-100
/// percentage, disk total in bytes, network counters as byte totals.
pub fn metric_snapshot(body: &Value) -> MetricSnapshot {
    let mut snapshot = MetricSnapshot::default();
    if body.get("cpu").is_none() {
        return snapshot;
    }
    snapshot.host = opt_str(body, "host");
    snapshot.cpu = num(body, "cpu");
    snapshot.load1 = num(body, "load1");
    snapshot.mem_used = num(body, "mem_used") / 1048576.0; // bytes → MiB
    snapshot.mem_total = num(body, "mem_total") / 1048576.0;
    snapshot.disk_used = num(body, "disk_used"); // already a percentage
    snapshot.disk_total = num(body, "disk_total") / 1073741824.0; // bytes → GiB
    snapshot.net_rx = num(body, "net_rx"); // byte total, not a rate
    snapshot.net_tx = num(body, "net_tx");
    snapshot.ts = int(body, "ts");
    snapshot
}

/// Network rate in bytes/second between two consecutive snapshots.
pub fn network_rate(previous: &Value, current: &Value) -> (f32, f32) {
    let previous_ts = int(previous, "ts") as i64;
    let current_ts = int(current, "ts") as i64;
    let seconds = ((current_ts - previous_ts).max(1)) as f32;
    let rx = (num(current, "net_rx") - num(previous, "net_rx")) / seconds;
    let tx = (num(current, "net_tx") - num(previous, "net_tx")) / seconds;
    (rx.max(0.0), tx.max(0.0))
}

pub fn to_services(body: &Value) -> Vec<Service> {
    service_items(body)
        .into_iter()
        .map(|item| Service {
            unit: str_(item, "unit"),
            status: str_(item, "status"),
        })
        .collect()
}

/// The REST envelope wraps the array as `{"services": [...]}`; the
/// WebSocket snapshot carries the bare array.
fn service_items(body: &Value) -> Vec<&Value> {
    if body.is_array() {
        return body
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item.is_object())
            .collect();
    }
    objects(body, "services")
}

pub fn to_audit(body: &Value) -> Vec<AuditEntry> {
    objects(body, "entries")
        .into_iter()
        .map(|item| AuditEntry {
            action: str_(item, "action"),
            target: opt_str(item, "target"),
            actor: opt_str(item, "actor"),
            ip: opt_str(item, "ip"),
            when: opt_str(item, "created_at"),
        })
        .collect()
}

pub fn to_sites(body: &Value) -> Vec<SiteRow> {
    objects(body, "sites")
        .into_iter()
        .map(|item| SiteRow {
            id: str_(item, "id"),
            domain: str_(item, "domain"),
            template: str_(item, "template"),
            php_version: opt_str(item, "php_version"),
            ssl_mode: str_(item, "ssl_mode"),
            status: str_(item, "status"),
            document_root: opt_str(item, "document_root"),
        })
        .collect()
}

pub fn to_site_detail(site: &Value) -> SiteDetail {
    SiteDetail {
        id: str_(site, "id"),
        domain: str_(site, "domain"),
        aliases: opt_str(site, "aliases"),
        document_root: opt_str(site, "document_root"),
        template: str_(site, "template"),
        php_version: opt_str(site, "php_version"),
        pool_user: opt_str(site, "pool_user"),
        status: str_(site, "status"),
        ssl_mode: str_(site, "ssl_mode"),
        created: opt_str(site, "created_at"),
    }
}

pub fn to_ssl(body: &Value) -> crate::SslStatus {
    crate::SslStatus {
        domain: opt_str(body, "domain"),
        ssl_mode: str_(body, "ssl_mode"),
        expires: opt_str(body, "expires_at"),
        expiring_soon: bool_(body, "expiring_soon"),
    }
}

pub fn to_databases(body: &Value) -> Vec<DatabaseRow> {
    objects(body, "databases")
        .into_iter()
        .map(|item| DatabaseRow {
            id: str_(item, "id"),
            name: str_(item, "name"),
            engine: str_(item, "engine"),
        })
        .collect()
}

pub fn to_engines(body: &Value) -> Vec<EngineRow> {
    objects(body, "engines")
        .into_iter()
        .map(|item| EngineRow {
            engine: str_(item, "engine"),
            reachable: bool_(item, "server_reachable"),
            client: bool_(item, "client_available"),
        })
        .collect()
}

pub fn to_php_versions(body: &Value) -> Vec<PhpVersionRow> {
    objects(body, "installed")
        .into_iter()
        .map(|item| {
            let version = str_(item, "version");
            let sites = objects(body, "sites")
                .into_iter()
                .filter(|site| site.get("version").and_then(Value::as_str) == Some(version.as_str()))
                .map(|site| PhpLink {
                    id: str_(site, "id"),
                    domain: str_(site, "domain"),
                })
                .collect();
            PhpVersionRow {
                version,
                binary: opt_str(item, "binary"),
                pool_dir: str_(item, "pool_dir"),
                active: bool_(item, "active"),
                pools: model(str_array(item, "pools")),
                sites: model(sites),
            }
        })
        .collect()
}

pub fn to_jails(body: &Value) -> Vec<JailRow> {
    objects(body, "jails")
        .into_iter()
        .map(|item| JailRow {
            name: str_(item, "name"),
            status: str_(item, "status"),
            banned_now: int(item, "currently_banned"),
            banned_total: int(item, "total_banned"),
        })
        .collect()
}

pub fn to_ssh_keys(body: &Value) -> Vec<SshKeyRow> {
    objects(body, "keys")
        .into_iter()
        .map(|item| SshKeyRow {
            id: str_(item, "id"),
            label: str_(item, "label"),
            fingerprint: opt_str(item, "fingerprint"),
        })
        .collect()
}

pub fn to_backups(body: &Value) -> Vec<BackupRow> {
    objects(body, "backups")
        .into_iter()
        .map(|item| BackupRow {
            id: str_(item, "id"),
            name: str_(item, "name"),
            kind: str_(item, "kind"),
            status: str_(item, "status"),
            size: num(item, "size"),
            size_label: format_bytes(num(item, "size")).into(),
            created: opt_str(item, "created_at"),
            encrypted: bool_(item, "encrypted"),
            error: opt_str(item, "error"),
        })
        .collect()
}

pub fn to_schedules(body: &Value) -> Vec<ScheduleRow> {
    objects(body, "schedules")
        .into_iter()
        .map(|item| ScheduleRow {
            id: str_(item, "id"),
            name: str_(item, "name"),
            cron: str_(item, "cron"),
            enabled: bool_(item, "enabled"),
        })
        .collect()
}

pub fn to_cron_jobs(body: &Value) -> Vec<CronRow> {
    objects(body, "jobs")
        .into_iter()
        .map(|item| CronRow {
            id: str_(item, "id"),
            expression: str_(item, "expression"),
            command: str_(item, "command"),
            user: str_(item, "user"),
            next_run: opt_str(item, "next_run"),
            last_run: opt_str(item, "last_run_at"),
            last_status: opt_str(item, "last_status"),
            runs: Default::default(),
        })
        .collect()
}

pub fn to_cron_runs(body: &Value) -> Vec<CronRunRow> {
    objects(body, "runs")
        .into_iter()
        .map(|item| CronRunRow {
            id: str_(item, "id"),
            started: opt_str(item, "started_at"),
            duration: int(item, "duration_ms"),
            exit_code: int(item, "exit_code"),
            output: str_(item, "output"),
        })
        .collect()
}

pub fn to_deploy_runs(body: &Value) -> Vec<DeployRunRow> {
    objects(body, "runs")
        .into_iter()
        .map(|item| DeployRunRow {
            id: str_(item, "id"),
            commit: str_(item, "commit_sha"),
            status: str_(item, "status"),
            message: str_(item, "message"),
            output: str_(item, "output"),
            started: opt_str(item, "started_at"),
        })
        .collect()
}

pub fn to_log_sources(body: &Value) -> Vec<LogSource> {
    // /api/v1/logs/sources wraps the array as {"sources": [...]}; the IPC
    // call used by log tailing returns the bare array.
    let source = if body.is_array() {
        body.clone()
    } else {
        body.get("sources").cloned().unwrap_or(serde_json::Value::Array(Vec::new()))
    };
    source
        .as_array()
        .map(|items| items.iter().filter(|item| item.is_object()).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|item| LogSource {
            id: str_(item, "id"),
            label: str_(item, "label"),
            requires_site: bool_(item, "requires_site"),
            available: bool_(item, "available"),
        })
        .collect()
}

pub fn to_log_lines(body: &Value) -> Vec<LogLine> {
    objects(body, "lines")
        .into_iter()
        .map(|item| LogLine {
            text: format!(
                "{} {}",
                opt_str(item, "timestamp"),
                str_(item, "message")
            )
            .into(),
            level: opt_str(item, "level"),
        })
        .collect()
}

pub fn to_file_rows(body: &Value) -> Vec<FileRow> {
    // The listing endpoint returns a bare `Listing` object whose `entries`
    // array sits next to `path` and `parent`.
    let source = body.get("entries").cloned().unwrap_or_else(|| body.clone());
    objects(&source, "entries")
        .into_iter()
        .map(|item| FileRow {
            name: str_(item, "name"),
            path: str_(item, "path"),
            is_directory: bool_(item, "is_directory"),
            size: num(item, "size"),
            size_label: format_bytes(num(item, "size")).into(),
            mode: str_(item, "mode"),
            owner: str_(item, "owner"),
            modified: opt_str(item, "modified"),
        })
        .collect()
}

pub fn to_deployments(body: &Value) -> Vec<DeploymentRow> {
    objects(body, "deployments")
        .into_iter()
        .map(|item| DeploymentRow {
            id: str_(item, "id"),
            repo: str_(item, "repo_url"),
            branch: str_(item, "branch"),
            site: str_(item, "site_id"),
            subdir: opt_str(item, "target_subdir"),
            build: opt_str(item, "build_command"),
            enabled: bool_(item, "enabled"),
            runs: Default::default(),
        })
        .collect()
}

pub fn to_channels(body: &Value) -> Vec<ChannelRow> {
    objects(body, "channels")
        .into_iter()
        .map(|item| ChannelRow {
            id: str_(item, "id"),
            name: str_(item, "name"),
            kind: str_(item, "kind"),
            created: opt_str(item, "created_at"),
        })
        .collect()
}

pub fn to_rules(body: &Value) -> Vec<AlertRuleRow> {
    objects(body, "rules")
        .into_iter()
        .map(|item| AlertRuleRow {
            id: str_(item, "id"),
            name: str_(item, "name"),
            event: str_(item, "event"),
            severity: str_(item, "severity"),
            channel: str_(item, "channel_id"),
            cooldown: int(item, "cooldown_secs"),
        })
        .collect()
}

pub fn to_alerts(body: &Value) -> Vec<AlertRow> {
    objects(body, "alerts")
        .into_iter()
        .map(|item| AlertRow {
            id: str_(item, "id"),
            event: str_(item, "event"),
            severity: str_(item, "severity"),
            message: str_(item, "message"),
            status: str_(item, "status"),
            created: opt_str(item, "created_at"),
            error: opt_str(item, "error"),
        })
        .collect()
}

pub fn to_users(body: &Value) -> Vec<UserRow> {
    objects(body, "users")
        .into_iter()
        .map(|item| UserRow {
            id: str_(item, "id"),
            email: str_(item, "email"),
            name: str_(item, "name"),
            role: str_(item, "role"),
            status: str_(item, "status"),
            totp: if bool_(item, "totp_enabled") { "on" } else { "off" }.to_string().into(),
            last_login: opt_str(item, "last_login_at"),
        })
        .collect()
}

pub fn to_api_keys(body: &Value) -> Vec<ApiKeyRow> {
    objects(body, "keys")
        .into_iter()
        .map(|item| ApiKeyRow {
            id: str_(item, "id"),
            name: str_(item, "name"),
            prefix: opt_str(item, "prefix"),
            // The API returns scopes as a JSON array (or a comma list in the
            // create request); flatten it for the row display.
            scopes: match item.get("scopes") {
                Some(serde_json::Value::Array(list)) => list
                    .iter()
                    .filter_map(|entry| entry.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
                    .join(", "),
                Some(serde_json::Value::String(text)) => text.to_string(),
                _ => String::new(),
            }
            .into(),
        })
        .collect()
}

pub fn to_panel_certificate(body: &Value) -> PanelCertificate {
    PanelCertificate {
        hostname: opt_str(body, "hostname"),
        path: opt_str(body, "path"),
        issuer: opt_str(body, "issuer"),
        self_signed: bool_(body, "self_signed"),
        expires: opt_str(body, "expires_at"),
        days_left: int(body, "days_left"),
        acme_installed: bool_(body, "acme_installed") || bool_(body, "exists"),
    }
}

pub fn terminal_output_line(output: &str) -> Vec<TerminalLine> {
    output
        .lines()
        .map(|line| TerminalLine {
            text: line.to_string().into(),
            kind: "output".into(),
        })
        .collect()
}
