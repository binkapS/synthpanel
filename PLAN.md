# SynthPanel — Design Document

**Status:** v1.0 implemented · **Last updated:** 2026-11-11 · **Owner:** synthpanel contributors

This document describes the design of SynthPanel and, for every feature, whether
it is **shipped**, **partial** or **planned**. The honest, per-phase status lives
in [`docs/roadmap.md`](./docs/roadmap.md); this file explains *how* the panel is
built and *why*.

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

**Status markers used throughout:**

| Marker | Meaning |
|---|---|
| ✅ | Shipped and covered by a test |
| 🟡 | Shipped in part, or shipped without the full UI |
| ⬜ | Planned, not implemented yet |

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
- **Licensed under AGPL-3.0-or-later** — free forever, with copyleft protection against SaaS repackaging

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

| Feature | Status |
|---|---|
| Stat cards: CPU, load, memory, swap, disk usage, network I/O (live) | ✅ |
| Service health for `nginx`, `php-*`, `mariadb`, `redis`, `synthpanel-*` | ✅ |
| Start/stop/restart of a service | 🟡 the API works, the dashboard only lists them |
| Quick actions: add site, run backup, open terminal | ✅ |
| Audit feed of the recent actions | ✅ |
| Update banner | 🟡 the update card lives in Settings |

### 4.2 Sites (virtual hosts)

| Feature | Status |
|---|---|
| Create/edit/delete sites: domain, aliases, document root, PHP version, template | ✅ |
| Templates: static, PHP, Laravel, Symfony, WordPress, Node | ✅ |
| Per-site `php.ini` overrides (`memory_limit`, `max_execution_time`, upload size) | 🟡 stored and applied, no editor yet |
| Custom Nginx directives, validated before they are applied | ✅ |
| Redirects: www/non-www and HTTP → HTTPS | ✅ |
| Custom redirect rules | ⬜ |
| SSL: Let's Encrypt over HTTP-01, issued from the UI | ✅ |
| Custom certificate upload | ⬜ |
| HSTS, OCSP stapling | 🟡 HSTS only |
| Suspend/unsuspend (the site answers with 503) | ✅ |
| Cloning a site, export/import of a site definition | ⬜ |

### 4.3 PHP management

| Feature | Status |
|---|---|
| Side-by-side PHP versions 8.1 → 8.4 | ✅ |
| Per-site version selection | ✅ |
| A page that shows the installed versions, the pools and the sites | ✅ |
| Per-version extension manager | ⬜ |
| Per-site `php.ini` overrides | 🟡 see above |
| PHP-FPM pool metrics, status pages | ⬜ |

### 4.4 Databases

| Feature | Status |
|---|---|
| Engines: MariaDB/MySQL, PostgreSQL, Redis | ✅ |
| Create/drop databases and users with per-site privileges | ✅ |
| The database is provisioned on the server *before* the panel records it | ✅ |
| Connection strings and `.env` snippets | ⬜ |
| Dump/restore through the UI | ⬜ (the backups include the dumps) |
| One-click phpMyAdmin / pgAdmin | ⬜ |

### 4.5 Security

| Feature | Status |
|---|---|
| Firewall: UFW rules with ports, protocols and source CIDRs | ✅ |
| Fail2ban: jails, ban/unban | ✅ |
| SSH keys | ✅ |
| Two-factor authentication: TOTP with a QR code | ✅ |
| List and revoke the active sessions | ⬜ |
| Audit log of every mutating action | ✅ |
| IP allow-list for the panel | ⬜ |
| Rate limiting on the login | ✅ |
| Security headers on the panel itself (CSP, HSTS, X-Frame-Options) | ✅ |

### 4.6 Users & roles

| Feature | Status |
|---|---|
| Roles: admin, operator, user | ✅ |
| Per-user quotas for sites and databases | 🟡 stored, not enforced yet |
| Per-user SSH/system access toggles | ⬜ |
| Invitation flow | ⬜ |

### 4.7 Backups

