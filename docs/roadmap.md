# Roadmap

A short, honest view of what is implemented in this tree and what remains,
aligned with the phases in [PLAN.md §10](../../PLAN.md#10-roadmap--milestones).

## Phase 0 — Foundations (complete)

- Workspace + CI (`cargo fmt`, `clippy`, `test`, `audit`, `deny`, frontend).
- `synthpanel-core`: SQLite schema + migrations, settings, users, audit log,
  system metrics collector, IPC server over a Unix socket with per-request
  token auth, best-effort service-status reporting.
- `synthpanel-web`: axum + rustls (self-signed on first start), argon2
  password hashing, in-memory session store, TOTP (RFC 6238) 2FA, CSRF
  double-submit, security headers, rate-limited login, embedded Svelte 5
  SPA, REST API v1.
- `synthpanel-cli`: `status`, `metrics`, `metrics:range`, `audit`, `settings`.
- `synthpanel-installer`: distro detection, package installation, user &
  directory layout, systemd wiring, firewall baseline, admin bootstrap.
- Svelte 5 + Vite SPA: login (with 2FA), dashboard, settings, users.

## Phase 1 — Core hosting (complete)

- Site model and repository (`sites` table + `site_nginx` / `site_php`).
- Nginx vhost rendering for the static / PHP / Node templates, with LE and
  custom-certificate TLS blocks, HTTP -> HTTPS redirect, and custom directive
  injection.
- PHP-FPM pool rendering with per-site socket, dedicated pool user, and
  `php.ini` overrides.
- Directive linter: rejects unbalanced braces and the directives the panel
  reserves (`root`, `listen`, `server_name`, `ssl_certificate*`).
- Apply pipeline: render -> lint -> backup -> write -> `nginx -t` /
  `php-fpm -t` -> reload -> automatic rollback when validation fails.
- SSL: Let's Encrypt issuance over HTTP-01 (driving acme.sh), certificate
  publishing into the stable `/etc/letsencrypt/live` layout, expiry parsing
  straight from the DER `notAfter` field, and an expiry status endpoint.
- Suspend / unsuspend a site, and the PHP version catalogue.
- WebSocket channels: `/ws/dashboard` pushes a metrics + services snapshot
  every 3 seconds, `/ws/logs` tails the panel log.
- REST API: `GET|POST /api/v1/sites`, `GET|PATCH|DELETE /api/v1/sites/:id`,
  `.../ssl/issue`, `.../ssl/status`, `.../suspend`, `.../unsuspend`,
  `GET /api/v1/php/versions`.
- CLI: `sites:list`, `sites:add`, `sites:show`, `sites:delete`,
  `users:list`, `users:create` (the colon naming matches PLAN §8.3).
- SPA: site list, site detail (SSL + suspend), dashboard with live metrics.

## Phase 2 — Data & security (complete)

- Database management for MariaDB/MySQL, PostgreSQL and Redis: the core
  provisions the database and its user on the server first and only then
  records it in the panel state, so the two never drift apart.
- Strict identifier validation for databases and usernames, with SQL built
  through quoted identifiers only.
- Generated credentials plus a ready-to-paste connection string and `.env`
  snippet per database.
- Engine health: client binary presence and server reachability per engine.
- Firewall: UFW rule modelling, validation (ports, CIDR sources) and rule
  application through the `ufw` CLI.
- fail2ban: jail listing with the current/total ban counters and unban.
- SSH keys: key-type validation, OpenSSH `SHA256:` fingerprints, and
  installation into the user's `authorized_keys`.
- REST API: `/api/v1/databases*` and `/api/v1/security/*`.
- CLI: `db:list`, `db:create`, `db:delete`, `security:firewall`,
  `security:fail2ban`, `security:ssh-keys`.
- SPA: database list with engine health, security page with the firewall
  form, fail2ban jails and SSH key management.

## Phase 3 — Operations

- Backups (local / S3 / SFTP) with restore and verification.
- Cron UI with a human-readable schedule builder and run logs.
- Log viewers with filters and tail-follow.
- File manager, API keys, and a CLI that covers 90% of the UI operations.

## Phase 4 — v1.0 hardening

- Self-update with signature verification and rollback.
- Packaging: `.deb` + apt repository.
- Playwright e2e, chaos tests, external security audit.
- Docs site, i18n scaffolding, accessibility pass.
