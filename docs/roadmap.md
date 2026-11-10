# Roadmap

A short, honest view of what is implemented in this tree and what remains,
aligned with the phases in [PLAN.md §10](../../PLAN.md#10-roadmap--milestones).

## Phase 0 — Foundations (this tree)

Implemented:

- Workspace + CI (`cargo fmt`, `clippy`, `test`, `audit`, `deny`, frontend).
- `synthpanel-core`: SQLite schema + migrations, settings, users, audit log,
  system metrics collector, IPC server over a Unix socket with per-request
  token auth, best-effort service-status reporting.
- `synthpanel-web`: axum + rustls (self-signed on first start), argon2
  password hashing, in-memory session store, TOTP (RFC 6238) 2FA, CSRF
  double-submit, security headers, rate-limited login, embedded Svelte 5
  SPA, REST API v1 (`auth`, `metrics`, `services`, `audit`, `settings`,
  `users`).
- `synthpanel-cli`: `status`, `metrics`, `metrics-range`, `audit`, `settings`.
- `synthpanel-installer`: distro detection, package installation, user &
  directory layout, systemd wiring, firewall baseline, admin bootstrap.
- Svelte 5 + Vite SPA: login (with 2FA), dashboard (CPU/memory sparklines,
  service list, audit feed), sites empty-state, settings, users.

## Phase 1 — Core hosting (in progress)

Implemented:

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
- REST API: `GET|POST /api/v1/sites`, `GET|PATCH|DELETE /api/v1/sites/:id`.
- CLI: `sites:list`, `sites:add`, `sites:show`, `sites:delete`,
  `users:list`, `users:create` (the colon naming matches PLAN §8.3).
- SPA: site list with status/SSL/PHP columns, create form, delete.
- End-to-end smoke test that logs in, creates a site, asserts the generated
  vhost on disk, rejects an invalid domain, deletes the site and checks the
  audit log.

Still to come in Phase 1: Let's Encrypt issuance and renewal, per-site PHP
extensions, suspend/unsuspend, and the WebSocket live-metrics channel.

## What is still to come

Phase 2 — Databases & security: MariaDB/Postgres/Redis management, UFW,
fail2ban, SSH keys, roles, bundled phpMyAdmin / pgAdmin.

Phase 3 — Operations: backups, cron, logs, file manager, API keys.

Phase 4 — v1.0 hardening: self-update, packaging, e2e tests.