| Feature | Status |
|---|---|
| Nginx vhosts, PHP pools, document roots and database dumps in one archive | ✅ |
| Destinations: local, SFTP, S3-compatible (SigV4) | ✅ |
| rsync destination | ⬜ |
| Schedules with retention | ✅ |
| Optional ChaCha20-Poly1305 encryption | ✅ |
| Restore (full or per site), download the archive | ✅ |
| Verification of an archive | ✅ |

### 4.8 Cron

| Feature | Status |
|---|---|
| Jobs per system user and per site, written to `/etc/cron.d/synthpanel` | ✅ |
| Human-readable schedule summary plus the raw expression | ✅ |
| Run log with exit codes and output | ✅ |
| Run-now button | ✅ |

### 4.9 Logs

| Feature | Status |
|---|---|
| Viewers for Nginx access/error, PHP-FPM, the panel and the system auth log | ✅ |
| Filters for time, level, status code and a search term | ✅ |
| Tail-follow over a WebSocket | ✅ |

### 4.10 File manager

| Feature | Status |
|---|---|
| Browse, create, rename, delete, upload and download | ✅ |
| Permissions editor (chmod/chown) | ✅ |
| Archives: create and extract zip/tar.gz | ✅ |
| A code editor with syntax highlighting | ⬜ |
| Multi-file drag-and-drop | ⬜ |

### 4.11 Terminal

| Feature | Status |
|---|---|
| A shell in the browser | ✅ — a pseudo-terminal inside the core, **not** an SSH server |
| The session is closed when the page is left, or after 30 minutes | ✅ |
| Restricted shell mode for non-admin users | 🟡 the terminal is admin-only |
| Session recording | ⬜ |

> The plan originally called for an SSH server (`russh`). The panel deliberately
> does not open a second port: the shell lives in the privileged core and is
> reached through the authenticated API, so there is nothing extra to expose.

### 4.12 Metrics & monitoring

| Feature | Status |
|---|---|
| CPU, load, memory, swap, disk and network | ✅ |
| Service status | ✅ |
| Retention: raw → 5-minute → hourly aggregates | ✅ |
| Per-process and per-site request rates | ⬜ |
| Alerting on thresholds (disk, load, service down) | ✅ (Phase 5) |

### 4.13 API & CLI

| Feature | Status |
|---|---|
| REST API v1 under `/api/v1` | ✅ |
| API keys with scopes, stored as SHA-256 hashes | ✅ |
| WebSocket channels for live metrics and log tailing | ✅ |
| A CLI that mirrors the core operations | ✅ |
| `--output json` for scripting | ✅ |
| An OpenAPI 3.1 document | ⬜ |

### 4.14 Settings

| Feature | Status |
|---|---|
| Hostname, timezone, locale | ✅ |
| Panel certificate: issue a Let's Encrypt certificate for the panel | ✅ |
| Update channel and the signed self-update | ✅ |
| Panel port, session lifetime, password policy | 🟡 environment variables |
| OS security updates (unattended-upgrades) | ⬜ |
| Maintenance mode | ⬜ |

### 4.15 Updates

