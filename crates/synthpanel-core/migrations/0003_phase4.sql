-- Phase 4: self-update history.

CREATE TABLE IF NOT EXISTS updates (
    id          TEXT PRIMARY KEY,
    from_version TEXT NOT NULL,
    to_version   TEXT NOT NULL,
    status       TEXT NOT NULL,
    backup_dir   TEXT NOT NULL DEFAULT '',
    error        TEXT,
    started_at   TEXT NOT NULL,
    finished_at  TEXT
);

CREATE INDEX IF NOT EXISTS idx_updates_started ON updates(started_at);
