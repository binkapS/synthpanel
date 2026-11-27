-- Backup exclude patterns.
--
-- A row scoped to a site holds that site's patterns; the single row with the
-- empty site id holds the global patterns applied to every backup. Patterns
-- are stored as a JSON array of relative paths/globs and are validated by the
-- core before they are written or used while archiving site document roots.
-- An empty array means no excludes, preserving the historical behaviour.

CREATE TABLE IF NOT EXISTS backup_excludes (
    site_id    TEXT PRIMARY KEY, -- '' is the global scope
    patterns   TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL
);
