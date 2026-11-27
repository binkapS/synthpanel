-- Restrict API keys to specific sites.
--
-- A row grants an API key access to one site. No token material is stored
-- here: `id` is the API key's UUID, while the key's hash lives in `api_keys`.
-- An empty assignment means the key is not restricted to any particular site
-- (the legacy behaviour); a non-empty one limits the key to those sites in
-- addition to the owner's own site assignments.

CREATE TABLE IF NOT EXISTS api_key_sites (
    id      TEXT NOT NULL REFERENCES api_keys(id) ON DELETE CASCADE,
    site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    PRIMARY KEY (id, site_id)
);

CREATE INDEX IF NOT EXISTS idx_api_key_sites_id ON api_key_sites(id);
CREATE INDEX IF NOT EXISTS idx_api_key_sites_site ON api_key_sites(site_id);
