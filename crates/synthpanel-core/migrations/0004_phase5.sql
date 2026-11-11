-- Phase 5: alerting and Git deployments.

CREATE TABLE IF NOT EXISTS alert_channels (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL,
    config     TEXT NOT NULL DEFAULT '{}',
    enabled    INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS alert_rules (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    event         TEXT NOT NULL,
    severity      TEXT NOT NULL DEFAULT 'warning',
    channel_id    TEXT NOT NULL,
    enabled       INTEGER NOT NULL DEFAULT 1,
    cooldown_secs INTEGER NOT NULL DEFAULT 900,
    created_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_alert_rules_event ON alert_rules(event);

CREATE TABLE IF NOT EXISTS alerts (
    id           TEXT PRIMARY KEY,
    rule_id      TEXT,
    event        TEXT NOT NULL,
    severity     TEXT NOT NULL DEFAULT 'warning',
    message      TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'pending',
    error        TEXT,
    created_at   TEXT NOT NULL,
    delivered_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_alerts_created ON alerts(created_at);

CREATE TABLE IF NOT EXISTS deployments (
    id           TEXT PRIMARY KEY,
    site_id      TEXT NOT NULL,
    repo_url     TEXT NOT NULL,
    branch       TEXT NOT NULL DEFAULT 'main',
    target_subdir TEXT NOT NULL DEFAULT '',
    build_command TEXT NOT NULL DEFAULT '',
    deploy_user  TEXT NOT NULL DEFAULT 'www-data',
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_deployments_site ON deployments(site_id);

CREATE TABLE IF NOT EXISTS deploy_runs (
    id            TEXT PRIMARY KEY,
    deployment_id TEXT NOT NULL,
    site_id       TEXT NOT NULL,
    commit_sha    TEXT NOT NULL DEFAULT '',
    branch        TEXT NOT NULL DEFAULT '',
    status        TEXT NOT NULL DEFAULT 'ok',
    message       TEXT NOT NULL DEFAULT '',
    output        TEXT NOT NULL DEFAULT '',
    started_at    TEXT NOT NULL,
    finished_at   TEXT
);

CREATE INDEX IF NOT EXISTS idx_deploy_runs_started ON deploy_runs(started_at);
