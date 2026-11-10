-- SynthPanel initial schema.
-- Tables follow PLAN.md §5.5. Phase 0 uses settings, users, sessions,
-- audit_log and metrics_*; the remaining tables are created now so later
-- phases only add data, not migrations.

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS settings (
    key        TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
    id            TEXT PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    name          TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    totp_secret   TEXT,
    totp_enabled  INTEGER NOT NULL DEFAULT 0,
    role          TEXT NOT NULL DEFAULT 'admin',
    status        TEXT NOT NULL DEFAULT 'active',
    quota_sites   INTEGER NOT NULL DEFAULT 10,
    quota_dbs     INTEGER NOT NULL DEFAULT 10,
    last_login_at TEXT,
    created_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    ip         TEXT,
    user_agent TEXT,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_user   ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expiry ON sessions(expires_at);

CREATE TABLE IF NOT EXISTS sites (
    id            TEXT PRIMARY KEY,
    domain        TEXT NOT NULL UNIQUE,
    aliases       TEXT NOT NULL DEFAULT '[]',
    document_root TEXT NOT NULL,
    template      TEXT NOT NULL DEFAULT 'php',
    php_version   TEXT,
    pool_user     TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'active',
    ssl_mode      TEXT NOT NULL DEFAULT 'none',
    ssl_cert_id   TEXT,
    owner_id      TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sites_owner ON sites(owner_id);

CREATE TABLE IF NOT EXISTS site_nginx (
    site_id    TEXT PRIMARY KEY REFERENCES sites(id) ON DELETE CASCADE,
    directives TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS site_php (
    site_id       TEXT PRIMARY KEY REFERENCES sites(id) ON DELETE CASCADE,
    overrides_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS databases (
    id         TEXT PRIMARY KEY,
    engine     TEXT NOT NULL,
    name       TEXT NOT NULL,
    owner_id   TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    UNIQUE(engine, name)
);

CREATE TABLE IF NOT EXISTS db_users (
    id                TEXT PRIMARY KEY,
    database_id       TEXT NOT NULL REFERENCES databases(id) ON DELETE CASCADE,
    username          TEXT NOT NULL,
    password_encrypted TEXT NOT NULL,
    privileges        TEXT NOT NULL DEFAULT 'ALL',
    created_at        TEXT NOT NULL,
    UNIQUE(database_id, username)
);

CREATE TABLE IF NOT EXISTS ssl_certs (
    id         TEXT PRIMARY KEY,
    site_id    TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    cert_path  TEXT NOT NULL,
    key_path   TEXT NOT NULL,
    expires_at TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS firewall_rules (
    id         TEXT PRIMARY KEY,
    port       TEXT NOT NULL,
    proto      TEXT NOT NULL DEFAULT 'tcp',
    source     TEXT NOT NULL DEFAULT 'any',
    action     TEXT NOT NULL DEFAULT 'allow',
    comment    TEXT NOT NULL DEFAULT '',
    enabled    INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ssh_keys (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label       TEXT NOT NULL,
    public_key  TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS api_keys (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    prefix      TEXT NOT NULL,
    hash        TEXT NOT NULL,
    scopes_json TEXT NOT NULL DEFAULT '[]',
    last_used_at TEXT,
    created_at  TEXT NOT NULL
);

-- The autoincrement id is the monotonic sequence number (PLAN §5.10).
CREATE TABLE IF NOT EXISTS audit_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_id     TEXT,
    actor_name   TEXT,
    action       TEXT NOT NULL,
    target       TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    ip           TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_log(created_at);
CREATE INDEX IF NOT EXISTS idx_audit_actor    ON audit_log(actor_id);
CREATE INDEX IF NOT EXISTS idx_audit_action   ON audit_log(action);

CREATE TABLE IF NOT EXISTS backups (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    status      TEXT NOT NULL,
    size        INTEGER NOT NULL DEFAULT 0,
    location    TEXT NOT NULL,
    schedule_id TEXT,
    error       TEXT,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS backup_sched (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    cron             TEXT NOT NULL,
    destination_json TEXT NOT NULL,
    retention_json   TEXT NOT NULL,
    enabled          INTEGER NOT NULL DEFAULT 1,
    created_at       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cron_jobs (
    id           TEXT PRIMARY KEY,
    site_id      TEXT REFERENCES sites(id) ON DELETE CASCADE,
    user         TEXT NOT NULL,
    expression   TEXT NOT NULL,
    command      TEXT NOT NULL,
    last_run_at  TEXT,
    last_status  TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cron_site ON cron_jobs(site_id);

CREATE TABLE IF NOT EXISTS metrics_raw (
    ts        INTEGER NOT NULL,
    host      TEXT NOT NULL,
    cpu       REAL NOT NULL,
    load1     REAL NOT NULL,
    mem_used  INTEGER NOT NULL,
    mem_total INTEGER NOT NULL DEFAULT 0,
    swap_used INTEGER NOT NULL,
    swap_total INTEGER NOT NULL DEFAULT 0,
    disk_used REAL NOT NULL,
    disk_total INTEGER NOT NULL DEFAULT 0,
    net_rx    INTEGER NOT NULL,
    net_tx    INTEGER NOT NULL,
    PRIMARY KEY (ts, host)
);

CREATE TABLE IF NOT EXISTS metrics_5m (
    ts        INTEGER PRIMARY KEY,
    cpu       REAL NOT NULL,
    load1     REAL NOT NULL,
    mem_used  REAL NOT NULL,
    swap_used REAL NOT NULL,
    disk_used REAL NOT NULL,
    net_rx    REAL NOT NULL,
    net_tx    REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS metrics_1h (
    ts        INTEGER PRIMARY KEY,
    cpu       REAL NOT NULL,
    load1     REAL NOT NULL,
    mem_used  REAL NOT NULL,
    swap_used REAL NOT NULL,
    disk_used REAL NOT NULL,
    net_rx    REAL NOT NULL,
    net_tx    REAL NOT NULL
);
