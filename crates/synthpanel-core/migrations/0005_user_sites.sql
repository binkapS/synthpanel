-- CloudPanel parity: per-site user scoping.
--
-- A row grants a user access to one site. Admin/operator bypass is decided by
-- the web authorization layer, so the core only stores and checks the mapping.

CREATE TABLE IF NOT EXISTS user_sites (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, site_id)
);

CREATE INDEX IF NOT EXISTS idx_user_sites_user ON user_sites(user_id);
CREATE INDEX IF NOT EXISTS idx_user_sites_site ON user_sites(site_id);
