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

## What is still to come

Phase 1 — Core hosting: site CRUD + Nginx vhost engine + PHP-FPM pools,
Let's Encrypt, apply pipeline with rollback.

Phase 2 — Databases & security: MariaDB/Postgres/Redis management, UFW,
fail2ban, SSH keys, roles, bundled phpMyAdmin / pgAdmin.

Phase 3 — Operations: backups, cron, logs, file manager, API keys, CLI.

Phase 4 — v1.0 hardening: self-update, packaging, e2e tests.
