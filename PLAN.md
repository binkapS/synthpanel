# SynthPanel — Comprehensive Plan

**Status:** Draft v0.1 · **Date:** 2026-11-10 · **Owner:** synthpanel contributors

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Goals & Non-Goals](#2-goals--non-goals)
3. [Design Principles](#3-design-principles)
4. [Feature Map](#4-feature-map)
5. [Architecture](#5-architecture)
6. [Security Model](#6-security-model)
7. [Installation & Provisioning](#7-installation--provisioning)
8. [API & CLI](#8-api--cli)
9. [UI/UX Design](#9-uiux-design)
10. [Roadmap & Milestones](#10-roadmap--milestones)
11. [Testing & Quality](#11-testing--quality)
12. [Packaging & Distribution](#12-packaging--distribution)
13. [Documentation & Community](#13-documentation--community)
14. [Risks & Mitigations](#14-risks--mitigations)
15. [Success Metrics](#15-success-metrics)
16. [Decisions](#16-decisions)

---

## 1. Executive Summary

**SynthPanel** is a free, open-source server control panel for single Linux servers. It installs a complete, production-ready web stack — Nginx, multi-version PHP-FPM, MariaDB/PostgreSQL, Redis — and lets you manage everything (sites, databases, SSL, security, backups, cron, logs) from a clean web UI, a REST API, or a CLI.

The design is deliberately inspired by **CloudPanel**, which proves that a control panel can be:

- **Lightweight** — a few MB of RAM at idle, no unnecessary services
- **Modern** — Nginx-only, per-site PHP-FPM isolation, free Let's Encrypt SSL
- **Simple to install** — one command, clean OS, five minutes
- **Uncluttered** — a focused UI that does the essentials extremely well

SynthPanel rebuilds that philosophy in **Rust** (backend) with a **Svelte 5** SPA (frontend), targeting a panel that is fast, memory-safe, and pleasant to operate.

### Target outcomes

| Outcome | Target |
|---|---|
| Cold install → working panel | ≤ 5 minutes on a clean VM |
| Idle RAM footprint (web + core + workers) | < 150 MB |
| Site creation → serving HTTPS | ≤ 60 seconds |
| Unit-test coverage of config generation | ≥ 95% |
| Known critical vulnerabilities | 0 |

---

## 2. Goals & Non-Goals

### 2.1 Goals

- **Full LEMP lifecycle management**: install, configure, monitor, and update the stack
- **Site-centric model**: every domain = one Nginx vhost + one isolated PHP-FPM pool
- **Zero-dependency UI**: everything doable without SSH
- **Secure by default**: firewall, fail2ban, 2FA, automatic SSL, least privilege
- **Single binary distribution**: static Rust binaries, one-command installer
- **API-first**: every UI action has an equivalent API call and CLI command
- **Portable**: Ubuntu LTS + Debian support out of the box
- **Licensed under AGPL-3.0** — free forever, with copyleft protection against SaaS repackaging

### 2.2 Non-Goals (v1)

| Avoid | Rationale |
|---|---|
| Apache support | Nginx-only keeps templates, testing, and support surface small |
| Email server (Postfix/Dovecot) | Huge complexity; recommend external providers |
| Docker/container management | Scope creep; users can run Docker via SSH if needed |
| Multi-server management in v1 | v1 manages one server; the IPC is agent-ready (§5.10) so multi-server can arrive without a rewrite |
| Windows / macOS | Linux server only |
| cPanel-style "everything included" bloat | Focus on doing the core extremely well |
| RHEL-family support in v1 | v1 targets Ubuntu/Debian only; RPM packaging may follow post-1.0 based on demand |

---

## 3. Design Principles (CloudPanel-Inspired)

1. **One-command install** — `curl -sSfL https://get.synthpanel.io | sudo bash` on a clean server is the whole story.
2. **Nginx-only** — no Apache, no `.htaccess`, no mod_php.
3. **Per-site PHP-FPM isolation** — each site runs under its own system user and PHP-FPM pool with its own socket; a broken or compromised site cannot touch another.
4. **Security by default** — firewall enabled, fail2ban installed, panel on HTTPS (port 8443), 2FA available, audited actions.
5. **Minimal footprint** — the panel should be invisible at idle; no polling storms, no heavy JS bundle.
6. **Everything has three interfaces** — UI, REST API, CLI — all driven by the same core.
7. **Validate before apply** — every generated config is tested (`nginx -t`) before reload; failures roll back automatically.
8. **Free and open source** — no paid tiers, no feature locks.

---

## 4. Feature Map

### 4.1 Dashboard

- Stat cards: CPU, load, memory, swap, disk usage, network I/O (live)
- Service health: `nginx`, `php-*`, `mariadb`, `redis`, `synthpanel-*` with start/stop/restart
- Quick actions: add site, issue SSL, run backup, view logs
- Audit feed: recent user actions
- Version/update banner

### 4.2 Sites (Virtual Hosts)

- Create/edit/delete sites: domain, aliases, document root, PHP version, template
- **Templates**: static HTML, generic PHP, Laravel, Symfony, WordPress (optimized), Node/Next.js reverse proxy
- Per-site settings: PHP limits (`memory_limit`, `max_execution_time`, upload size), `php.ini` overrides
- Custom Nginx directives (validated before apply, with syntax highlighting)
- Redirects: www/non-www, HTTP→HTTPS, custom 301/302 rules
- **SSL**: Let's Encrypt (auto-issue + auto-renew), custom certificate upload, force HTTPS, HSTS, OCSP stapling
- Site status: live/offline, last deploy, last error
- Suspend/unsuspend (return 503 page)
- Cloning a site's configuration
- Export/import site definition (JSON)

### 4.3 PHP Management

- Side-by-side PHP versions (8.1 → 8.4), installed via packages
- Per-site version selection
- Per-version extension manager (enable/disable, e.g. `gd`, `intl`, `redis`, `xdebug`)
- Per-site `php.ini` overrides
- PHP-FPM pool metrics (active processes, slow requests)
- Optional status pages

### 4.4 Databases

- Engines: MariaDB/MySQL, PostgreSQL, Redis (all built in at v1)
- Create/drop databases and users; per-site credential generation
- Connection strings and `.env` snippet generator
- Import/export (dump) in the UI
- Bundled one-click installs: **phpMyAdmin** (MariaDB/MySQL) and **pgAdmin** (PostgreSQL), managed by the panel and linked from the Databases page

### 4.5 Security

- **Firewall**: UFW rules UI — ports, protocols, source IP/CIDR, profiles (web, ssh, custom)
- **Fail2ban**: jail status, ban/unban, add IP to ignorelist
- **SSH keys**: manage keys per user; toggle password login
- **Two-factor auth**: TOTP (Authenticator apps), recovery codes
- **Sessions**: list/revoke active sessions, device + IP info
- **Audit log**: every mutating action, searchable, exportable
- **IP allow-list** for panel access
- **Rate limiting** and brute-force lockout on login
- **Security headers** on panel itself (CSP, HSTS, X-Frame-Options)

### 4.6 Users & Roles

- Roles: **Admin** (full access), **Operator** (sites/databases, no security), **User** (own sites only)
- Per-user quotas: max sites, max databases
- Per-user SSH/system access toggles
- Invitation flow (link-based, expiring)

### 4.7 Backups

- What's included: Nginx vhosts, PHP pools, site document roots, database dumps, panel settings
- Destinations: local (`/var/backups/synthpanel`), S3-compatible (incl. MinIO), SFTP, rsync
- Schedules: daily/weekly/monthly with retention (e.g. keep 7 daily, 4 weekly)
- Encryption at rest (optional passphrase)
- One-click restore (full or per-site), download backup archive
- Backup verification: periodic restore test into a staging directory

### 4.8 Cron

- Manage crontabs per system user / per site
- Human-readable schedule builder + raw expression
- Run log viewer (last N runs, exit codes, output tail)
- Run-now button

### 4.9 Logs

- Viewers for: Nginx access/error per site, PHP-FPM error/slow logs, panel audit log, system auth log
- Filters: time range, level, status code, search term
- Tail-follow mode (WebSocket) for live debugging

### 4.10 File Manager

- Browse, create, rename, move, delete, upload (multi-file + drag-and-drop)
- Code editor with syntax highlighting for config files
- Permissions editor (chmod/chown)
- Archive create/extract (zip, tar.gz)

### 4.11 Terminal (Phase 3+)

- Browser-based SSH terminal (WebSocket → `russh` server side)
- Optional per-session recording
- Restricted shell mode for non-admin users

### 4.12 Metrics & Monitoring

- Collector: CPU, load, memory, swap, disk I/O, network I/O, per-process top consumers, per-site request rates (from Nginx logs)
- Retention: raw 10s samples for 24h → 5-min aggregates for 30 days → hourly for 1 year
- Service status + uptime
- Phase 2: alerting (email/webhook) for thresholds (disk > 90%, load, service down)

### 4.13 API & CLI

- REST API v1 under `/api/v1`, OpenAPI 3.1 spec, API keys with scopes
- WebSocket channel for live metrics and log tailing
- CLI `synthpanel` mirroring core operations (root-only, talks to core socket)
- Idempotent, scriptable, JSON output mode

### 4.14 Settings

- Hostname, timezone, locale
- Panel port, session lifetime, password policy
- Panel TLS certificate — issue/renew Let's Encrypt or upload a custom cert for the panel hostname (post-install, requires DNS)
- Update channel (stable/beta), self-update
- Optional OS security updates (unattended-upgrades)
- Maintenance mode (panel-level, not site-level)

### 4.15 Updates

- Self-update: download → signature verify → backup → apply → restart services → rollback on failure
- OS package updates via UI (optional, behind a toggle)
- Changelog view

---

## 5. Architecture

### 5.1 Overview

```
                         ┌────────────────────────────┐
                         │           Browser          │
                         │   https://<host>:8443      │
                         │    Svelte 5 SPA (TLS)      │
                         └─────────────┬──────────────┘
                                       │
                         ┌─────────────▼──────────────┐
                         │      synthpanel-web        │
                         │  systemd · user synthpanel │
                         │  axum · sessions · 2FA     │
                         │  REST /api/v1 · WS /ws     │
                         └─────────────┬──────────────┘
                                       │ Unix socket (0660, group synthpanel)
                                       │ token-authenticated IPC
                         ┌─────────────▼──────────────┐
                         │      synthpanel-core       │
                         │  systemd · user root       │
                         │  privileged ops only:      │
                         │  nginx · php-fpm · ufw     │
                         │  fail2ban · ssh · systemd │
                         │  sqlite state · scheduler  │
                         │  metrics · backups · acme  │
                         └─────────────┬──────────────┘
                                       │
        ┌──────────┬──────────┬────────┴───────┬───────────┬──────────┐
        ▼          ▼          ▼                ▼           ▼          ▼
     Nginx     PHP-FPM    MariaDB/        UFW/         Redis    Filesystem
     vhosts    pools      PostgreSQL      fail2ban              (/var/www, /etc)
```

### 5.2 Privilege separation

- **`synthpanel-core` runs as root**, but its entire network surface is the local Unix socket; it never listens on TCP.
- **`synthpanel-web` runs as the unprivileged `synthpanel` user** and owns all user-facing state: sessions, CSRF tokens, request validation, rate limiting.
- IPC is authenticated: web holds a shared secret (0600, group `synthpanel`); core verifies it per request. Every IPC call is audit-logged.
- **No sudoers entries** — the web process never shells out; core performs all privileged operations through narrow, audited code paths (not arbitrary command execution).
- If a future single-process MVP is preferred for simplicity, it must still run behind HTTPS + 2FA + rate limiting; the two-process design is the target.

### 5.3 Technology stack

| Layer | Choice | Rationale |
|---|---|---|
| Language | Rust (edition 2024, MSRV 1.85) | static binaries, memory safety, excellent ecosystem for systems work |
| Async runtime | tokio | de facto standard, great tooling |
| HTTP server | axum 0.8 | ergonomic, tower middleware, websocket support |
| State store | SQLite via sqlx (WAL) | embedded, zero-config, single-node, easy backups |
| Templating | tinytemplate / askama | typed templates for Nginx & PHP-FPM configs, golden-testable |
| Password hashing | argon2 | OWASP recommended |
| TOTP | totp-rs | standard authenticator apps |
| Metrics | sysinfo + procfs | no external agents |
| ACME client | instant-acme | Let's Encrypt, HTTP-01 + DNS-01 |
| SSH (terminal) | russh | pure-Rust SSH server for the web terminal |
| Process control | systemctl (via core) | systemd is the service manager on all target distros |
| **Frontend framework** | **Svelte 5 + TypeScript** | compile-time reactivity, tiny runtime, minimal bundle — matches the panel's no-bloat ethos |
| Build tool | Vite | fast dev server, small optimized production builds |
| Routing | svelte-spa-router (or SvelteKit SPA mode) | lightweight client-side routing for a self-hosted SPA |
| State | Svelte 5 runes + stores | built-in, no external state library needed |
| Data fetching | `@tanstack/svelte-query` (optional) | caching, retries, optimistic updates |
| Charts | uPlot (wrapped in a Svelte component) | ~35 KB, canvas-based, fast |
| Styling | Tailwind CSS | utility-first, small production bundle |
| Icons | lucide-svelte | clean line icons, tree-shaken |
| i18n | svelte-i18n | community standard |

> **Why Svelte over Vue:** no virtual DOM, so the compiled bundle is smaller and runtime overhead near zero — ideal for a panel that should feel instant while staying invisible on server resources. Runes (`$state`, `$derived`, `$effect`) give fine-grained reactivity with far less boilerplate than stores or Pinia.

### 5.4 Repository layout

```
synthpanel/
├── Cargo.toml                 # workspace
├── crates/
│   ├── synthpanel-core/       # privileged daemon (root)
│   │   └── src/
│   │       ├── main.rs
│   │       ├── ipc/           # unix-socket API + auth
│   │       ├── state/         # sqlite schema + repositories
│   │       ├── nginx/         # vhost render/validate/reload
│   │       ├── php/           # pools, versions, extensions
│   │       ├── ssl/           # acme, renewal, custom certs
│   │       ├── database/      # db/user lifecycle
│   │       ├── security/      # ufw, fail2ban, ssh keys
│   │       ├── backup/        # backup/restore engine
│   │       ├── scheduler/     # cron, renewals, retention
│   │       ├── metrics/       # collector + downsampling
│   │       └── updater/       # self-update
│   ├── synthpanel-web/        # HTTP + API (unprivileged)
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── auth/          # sessions, 2fa, rate limits
│   │   │   ├── api/           # /api/v1 handlers
│   │   │   └── ws/            # dashboard + log tail
│   │   └── frontend/          # Svelte 5 + Vite SPA
│   │       ├── src/
│   │       │   ├── lib/       # api client, stores, components
│   │       │   ├── routes/    # Dashboard, Sites, Databases, ...
│   │       │   └── app.html
│   │       └── vite.config.ts
│   ├── synthpanel-cli/        # user/root CLI
│   └── synthpanel-installer/  # bootstrap installer
├── packaging/
│   ├── debian/
│   └── systemd/
├── docs/
└── tests/                     # integration + e2e
```

### 5.5 Data model (core tables)

```sql
users          (id, email, name, password_hash, totp_secret, totp_enabled,
                role, status, quota_sites, quota_dbs, last_login_at, created_at)
sessions       (id, user_id, token_hash, ip, user_agent, expires_at, created_at)
sites          (id, domain, aliases, document_root, template, php_version,
                pool_user, status, ssl_mode, ssl_cert_id, owner_id,
                created_at, updated_at)
site_nginx     (site_id, directives)            -- custom nginx block
site_php       (site_id, overrides_json)        -- php.ini overrides
databases      (id, engine, name, owner_id, created_at)
db_users       (id, database_id, username, password_encrypted, privileges)
ssl_certs      (id, site_id, kind[le|custom], cert_path, key_path, expires_at)
firewall_rules (id, port, proto, source, action, comment, enabled)
ssh_keys       (id, user_id, label, public_key, fingerprint)
api_keys       (id, user_id, name, prefix, hash, scopes_json, last_used_at)
audit_log      (id, actor_id, action, target, metadata_json, ip, created_at)
backups        (id, kind, status, size, location, schedule_id, created_at)
backup_sched   (id, name, cron, destination_json, retention_json, enabled)
cron_jobs      (id, site_id, user, expression, command, last_run_at, last_status)
settings       (key PRIMARY KEY, value_json)
metrics_raw    (ts, host, cpu, load1, mem_used, swap_used, disk_used, net_rx, net_tx)
metrics_5m     (ts, ... downsampled ...)
metrics_1h     (ts, ...)
```

Notes:

- All secrets (db passwords, API keys, session tokens) stored hashed or encrypted; encryption key lives in `/etc/synthpanel/secret.key` (0600 root).
- SQLite in WAL mode; single writer = core; backups via `VACUUM INTO` (online-consistent).

### 5.6 Nginx integration

- Vhosts live in `/etc/nginx/sites-available/synthpanel/<site-id>.conf`, symlinked to `sites-enabled/`.
- Shared snippets in `/etc/nginx/synthpanel/snippets/` (TLS defaults, security headers, PHP location block).
- Panel-managed block at the top of every generated file:

```nginx
# Managed by SynthPanel — do not edit manually
server {
    listen 443 ssl http2;
    server_name example.com www.example.com;
    root /var/www/<site-id>/public;

    include /etc/nginx/synthpanel/snippets/ssl.conf;
    include /etc/nginx/synthpanel/snippets/security-headers.conf;

    location ~ \.php$ {
        fastcgi_pass unix:/run/php/php8.3-sp-<site-id>.sock;
        include /etc/nginx/synthpanel/snippets/php.conf;
    }

    # Custom directives
    # {{ site.directives }}
}
```

- Every change goes through the **apply pipeline** (§5.9).

### 5.7 PHP-FPM integration

- Pools at `/etc/php/<version>/fpm/pool.d/synthpanel-<site-id>.conf`:

```ini
[sp-<site-id>]
user = sp_<site-id>
group = sp_<site-id>
listen = /run/php/php<ver>-sp-<site-id>.sock
listen.owner = synthpanel
pm = dynamic
pm.max_children = 10
pm.start_servers = 2
php_admin_value[memory_limit] = 256M
```

- Each site gets a dedicated system user (`sp_<site-id>`) owning its document root — **isolation between sites**.
- `php-fpm<ver> -t` before reload; rollback on failure.

### 5.8 Background workers

All workers live inside `synthpanel-core` as tokio tasks (no extra daemons to deploy):

| Worker | Interval | Job |
|---|---|---|
| Metrics collector | 10 s | sample system stats → SQLite |
| Downsampler | 1 min / 1 h | raw → 5m → 1h aggregates |
| Scheduler | 1 min | due cron jobs, cert renewals, backup runs, retention pruning |
| Log watcher | event | tail nginx/php logs for WS subscribers |
| Health checker | 30 s | systemd unit status; auto-restart optional |

### 5.9 Config apply pipeline (the "never break the server" rule)

```
render config → syntax check (nginx -t / php-fpm -t) → backup current config
     → write new config → reload service → probe (curl local site / check pool)
     → OK: mark applied        → FAIL: restore backup, reload, alert
```

- All generated config changes are transactional; the panel never leaves the server in a broken state.
- Every apply is recorded in the audit log with the diff.

### 5.10 Forward compatibility with agents

v1 runs on a single server, but the IPC contract is designed so remote agents can join without a rewrite:

- **Transport-agnostic protocol**: core↔web exchanges typed request/response messages over a Unix socket today; a TCP or QUIC transport with mTLS can carry the same messages to remote nodes later
- **Node identity**: every message carries a node ID + capability token; the same auth model serves both local web processes and future remote agents
- **Idempotent operations**: all mutations are idempotent (request IDs + state versioning), so a retried agent request can never double-apply a change
- **Event-sourced audit log**: audit entries append with monotonic sequence numbers, ready to replicate to a future multi-server control plane
- **Scoped capabilities**: agents authenticate with per-node scopes (e.g. `metrics:read`, `site:write`) rather than blanket root trust

---

## 6. Security Model

### 6.1 Threat model (summary)

- **Remote attacker** targeting the panel (auth bypass, RCE via upload, CSRF, XSS)
- **Compromised site** attempting to read other sites' files or panel secrets
- **Insider** (low-privilege user) attempting privilege escalation
- **Operator error** (bad nginx directive, wrong firewall rule)

### 6.2 Controls

| Risk | Control |
|---|---|
| Panel compromise | HTTPS-only (8443), 2FA, session lifetime limits, IP allow-list, rate limiting, CSP |
| Auth bypass | argon2 hashing, constant-time token compare, CSRF tokens, strict SameSite cookies |
| Site isolation | per-site system user + PHP-FPM pool + open_basedir; document roots owned per user |
| Privilege escalation | core runs as root but exposes only typed operations; no shell passthrough in IPC |
| Config breakage | validate → backup → apply → verify → rollback pipeline |
| Data at rest | encrypted secrets store; backups optionally encrypted (age/gpg) |
| Auditability | every mutating action logged with actor, IP, diff |
| Supply chain | reproducible builds, signed releases, `cargo-audit` + `cargo-deny` in CI, vendored deps option |
| Frontend-specific | Svelte escapes by default; `sanitize` discipline for any `{@html}` usage (file editor preview, log rendering) |

### 6.3 Panel hardening checklist (installer applies automatically)

- UFW: default deny incoming; allow 22, 80, 443, 8443
- fail2ban jails: `sshd`, `nginx-http-auth`, `synthpanel-login`
- SSH: disable root login, key-based auth encouraged (toggle in UI)
- Panel session cookie: `Secure; HttpOnly; SameSite=Strict`
- Security headers: `CSP`, `X-Content-Type-Options`, `X-Frame-Options: DENY`, `Referrer-Policy`, `HSTS` (after first successful login)
- Automatic security updates optional (unattended-upgrades)

---

## 7. Installation & Provisioning

### 7.1 Installer UX (CloudPanel-style)

```bash
curl -sSfL https://get.synthpanel.io | sudo bash
```

The installer:

1. Checks prerequisites: supported distro (Ubuntu 22.04 LTS / 24.04 LTS, Debian 12+), root, ≥ 1 GB RAM, clean port 8443
2. Installs base packages: Nginx, MariaDB, PHP versions, Redis, UFW, fail2ban, acme.sh deps
3. Creates `synthpanel` user + group; creates `/var/www`, `/var/lib/synthpanel`, `/etc/synthpanel`
4. Installs `synthpanel-core` + `synthpanel-web` + `synthpanel` CLI binaries
5. Writes systemd units, enables + starts them
6. Generates a self-signed certificate for the panel UI; a Let's Encrypt (or custom) cert can be issued later via **Settings → Panel TLS** once a hostname + DNS is in place
7. Prints the one-time setup URL: `https://<ip>:8443` + admin token

Unattended mode for automation:

```bash
curl -sSfL https://get.synthpanel.io | sudo bash -s -- \
  --hostname panel.example.com \
  --admin-email admin@example.com \
  --channel stable
```

### 7.2 First-run wizard

1. Accept license / set admin account
2. Set hostname + timezone
3. Set hostname (required before a Let's Encrypt cert can be issued for the panel UI later)
4. Review security baseline (firewall, fail2ban, SSH)
5. Dashboard

### 7.3 Upgrade & uninstall

- `synthpanel update` (CLI) or **Settings → Update** (UI): backup → download → verify signature → apply → health check → rollback on failure
- `synthpanel uninstall [--purge]`: stops services, removes units; `--purge` also removes `/var/www`, databases, configs (with confirmation + final backup)

---

## 8. API & CLI

### 8.1 API conventions

- Base: `/api/v1`
- Auth: session cookie (browser) or `Authorization: Bearer <api-key>` (scripts)
- Format: JSON; errors as `{ "error": { "code", "message", "details" } }`
- Rate limits: `429` with `Retry-After`; API keys have per-scope quotas
- Every mutating call is audit-logged
- OpenAPI 3.1 spec published at `/api/v1/openapi.json` and in the docs

### 8.2 Endpoint map (core)

```
Auth      POST /auth/login  POST /auth/logout  GET /auth/me
          POST /auth/2fa/enable|verify|disable
Sites     GET|POST /sites   GET|PATCH|DELETE /sites/:id
          POST /sites/:id/ssl/issue   POST /sites/:id/ssl/force-https
PHP       GET /php/versions  GET /php/versions/:v/extensions
          POST /php/versions/:v/extensions/:name/{enable,disable}
Databases GET|POST /databases  DELETE /databases/:id
          POST /databases/:id/dump   POST /databases/:id/import
Security  GET|PUT /security/firewall   POST /security/firewall/rules
          GET /security/fail2ban/jails  POST /security/fail2ban/:jail/unban
          GET|POST /security/ssh-keys
Users     GET|POST /users  PATCH /users/:id
Backups   GET /backups  POST /backups/run  POST /backups/:id/restore
          GET /backups/:id/download
Cron      GET|POST /cron  DELETE /cron/:id  POST /cron/:id/run
Logs      GET /logs?type=nginx-access&site_id=…&follow=1
Files     GET /files?path=…  POST /files/upload  PATCH /files/:path/permissions
Metrics   GET /metrics?range=24h
Audit     GET /audit
Settings  GET|PATCH /settings
API keys  GET|POST /api-keys  DELETE /api-keys/:id
Realtime  WS   /ws/dashboard   WS /ws/logs
```

### 8.3 CLI

```
synthpanel sites:list | sites:add example.com --php=8.3 --ssl
synthpanel sites:delete <id> --force
synthpanel php:versions [--available]
synthpanel db:create --site <id>
synthpanel backup:run [--full]   synthpanel backup:list
synthpanel logs:tail nginx --site example.com
synthpanel status   synthpanel update   synthpanel uninstall
```

- Root-only (via sudo); talks to core over the Unix socket
- `--output json|table|yaml` for scripting
- Same operations as UI → guarantees API-first design

---

## 9. UI/UX Design

### 9.1 Layout (CloudPanel-like)

```
┌───────────────────────────────────────────────────────────┐
│  SynthPanel   [search]        [updates] [alerts] [admin ▾] │
├────────────┬──────────────────────────────────────────────┤
│  Dashboard │   Page title                        [actions] │
│  Sites     │  ┌────────┬────────┬────────┬────────┐        │
│  Databases │  │ stat   │ stat   │ stat   │ stat   │        │
│  PHP       │  └────────┴────────┴────────┴────────┘        │
│  Security  │                                              │
│  Backups   │   Content (tables, forms, charts)            │
│  Cron      │                                              │
│  Logs      │                                              │
│  Files     │                                              │
│  Metrics   │                                              │
│  Settings  │                                              │
└────────────┴──────────────────────────────────────────────┘
```

### 9.2 Design principles

- **Calm and clean**: white cards, generous whitespace, restrained color (blue accent), no clutter
- **Information hierarchy**: most-used actions one click away; destructive actions behind confirmation + type-to-confirm for deletes
- **Live where it matters**: dashboard metrics and logs push over WebSocket; everything else is snappy polling (≤ 30 s)
- **Keyboard friendly**: `/` focuses search, `g d` → dashboard, etc.
- **Responsive**: usable on tablet; mobile gets a simplified dashboard + essential actions
- **Dark mode** (user preference, persisted)
- **Accessible**: WCAG 2.2 AA, focus states, aria labels, reduced-motion support
- **i18n ready**: all strings through svelte-i18n (English first)

### 9.3 Frontend architecture (Svelte 5)

- **SPA served by axum**: Vite builds static assets embedded in the web binary; no separate Node server in production
- **Routing**: `svelte-spa-router` with hash-free history API fallback handled by axum (all non-`/api` paths serve `index.html`); SvelteKit SPA mode is an acceptable alternative if file-based conventions are preferred
- **State**: Svelte 5 runes (`$state`, `$derived`) for local component state; a small set of global stores for session, sites, and live metrics
- **Data layer**: typed API client generated from the OpenAPI spec; `@tanstack/svelte-query` for caching and refetch orchestration
- **Realtime**: a `websocketStore` rune that multiplexes `/ws/dashboard` and `/ws/logs` subscriptions with auto-reconnect
- **Components**: small, composable primitives (`StatCard`, `DataTable`, `SideNav`, `ConfirmDialog`, `CodeEditor`, `Chart`) in `src/lib/components`
- **Charts**: thin Svelte wrapper around uPlot for time-series; no chart library runtime overhead

### 9.4 Key screens

- **Dashboard**: stat cards, network chart, service list, audit feed, quick actions
- **Sites list**: status dot, domain, PHP version, SSL expiry badge, actions menu
- **Site detail**: tabs (Overview, SSL, Nginx, PHP, Logs, Backups, Cron, Files)
- **Forms**: inline validation, sensible defaults, "advanced" collapsibles
- **Empty states** with clear next steps ("No sites yet → Add your first site")

---

## 10. Roadmap & Milestones

### Phase 0 — Foundations (Weeks 1–3)

- Workspace + CI (fmt, clippy, test, cargo-audit, cargo-deny)
- Core daemon skeleton: IPC, SQLite schema, settings, systemd units
- Web skeleton: axum, auth, session, 2FA, Svelte app with layout + routing
- Installer: bootstrap script + `--unattended`
- **Exit:** fresh Ubuntu 24.04 → installer → login → dashboard shows CPU/RAM

### Phase 1 — Core Hosting (Weeks 4–9)

- Sites CRUD + Nginx vhost engine + apply pipeline with rollback
- PHP versions + per-site pools + extensions + php.ini overrides
- Let's Encrypt (HTTP-01) + auto-renew + force HTTPS
- Dashboard metrics + service health + WebSocket push
- **Exit:** create site in UI → serves PHP over HTTPS in ≤ 60 s; corrupt nginx config → panel recovers

### Phase 2 — Data & Security (Weeks 10–14)

- MariaDB + PostgreSQL + Redis management; per-site credentials; bundled phpMyAdmin / pgAdmin one-click installs
- Firewall UI (UFW), fail2ban, SSH keys, IP allow-list
- Users & roles + audit log
- **Exit:** full stack managed end-to-end; external security baseline passes

### Phase 3 — Operations (Weeks 15–19)

- Backups (local/S3/SFTP) + restore + verification
- Cron UI + run logs
- Logs viewer + tail-follow
- File manager
- REST API v1 + API keys + CLI
- **Exit:** backup → delete site → restore works; CLI covers 90% of UI ops

### Phase 4 — v1.0 Hardening (Weeks 20–24)

- Self-update with signature verification + rollback
- Packaging: .deb + apt repository
- Playwright e2e, chaos tests, external security audit
- Docs site, i18n scaffolding, accessibility pass
- **Exit:** v1.0 released; upgrade path from RC verified

### Phase 5 — Beyond v1.0 (optional)

- Web terminal (russh), alerting (email/webhook), Git-based deployments
- Multi-server agents, plugin API, mobile-friendly PWA
- DNS zone management (if demand)

---

## 11. Testing & Quality

| Level | Tooling | Focus |
|---|---|---|
| Backend unit | `cargo test` | config rendering, validation, parsers, crypto |
| Frontend unit | Vitest + @testing-library/svelte | stores, components, API client |
| Golden | insta snapshots | Nginx/PHP-FPM configs for every template |
| Integration | Docker (ubuntu:24.04 + nginx + php-fpm + mariadb) | install → site → SSL → backup/restore |
| E2E | Playwright | core user journeys (add site, issue SSL, backup, restore) |
| Chaos | scripts | kill nginx mid-reload, corrupt config, disk full, ACME failure → verify rollback |
| Security | `cargo-audit`, `cargo-deny`, `cargo-semver-checks` | dependency vulnerabilities, license compliance |
| Performance | `wrk`/k6 | API throughput; UI bundle size budget (< 150 KB gzip — Svelte makes this easy) |
| Upgrade | CI matrix | RC → stable, stable → stable, rollback |

**Quality gates for merge:** clippy clean, ≥ 95% coverage on config generation, e2e green, no audit findings, changelog updated.

---

## 12. Packaging & Distribution

- **Debian/Ubuntu**: `.deb` + apt repository (`apt.synthpanel.io`), dependencies pinned
- **Offline**: single tarball with all binaries + `synthpanel install --offline`
- **Post-v1.0 (if demand)**: RHEL-family RPMs and Arch AUR packaging
- **systemd units**: `synthpanel-core.service`, `synthpanel-web.service`, `synthpanel-scheduler.service` (or timers)
- **Ports**: panel 8443/TCP (HTTPS); nothing else exposed by the panel itself
- **Paths**:
  - `/etc/synthpanel/` — config + secrets
  - `/var/lib/synthpanel/` — SQLite DB, state
  - `/var/www/<site-id>/` — site roots
  - `/var/log/synthpanel/` — panel logs
  - `/var/backups/synthpanel/` — local backups

---

## 13. Documentation & Community

- **Docs site** (VitePress): installation, quickstart, how-to guides, API reference (OpenAPI), CLI reference, troubleshooting, security model
- **Contributing guide**: dev environment (single `cargo run` + Vite dev server), code style, PR process
- **Changelog + release notes** with upgrade instructions
- **i18n**: English first; community translations via svelte-i18n JSON workflow
- **Community**: GitHub Discussions, issue templates (bug/feature/security), security policy (`SECURITY.md`) with private disclosure

---

## 14. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Privileged core compromised | Low | Critical | minimal attack surface (UDS only), typed IPC, audit log, memory-safe Rust |
| Generated Nginx config breaks server | Medium | High | `nginx -t` before apply, automatic rollback, backups, golden tests |
| Distro/package drift (PHP, Nginx versions) | Medium | Medium | pin versions, CI matrix per distro, abstraction layer over package manager |
| Let's Encrypt rate limits / outages | Medium | Low | retry with backoff, DNS-01 option, expiry alerts |
| SQLite contention/locking | Low | Medium | WAL mode, single writer, timeouts, monitored |
| Panel is single point of failure for management | Medium | Low | server keeps serving sites if panel dies; CLI emergency mode |
| Upgrade breaks existing setup | Medium | High | backup-first, health check, one-command rollback |
| Scope creep (mail, Docker, clusters) | High | Medium | explicit non-goals; phased roadmap |

---

## 15. Success Metrics

| Metric | Target |
|---|---|
| Cold install → panel reachable | ≤ 5 min (1 vCPU / 1 GB) |
| Idle RAM (web + core + workers) | < 150 MB |
| Site create → HTTPS live | ≤ 60 s |
| Panel UI TTI | < 1.5 s on 4G |
| Unit coverage (config generation) | ≥ 95% |
| Open critical CVEs in dependencies | 0 |
| Backup → restore success rate | ≥ 99.9% |
| Panel availability | ≥ 99.9% |

---

## 16. Decisions

| Question | Decision |
|---|---|
| License | **AGPL-3.0** — copyleft protection against SaaS repackaging |
| Initial distros | **Ubuntu LTS (22.04/24.04) + Debian 12 only**; RHEL family post-v1.0 if demand |
| PostgreSQL / Redis | **Built in at v1** — full database stack managed from the panel |
| Panel TLS | **Self-signed at install**; Let's Encrypt or custom cert can be issued after installation once hostname + DNS is set |
| phpMyAdmin / pgAdmin | **Bundled** — one-click installs managed by the panel |
| Telemetry | **None** — no telemetry, no install stats, no external calls |
| Multi-server | **v1 is single-server**, but the IPC is designed agent-ready from day one (§5.10) — no rewrite needed later |

---

*This plan is a living document. Decisions in §5–§9 are recommendations; the roadmap (§10) is the committed sequence for v1.0.*