| Feature | Status |
|---|---|
| Self-update: download → verify the signature → back up → apply → restart → health check → roll back | ✅ |
| A changelog view | ⬜ |
| OS package updates from the UI | ⬜ |

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
| HTTP server | axum 0.8 | ergonomic, tower middleware, WebSocket support |
| TLS | rustls 0.23 | the certificate is resolved from disk on every handshake, so it can be replaced while the panel runs |
| State store | SQLite via sqlx (WAL) | embedded, zero-config, single-node, easy backups |
| Config rendering | plain string building | the templates are small and fully covered by unit tests; a template engine would only add a dependency |
| Password hashing | argon2id | OWASP recommended |
| TOTP | totp-rs | standard authenticator apps |
| Metrics | sysinfo | no external agents |
| Certificates | acme.sh | one well-known tool instead of an embedded ACME client |
| Mail | a small SMTP client over tokio-rustls | STARTTLS and AUTH, or a local MTA through `sendmail` |
| QR codes | qrcodegen | rendered server-side as an SVG, so the bundle stays small |
| Process control | systemctl (via the core) | systemd is the service manager on all target distros |
| **Frontend framework** | **Svelte 5 + TypeScript** | compile-time reactivity, tiny runtime, minimal bundle — matches the panel's no-bloat ethos |
| Build tool | Vite | fast dev server, small optimized production builds |
| Routing | a small History-API router (`lib/router.ts`) | 40 lines instead of a dependency |
| State | Svelte 5 runes and stores | built in, no state library needed |
| Data layer | a typed `fetch` wrapper (`lib/api.ts`) | the API is small; caching would be premature |
| Charts | hand-written SVG sparklines | no chart library, a few kilobytes instead of tens |
| Terminal | xterm.js, loaded on demand | the emulator is a separate chunk, the rest of the panel stays at ~57 kB gzip |
| Styling | Tailwind CSS | utility-first, small production bundle |
| Icons | lucide-svelte | clean line icons, tree-shaken |
| i18n | a small store (`lib/i18n.ts`) | English is the reference locale, other locales are a dictionary |

> **Why Svelte over Vue:** no virtual DOM, so the compiled bundle is smaller and
> the runtime overhead is near zero — ideal for a panel that should feel instant
> while staying invisible on the server. Runes (`$state`, `$derived`, `$effect`)
> give fine-grained reactivity with far less boilerplate than stores or Pinia.
>
> **Why so much hand-written?** The panel ships a fixed set of pages that talk to
> one API. Every dependency that replaces thirty lines of code also has to be
> audited, updated and kept inside the bundle budget.

### 5.4 Repository layout

```
synthpanel/
├── Cargo.toml                   # the workspace
├── crates/
│   ├── synthpanel-ipc/          # the protocol shared by core, web and CLI
│   ├── synthpanel-core/         # the privileged daemon (root)
│   │   ├── migrations/          # the SQLite schema, applied in order
│   │   └── src/
│   │       ├── ipc.rs           # the Unix-socket server and its dispatch table
│   │       ├── apply.rs         # render → lint → backup → write → validate → reload
│   │       ├── sites.rs         # sites and their per-site state
│   │       ├── nginx.rs         # the vhost templates
│   │       ├── php.rs           # the PHP-FPM pools and the versions
│   │       ├── ssl.rs           # acme.sh, the certificate status, the panel certificate
│   │       ├── databases.rs     # the database and user lifecycle
│   │       ├── security.rs      # ufw, fail2ban, ssh keys
│   │       ├── backup.rs        # the backup and restore engine
│   │       ├── cron.rs          # the crontab parser and the job runner
│   │       ├── scheduler.rs     # the one-minute tick
│   │       ├── logs.rs          # the log sources and the tail
│   │       ├── files.rs         # the sandboxed file manager
│   │       ├── deploy.rs        # the Git deployments
│   │       ├── alerts.rs        # the alerting engine
│   │       ├── mailer.rs        # SMTP and the local MTA
│   │       ├── terminal.rs      # the pseudo-terminal sessions
│   │       ├── update.rs        # the signed self-update
│   │       ├── metrics.rs       # the collector and the downsampling
│   │       └── users.rs         # the users and the roles
│   ├── synthpanel-web/          # the HTTP API and the embedded SPA
│   │   ├── src/
│   │   │   ├── api/             # the /api/v1 handlers
│   │   │   ├── ws.rs            # the dashboard, log and terminal sockets
│   │   │   ├── tls.rs           # the certificate and the resolver
│   │   │   └── middleware.rs    # the security headers and the CSRF guard
│   │   └── frontend/            # the Svelte 5 SPA
│   │       ├── public/          # the manifest, the service worker, the icons
│   │       └── src/
│   │           ├── lib/         # the API client, the stores, the components
│   │           └── routes/      # Dashboard, Sites, Databases, PHP, …
│   ├── synthpanel-cli/          # the `synthpanel` command-line interface
│   └── synthpanel-installer/    # the bootstrap installer
├── packaging/
│   ├── build-deb.sh             # builds the .deb
│   ├── apt/publish.sh           # publishes a flat apt repository
│   ├── sign-release.sh          # signs a release with Ed25519
│   ├── systemd/                 # the unit files
│   └── install.sh               # the `curl | sudo bash` bootstrap
├── docs-site/                   # the VitePress documentation site
├── e2e/                         # the Playwright suite
├── tests/                       # the smoke, chaos and update scripts
├── docs/roadmap.md              # the honest status
└── PLAN.md                      # this document
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
metrics_1h     (ts, ... downsampled ...)
alert_channels (id, name, kind[webhook|email], config_json, enabled, created_at)
alert_rules    (id, name, event, severity, channel_id, enabled, cooldown_secs, created_at)
alerts         (id, rule_id, event, severity, message, status, error, created_at, delivered_at)
deployments    (id, site_id, repo_url, branch, target_subdir, build_command, deploy_user, enabled)
deploy_runs    (id, deployment_id, site_id, commit_sha, branch, status, message, output)
updates        (id, from_version, to_version, status, backup_dir, error, started_at, finished_at)
```

