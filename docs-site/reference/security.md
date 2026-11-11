# Security model

## Trust boundary

Only `synthpanel-core` runs as root. It listens on a Unix socket, not TCP, and
requires the shared token from `/etc/synthpanel/ipc-token` on every request.
`synthpanel-web` runs as the unprivileged `synthpanel` user and cannot mutate
system state except through the core's typed API.

## Authentication

- Argon2id password hashes
- Optional TOTP two-factor authentication
- In-memory sessions with an HttpOnly, SameSite cookie
- CSRF double-submit token for browser mutations
- Bearer API keys for scripts, stored as SHA-256 hashes with scopes

## Change safety

Every config change goes through render → lint → backup → write → validate →
reload. Validation failures restore the previous file automatically.

## Backups

Archives can be encrypted with ChaCha20-Poly1305. Update archives are verified
against an Ed25519 signature and a SHA-256 checksum before the binaries are
replaced; a failed health check restores the previous binaries.

## Reporting

Email `security@synthpanel.io` for private disclosure. Please do not open a
public issue for a vulnerability.
