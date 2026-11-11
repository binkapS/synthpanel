# Roadmap

What is implemented in this tree, and what is not. The design behind it lives
in [PLAN.md](../PLAN.md); the per-feature status is in
[PLAN.md §4](../PLAN.md#4-feature-map).

| Phase | Scope | Status |
|---|---|---|
| 0 | Foundations: workspace, IPC, the schema, auth, the installer | ✅ complete |
| 1 | Core hosting: sites, vhosts, PHP-FPM pools, SSL, live metrics | ✅ complete |
| 2 | Data and security: MariaDB, PostgreSQL, Redis, UFW, fail2ban, SSH keys | ✅ complete |
| 3 | Operations: backups, cron, logs, the file manager, API keys | ✅ complete |
| 4 | v1.0 hardening: the signed self-update, packaging, e2e, the docs site | ✅ complete |
| 5 | Beyond v1.0: alerting, Git deployments, the web terminal, the PWA | ✅ complete |
| 6 | Panel polish: the PHP page, the panel certificate, the 2FA QR code | ✅ complete |

The phases below describe what each one delivered.

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

## Phase 3 — Operations (complete)

- Backups: full and per-site archives as a single `.tar.gz` with a
  `synthpanel-manifest.json`, optional ChaCha20-Poly1305 encryption,
  checksum verification, and restore (nginx vhosts, PHP-FPM pools, document
  roots). Destinations: local, SFTP (via `sftp`/`ssh`), and S3-compatible
  (hand-rolled SigV4). Schedules with retention pruning, plus
  verify/download/delete from the UI and CLI.
- Cron: five-field parser with human-readable descriptions, DB-backed jobs
  written to `/etc/cron.d/synthpanel`, run-now with exit code and bounded
  output, and per-job run history.
- Logs: nginx access/error, PHP-FPM, panel and system auth sources with
  time/level/status/search filters and a WebSocket tail that reads through
  the core.
- File manager: sandboxed scopes (`www`, `etc`, `backups`, `logs`) with list,
  read, write, mkdir, rename, delete, chmod/chown, upload/download, and
  tar.gz/zip archive + extract. Path traversal is rejected.
- API keys: `sp_<prefix>_<secret>` bearer tokens, SHA-256 at rest, scopes,
  last-used tracking, CSRF exemption for bearer clients, and revocation.
- Scheduler: a 60-second tick runs due cron jobs and backup schedules.
- REST API: `/api/v1/backups*`, `/api/v1/cron*`, `/api/v1/logs*`,
  `/api/v1/files*`, `/api/v1/api-keys*`.
- CLI: `backup:list|run|restore|verify`, `cron:list|add|delete|run|runs`,
  `logs`, `files:list|read|write`, `apikeys:list|create|delete`.
- SPA: Backups, Cron, Logs and Files pages, plus API-key management in
  Settings.
- End-to-end smoke test covers backup → delete site → restore, cron run
  history, filtered log reads, file manager traversal rejection, and API-key
  authentication/revocation.


## Phase 4 — v1.0 hardening (complete)

- Self-update: Ed25519-signed release envelope (`payload` + `signature`),
  SHA-256 archive verification, safe tar extraction, binary backup, unit
  restart, health check and automatic rollback. Exposed as
  `update.check|apply|status|rollback`, `synthpanel update*` and a Settings
  card. History is recorded in the `updates` table.
- Packaging: `packaging/build-deb.sh` produces a working `.deb` (binaries,
  systemd units, user/directory provisioning), and `packaging/apt/publish.sh`
  builds a flat apt repository (optionally GPG-signed). `sign-release.sh`
  signs a release and prints the trusted public key.
- End-to-end tests: Playwright covers login, live metrics, the site lifecycle,
  backups and cron through the real UI; `e2e/run.sh` boots a throwaway stack.
  `tests/update.sh` exercises a signed release, apply, history and tampered
  envelope rejection.
- Docs site: a VitePress site under `docs-site/` with installation,
  quickstart, backups, updating, troubleshooting, CLI, REST API and security
  pages.
- i18n scaffolding: `$t` store with English as the reference locale, a locale
  selector in Settings, and translated navigation and login copy.
- Accessibility: skip link, `#main` landmark, `aria-label` on navigation,
  `aria-current` on the active route, and labelled form controls.


## Phase 5 — Beyond v1.0 (complete, scoped)

The optional Phase 5 items from PLAN §10 are implemented, except the
multi-server agent and the plugin API (both need a protocol that does not
exist yet).

- Alerting: channels (webhook with an HMAC-SHA256 signature, or email over
  SMTP with AUTH and STARTTLS, or a local MTA via `sendmail`) and rules that
  match an event, a severity and a cooldown. Events: failed backups, failed
  cron jobs, failed and successful deployments, expiring certificates, a
  full disk and failed updates. Every delivery is recorded with its error.
- Git deployments: a site can be connected to a repository. The panel clones
  or updates it, runs an optional build command as the deploy user and
  publishes the result into the document root. The previous release is kept,
  so a failed build or a failed publish is rolled back automatically.
- Web terminal: the core allocates a pseudo-terminal and runs the shell
  inside it; the browser talks to it through a WebSocket. The panel
  deliberately does **not** open an SSH port — the terminal is only reachable
  through the authenticated API and only for administrators.
- PWA: a web application manifest, a service worker that caches the app
  shell (never the API), install icons and shortcuts.
- Chaos test: `tests/chaos.sh` puts a failing `nginx` on the PATH and proves
  that a broken change never reaches the server and that a broken new site
  leaves no vhost behind. This closes the gap that was still open after
  Phase 4.
- The error mapping between the core and the API was corrected: validation
  failures answer `400`, internal failures answer `500`.


## Phase 6 — Panel polish (complete)

- PHP page: the supported versions, the FPM binary of each version, the
  pools the panel generated and the sites that use them. The endpoint is
  `php.status` and the page replaces the old "coming soon" placeholder.
- Dashboard quick actions are real navigation again (add site, run backup,
  open terminal) instead of disabled buttons.
- Panel certificate: the panel can request a Let's Encrypt certificate for
  its own hostname (webroot or standalone), install it and serve it without
  a restart — the rustls resolver reloads the files when they change. The
  status shows the issuer, the expiry and whether the certificate is
  self-signed.
- ACME challenge endpoint: the panel serves
  `/.well-known/acme-challenge/<token>` from `/var/lib/synthpanel/acme-webroot`
  so the HTTP-01 validation can reach it.
- Two-factor setup shows a QR code (rendered server-side as an SVG) next to
  the secret.
- Certificate parsing bug: the PEM base64 payload is wrapped over several
  lines, which the decoder rejected — the expiry of a certificate was never
  readable. The decoder now ignores whitespace.

---

## What is not implemented yet

Everything below is either planned in [PLAN.md](../PLAN.md) or was cut for
scope. Nothing here is half-finished: either the feature works, or it is not
there.

### Planned

- A multi-server agent and a plugin API — they need a protocol that does not
  exist yet; the IPC was designed with them in mind.
- DNS zone management, if there is demand for it.
- An OpenAPI 3.1 document for the REST API.
- A code editor with syntax highlighting in the file manager.
- A session list where the active sessions can be revoked.
- The quota enforcement: the quotas are stored per user, but they are not
  checked when a site or a database is created.
- OS security updates (unattended-upgrades) from the UI.
- phpMyAdmin and pgAdmin as one-click installs.
- Golden snapshot tests of the generated configurations, and a coverage
  report.

### Known limitations

- The web terminal does not support `su` or `sudo` prompts that need a second
  TTY.
- The SMTP client has no implicit-TLS mode (port 465); use STARTTLS or a local
  MTA.
- Requesting a certificate for the panel needs port 80 to reach the panel
  (directly or through a proxy) and acme.sh to be installed.
- The panel certificate is trusted only when it comes from Let's Encrypt; a
  self-signed one always triggers a browser warning.
- The SFTP and S3 backup destinations are implemented but have never been run
  against a live server.
- The external security audit has not been run.
