-- Per-site SSH users.
--
-- System usernames are global on a host, so `username` is unique across all
-- sites rather than only within a site. The document root is stored with the
-- user so a user can still be cleaned up if its site record is removed first.

CREATE TABLE IF NOT EXISTS site_ssh_users (
    id            TEXT PRIMARY KEY,
    site_id       TEXT NOT NULL,
    username      TEXT NOT NULL UNIQUE,
    document_root TEXT NOT NULL,
    public_key    TEXT NOT NULL,
    created_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_site_ssh_users_site ON site_ssh_users(site_id);
