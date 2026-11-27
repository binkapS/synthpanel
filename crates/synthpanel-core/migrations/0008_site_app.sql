-- Node.js / Python site applications.
--
-- A site with a row here is served by an application process listening on
-- `app_port` on loopback instead of by the plain static/PHP vhost. The core
-- supervises the process (systemd unit, or a nohup process when systemd is
-- unavailable) and stores its configuration in this table. `app_command` may
-- carry environment assignments, for example `PORT=3000 npm start --production`.

CREATE TABLE IF NOT EXISTS site_apps (
    site_id     TEXT PRIMARY KEY REFERENCES sites(id) ON DELETE CASCADE,
    app_type    TEXT NOT NULL CHECK (app_type IN ('node', 'python')),
    app_port    INTEGER NOT NULL CHECK (app_port BETWEEN 1 AND 65535),
    app_command TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_site_apps_type ON site_apps(app_type);
