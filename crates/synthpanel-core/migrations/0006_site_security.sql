-- Site-level security controls.
--
-- Sites without a row here keep the permissive defaults. The password column
-- holds an htpasswd line (`user:{SHA}<base64>`) that the core writes to the
-- per-site auth_basic file; it is never exposed by site_security.get.

CREATE TABLE IF NOT EXISTS site_security (
    site_id                  TEXT PRIMARY KEY REFERENCES sites(id) ON DELETE CASCADE,
    ip_blocks                TEXT NOT NULL DEFAULT '[]',
    bot_blocks               TEXT NOT NULL DEFAULT '[]',
    basic_auth_enabled       INTEGER NOT NULL DEFAULT 0,
    basic_auth_user          TEXT NOT NULL DEFAULT '',
    basic_auth_password_hash TEXT NOT NULL DEFAULT '',
    cloudflare_only          INTEGER NOT NULL DEFAULT 0,
    updated_at               TEXT NOT NULL
);