Notes:

- All secrets (db passwords, API keys, session tokens) stored hashed or encrypted; encryption key lives in `/etc/synthpanel/secret.key` (0600 root).
- SQLite in WAL mode; single writer = core; backups via `VACUUM INTO` (online-consistent).

### 5.6 Nginx integration

- The vhosts live in `/etc/synthpanel/nginx/sites-available/synthpanel/<site-id>.conf`,
  symlinked into `sites-enabled/`. The panel owns its own tree, so it never
  touches the distribution's configuration.
- The shared snippets come from the distribution (`/etc/nginx/snippets/`); the
  panel only owns the few directives it has to control.
- The generated file starts with a marker:

```nginx
# Managed by SynthPanel — do not edit manually
# site-id=<uuid> domain=example.com

server {
    listen 443 ssl;
    server_name example.com www.example.com;
    root /var/www/sp_example.com;

    location ~ \.php$ {
        fastcgi_pass unix:/run/php/php8.3-sp-<uuid>.sock;
        include /etc/nginx/snippets/fastcgi-php.conf;
    }

    # Custom directives
    # {{ site.directives }}
}
```

The panel only owns `root`, `listen`, `server_name` and the PHP block; every
other directive in the file is left alone, and the custom directives of the
site are appended below them.

- Every change goes through the **apply pipeline** (§5.9).

### 5.7 PHP-FPM integration

- The pools live in `/etc/synthpanel/php/<version>/fpm/pool.d/synthpanel-<site-id>.conf`:

```ini
[sp-<site-id>]
user = sp_<site-id>
group = sp_<site-id>
listen = /run/php/php8.3-sp-<site-id>.sock
listen.owner = synthpanel
pm = dynamic
pm.max_children = 10
pm.start_servers = 2
php_admin_value[memory_limit] = 256M
```

- Every site gets its own system user (`sp_<site-id>`) that owns its document
  root — that is the isolation between two sites.
- The pool is validated with `php-fpm<version> -t` before the reload, and a
  failure rolls the change back.

### 5.8 Background workers

All the workers live inside `synthpanel-core` as tokio tasks, so there is no
extra daemon to deploy and nothing to supervise.

| Worker | Interval | Job |
|---|---|---|
| The metrics collector | 10 s | samples the system into SQLite |
| The pruner | 1 h | deletes the old raw samples |
| The scheduler | 60 s | runs the due cron jobs and the backup schedules |
| The alert tick | 60 s | checks the disk usage and the certificates that expire |

The service status and the log tail are read on demand — the panel never
polls in the background.

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

The bootstrap script downloads the release tarball and hands over to the Rust
installer, which:

1. Checks the prerequisites: a supported distribution (Ubuntu 22.04/24.04,
   Debian 12+), root, and a free port 8443.
