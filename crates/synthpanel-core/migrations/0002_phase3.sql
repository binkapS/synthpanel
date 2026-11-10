-- Phase 3 additions: richer backup records, cron run history and API-key
-- usage metadata.

ALTER TABLE backups ADD COLUMN name        TEXT    NOT NULL DEFAULT '';
ALTER TABLE backups ADD COLUMN site_id     TEXT;
ALTER TABLE backups ADD COLUMN checksum    TEXT    NOT NULL DEFAULT '';
ALTER TABLE backups ADD COLUMN encrypted   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE backups ADD COLUMN entries_json TEXT   NOT NULL DEFAULT '[]';
ALTER TABLE backups ADD COLUMN duration_ms INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_backups_created ON backups(created_at);

CREATE TABLE IF NOT EXISTS cron_runs (
    id          TEXT PRIMARY KEY,
    job_id      TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    started_at  TEXT NOT NULL,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    exit_code   INTEGER NOT NULL DEFAULT 0,
    output      TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_cron_runs_job ON cron_runs(job_id, started_at);
