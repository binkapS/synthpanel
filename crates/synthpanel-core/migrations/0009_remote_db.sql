-- CloudPanel parity wave 3: external / remote database servers.
--
-- These are connection definitions for databases that live outside the panel
-- (RDS, Aurora, a managed Postgres instance, or any remote MySQL/Postgres
-- server). No connection is opened here: the panel only keeps the credential
-- so other operations can use it later.
--
-- Passwords are NEVER stored in clear text. `password_encrypted` holds
-- base64(nonce || ChaCha20-Poly1305 ciphertext), keyed by the persistent
-- secret.key created at startup by synthpanel-core and with the record id as
-- additional authenticated data. Without that key the column is unreadable,
-- and swapping a record's id invalidates the ciphertext.

CREATE TABLE IF NOT EXISTS remote_databases (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL UNIQUE COLLATE NOCASE,
    engine            TEXT NOT NULL CHECK (engine IN
                          ('mysql', 'mariadb', 'postgres',
                           'aurora-mysql', 'aurora-postgresql')),
    host              TEXT NOT NULL,
    port              INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    database_name     TEXT NOT NULL,
    username          TEXT NOT NULL,
    password_encrypted TEXT NOT NULL,
    tls               INTEGER NOT NULL DEFAULT 0 CHECK (tls IN (0, 1)),
    created_at        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_remote_databases_engine
    ON remote_databases(engine);