2. Installs the base packages: Nginx, PHP-FPM, MariaDB, PostgreSQL, Redis,
   UFW and fail2ban (`--skip-packages` leaves them alone).
3. Creates the `synthpanel` user and group and the directory layout —
   `/var/www`, `/var/lib/synthpanel`, `/var/log/synthpanel`,
   `/var/backups/synthpanel`, `/etc/synthpanel`, `/run/synthpanel`. The TLS
   directory belongs to the web user, because the web process writes the
   certificate into it.
4. Installs `synthpanel-core`, `synthpanel-web` and the `synthpanel` CLI.
5. Writes the systemd units, enables and starts them.
6. Creates the admin account and prints the URL to sign in.

Unattended mode:

```bash
curl -sSfL https://get.synthpanel.io | sudo bash -s -- \
  --unattended --hostname panel.example.com \
  --admin-email admin@example.com
```

### 7.2 The first account

There is no sign-up form: the first administrator is created by the installer
or by the CLI, which talks to the core over its Unix socket:

```bash
sudo synthpanel users:create you@example.com --name "Your Name" --role admin
```

From then on the panel manages its own users, with roles and quotas.

### 7.3 Upgrade & uninstall

- `synthpanel update` (CLI) or **Settings → Panel update** (UI): back up the
  binaries → download → verify the Ed25519 signature and the SHA-256 checksum
  → replace → restart → health check → roll back when anything fails.
- The `.deb` package upgrades the installation with `dpkg -i`; the apt
  repository serves the same package.

## 8. API & CLI

### 8.1 API conventions

- Base: `/api/v1`
- Auth: a session cookie (browser) or `Authorization: Bearer <api-key>`
  (scripts). API keys bypass the CSRF check, because a browser never attaches
  them automatically.
- Format: JSON; errors as `{ "error": { "code", "message" } }`
- Rate limits: `429` on the login; every failed attempt is counted per IP.
- Every mutating call is audit-logged.
- An OpenAPI 3.1 document is planned, not written yet.

### 8.2 Endpoint map

```
Auth      POST /auth/login  POST /auth/logout  GET /auth/me
          POST /auth/2fa/setup|enable|verify|disable
Sites     GET|POST /sites   GET|PATCH|DELETE /sites/:id
          POST /sites/:id/ssl/issue   GET /sites/:id/ssl/status
          POST /sites/:id/suspend|unsuspend
PHP       GET /php/versions  GET /php/status
Databases GET|POST /databases  GET|DELETE /databases/:id
          GET|POST /databases/:id/users  DELETE /databases/users/:id
          GET /databases/engines
Security  GET /security/firewall  POST /security/firewall/rules
          GET /security/fail2ban  POST /security/fail2ban/unban
          GET|POST /security/ssh-keys  DELETE /security/ssh-keys/:id
Users     GET|POST /users  PATCH /users/:id
Backups   GET /backups  POST /backups/run  POST /backups/:id/verify|restore
          GET /backups/:id/download  DELETE /backups/:id
          GET|POST /backups/schedules  DELETE /backups/schedules/:id
Cron      GET|POST /cron  PATCH|DELETE /cron/:id  POST /cron/:id/run
          GET /cron/:id/runs  GET /cron/describe
Logs      GET /logs?type=…&site_id=…&search=…  GET /logs/sources
Files     GET /files?scope=…&path=…  GET|PUT /files/content
          POST /files/dir|rename|archive|extract  PATCH /files/permissions
Alerts    GET /alerts  POST /alerts/test
          GET|POST /alerts/channels  DELETE /alerts/channels/:id
          GET|POST /alerts/rules  DELETE /alerts/rules/:id
Deploy    GET|POST /deployments  PATCH|DELETE /deployments/:id
          POST /deployments/:id/run  GET /deployments/:id/runs
Terminal  GET /terminal  POST /terminal/open  DELETE /terminal/:id
          POST /terminal/:id/write|resize  GET /terminal/:id/read
Panel     GET /panel/ssl/status  POST /panel/ssl/issue
Update    GET /update  POST /update/check|apply|rollback
Metrics   GET /metrics/snapshot  GET /metrics/range?range=24h
Services  GET /services
Audit     GET /audit
Settings  GET|PATCH /settings
API keys  GET|POST /api-keys  DELETE /api-keys/:id
Realtime  WS /ws/dashboard   WS /ws/logs   WS /ws/terminal/:id
```

### 8.3 CLI

```
synthpanel status   synthpanel metrics   synthpanel metrics:range   synthpanel audit
synthpanel settings
synthpanel sites:list   synthpanel sites:add   synthpanel sites:show   synthpanel sites:delete
synthpanel users:list   synthpanel users:create
synthpanel db:list   synthpanel db:create   synthpanel db:delete
synthpanel security:firewall   synthpanel security:fail2ban   synthpanel security:ssh-keys
synthpanel backup:list   synthpanel backup:run   synthpanel backup:verify   synthpanel backup:restore
synthpanel cron:list   synthpanel cron:add   synthpanel cron:run   synthpanel cron:runs   synthpanel cron:delete
synthpanel logs   synthpanel files:list   synthpanel files:read   synthpanel files:write
synthpanel apikeys:list   synthpanel apikeys:create   synthpanel apikeys:delete
synthpanel alerts:list   synthpanel alerts:channels   synthpanel alerts:channel:add
synthpanel alerts:rule:add   synthpanel alerts:test
synthpanel deploy:list   synthpanel deploy:add   synthpanel deploy:run   synthpanel deploy:runs
synthpanel update   synthpanel update:status   synthpanel update:rollback
```

- Talks to the core over the Unix socket; reads the shared token from
  `/etc/synthpanel/ipc-token`, so it needs root.
- `--output json|table|yaml` for scripting.
- The same operations as the UI, which guarantees the API-first design.

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
│  Deploys   │                                              │
│  Alerts    │                                              │
│  Terminal  │                                              │
│  Users     │                                              │
│  Settings  │                                              │
└────────────┴──────────────────────────────────────────────┘
```

### 9.2 Design principles

- **Calm and clean**: white cards, generous whitespace, a restrained blue
  accent, no clutter.
- **Information hierarchy**: the most-used actions are one click away,
  destructive actions ask for a confirmation.
- **Live where it matters**: the dashboard metrics and the log tail push over
  a WebSocket, everything else polls.
- **Dark mode**, remembered in the browser.
- **Accessible**: a skip link, a `#main` landmark, labelled controls, focus
  states, `aria-current` on the active route, and a dark mode that respects
  the system preference.
- **Responsive**: the sidebar collapses on narrow screens; the terminal and
  the tables scroll rather than break.
- **i18n ready**: the navigation and the login go through the `$t` store, so
  another locale is a dictionary and an entry in the settings.

### 9.3 Frontend architecture (Svelte 5)

- **SPA served by axum**: Vite builds the static assets, and the web binary
  embeds them with `include_dir!`. There is no Node process in production.
- **Routing**: a small History-API router (`lib/router.ts`). Every path that is
  not an API route serves `index.html`, so a deep link works on a fresh reload.
- **State**: Svelte 5 runes (`$state`, `$derived`) for the local state of a
  component, and a handful of stores for the session, the theme and the
  locale.
- **Data layer**: a typed `fetch` wrapper that attaches the CSRF token, parses
  the error envelope and throws an `ApiError`.
- **Realtime**: `lib/socket.ts` opens a WebSocket and reconnects on its own;
  the dashboard and the log viewer use it, everything else polls.
- **Components**: small, composable primitives in `src/lib/components`
  (`Card`, `StatCard`, `StatusBadge`, `EmptyState`, `Sparkline`, …).
- **Charts**: hand-written SVG sparklines — no chart library.
- **The terminal**: `xterm.js`, imported dynamically, so its ~68 kB gzip only
  reaches the browser when the terminal page is opened.
- **i18n**: every string that the navigation and the login use goes through
  the `$t` store. English is the reference locale.

### 9.4 Key screens

- **Dashboard**: stat cards, network chart, service list, audit feed, quick actions
- **Sites list**: status dot, domain, PHP version, SSL expiry badge, actions menu
- **Site detail**: tabs (Overview, SSL, Nginx, PHP, Logs, Backups, Cron, Files)
- **Forms**: inline validation, sensible defaults, "advanced" collapsibles
- **Empty states** with clear next steps ("No sites yet → Add your first site")

---

## 10. Roadmap & Milestones

The phases below are the ones that were planned; all of them shipped. The
per-phase detail and the remaining gaps live in
[`docs/roadmap.md`](./docs/roadmap.md).

| Phase | Scope | Exit criterion | Result |
|---|---|---|---|
| **0 — Foundations** | workspace, CI, the IPC protocol, the SQLite schema, the web skeleton, the installer, the SPA | a fresh server → login → the dashboard shows CPU and RAM | ✅ |
| **1 — Core hosting** | sites, the vhost engine, the apply pipeline with a rollback, SSL issuance, suspend, PHP versions, live metrics | create a site → it serves PHP over HTTPS; a broken config → the panel recovers | ✅ |
| **2 — Data & security** | MariaDB, PostgreSQL, Redis, the firewall, fail2ban, SSH keys, users and roles, the audit log | the whole stack is manageable end to end | ✅ |
| **3 — Operations** | backups, cron, the log viewer, the file manager, API keys, the scheduler | backup → delete the site → restore works | ✅ |
| **4 — v1.0 hardening** | the signed self-update, the `.deb` and the apt repository, Playwright, the docs site, i18n and accessibility | the upgrade path is verified end to end | ✅ |
| **5 — Beyond v1.0** | alerting, Git deployments, the web terminal, the PWA | — | ✅ |
| **6 — Panel polish** | the PHP page, the panel certificate, the 2FA QR code | — | ✅ |

### Still open

- The multi-server agent and the plugin API need a protocol that does not exist
  yet; the IPC contract was designed with that in mind (§5.10).
- DNS zone management, if there is demand for it.
- An OpenAPI document, a code editor, a session list and the quota enforcement.
- An external security audit and reproducible builds.

## 11. Testing & Quality

| Level | Tooling | Focus | Status |
|---|---|---|---|
| Backend unit | `cargo test` | config rendering, parsers, crypto, the alerting and deployment engines, the terminal | ✅ |
| Frontend | `npm run check` | the Svelte and TypeScript types | ✅ |
| Integration | `tests/smoke.sh` | the whole panel over the real API: login, sites, backups, cron, logs, files, API keys, alerts, deployments, the terminal | ✅ |
| Chaos | `tests/chaos.sh` | a failing `nginx -t` must never reach the server | ✅ |
| Upgrade | `tests/update.sh` | a signed release, the apply, the history, a tampered envelope | ✅ |
| E2E | `e2e/run.sh` (Playwright) | the user journeys through a real browser | ✅ |
| Security | `cargo audit`, `cargo deny check` | vulnerable and disallowed dependencies | ✅ |
| Performance | the bundle budget | the SPA stays below 150 kB gzip | ✅ |
| Golden | insta snapshots of the generated configs | every template | ⬜ |
| Coverage | a coverage report | the config generation | ⬜ |

**Quality gates for a merge:** clippy is clean, the tests are green, the bundle
is inside its budget, and the changelog is updated.

## 12. Packaging & Distribution

| Channel | Status |
|---|---|
| Debian/Ubuntu `.deb` | ✅ `packaging/build-deb.sh` |
| A flat apt repository, optionally GPG-signed | ✅ `packaging/apt/publish.sh` |
| An offline tarball with all four binaries | ✅ `packaging/install.sh --offline` |
| Signed releases (Ed25519 envelope + SHA-256 checksum) | ✅ `packaging/sign-release.sh` |
| RHEL-family RPMs, the Arch AUR | ⬜ |

- **systemd units**: `synthpanel-core.service` and `synthpanel-web.service`.
  The scheduler runs inside the core, so there is no third unit to deploy.
- **Ports**: the panel speaks HTTPS on 8443/TCP. Nothing else is exposed —
  not even the terminal, which runs inside the core.
- **Paths**:
  - `/etc/synthpanel/` — the configuration, the secrets and the TLS material
  - `/var/lib/synthpanel/` — the SQLite database, the deployment workspaces,
    the ACME webroot
  - `/var/www/sp_<domain>/` — the site roots
  - `/var/log/synthpanel/` — the panel logs
  - `/var/backups/synthpanel/` — the local backups
  - `/run/synthpanel/` — the socket (the `/run` directory is a tmpfs)

## 13. Documentation & Community

| Item | Status |
|---|---|
| The docs site (VitePress): installation, quickstart, guides, the CLI and API reference, troubleshooting, the security model | ✅ |
| The API reference is written by hand | ✅ (an OpenAPI document is planned) |
| A contributing guide | ⬜ |
| A changelog with the release notes | ⬜ |
| `SECURITY.md` with the private disclosure process | ⬜ (the address is in the README) |
| i18n: English first, a dictionary per locale | ✅ the groundwork, one locale so far |

## 14. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The privileged core is compromised | Low | Critical | a Unix socket only, a typed IPC, the audit log, memory-safe Rust |
| A generated Nginx config breaks the server | Medium | High | `nginx -t` before the reload, an automatic rollback, and `tests/chaos.sh` proves it |
| Distro and package drift (PHP, Nginx versions) | Medium | Medium | the versions are detected, not assumed; the panels are per version |
| Let's Encrypt rate limits or outages | Medium | Low | the issuance is manual, the expiry is reported, and the certificate is only replaced when it is valid |
| SQLite contention | Low | Medium | WAL mode, a single writer, timeouts |
| The panel is a single point of failure for management | Medium | Low | the sites keep serving when the panel dies, and the CLI still works |
| An upgrade breaks the installation | Medium | High | back up first, health-check after, roll back automatically |
| Scope creep (mail, Docker, clusters) | High | Medium | the explicit non-goals in §2.2 |

## 15. Success Metrics

| Metric | Target | Measured |
|---|---|---|
| Cold install → the panel is reachable | ≤ 5 min | — |
| Idle memory (web + core) | < 150 MB | ~32 MB |
| Create a site → it serves over HTTPS | ≤ 60 s | — |
| The SPA bundle | < 150 kB gzip | 57 kB (the terminal is a separate, lazy chunk) |
| The core test suite | as much as possible | 106 tests |
| Known critical CVEs in the dependencies | 0 | `cargo audit` in CI |

## 16. Decisions

| Question | Decision |
|---|---|
| License | **AGPL-3.0-or-later** — copyleft protection against SaaS repackaging |
| The initial distributions | **Ubuntu LTS (22.04/24.04) and Debian 12 only**; the RPM family later, if there is demand |
| PostgreSQL and Redis | **built in from the start** — the whole database stack is managed from the panel |
| Panel TLS | **self-signed at install**; a Let's Encrypt certificate can be requested from Settings once the hostname and the DNS are in place |
| The ACME client | **acme.sh** instead of an embedded client — one well-known tool, fewer dependencies to audit |
| The web terminal | **a pseudo-terminal inside the core**, not an SSH server — the panel opens exactly one port |
| The template engine | **none** — the config templates are small and fully covered by unit tests |
| The frontend dependencies | **as few as possible** — the router, the charts and the i18n are hand-written, and xterm.js is loaded on demand |
| phpMyAdmin and pgAdmin | **not bundled** — the scope is the panel itself |
| Telemetry | **none** — no telemetry, no install statistics, no calls home |
| Multi-server | **single-server for now**, but the IPC is designed to be agent-ready (§5.10), so it needs no rewrite later |

*This document is a living one: the status markers in §4 are updated with every
release, and [`docs/roadmap.md`](./docs/roadmap.md) holds the per-phase detail
and the remaining gaps.*
