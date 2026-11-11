# SynthPanel

**A free, open-source control panel for a single Linux server.**

SynthPanel installs and manages a complete LEMP stack — Nginx, multi-version
PHP-FPM, MariaDB/PostgreSQL, Redis — and lets you operate it from a clean web
UI, a REST API or a CLI.

It is written in **Rust** (backend) with a **Svelte 5** SPA (frontend), and it
runs as two processes: a small privileged core that owns the system state, and
an unprivileged web process that talks to it over a Unix socket.

---

## Contents

- [What it does](#what-it-does)
- [Install](#install)
- [First steps](#first-steps)
- [The three interfaces](#the-three-interfaces)
- [Architecture](#architecture)
- [Development](#development)
- [Testing](#testing)
- [Repository layout](#repository-layout)
- [Security](#security)
- [License](#license)

---

## What it does

| Area | What you get |
| --- | --- |
| **Sites** | One domain = one Nginx vhost + one isolated PHP-FPM pool. Templates for static, PHP, Laravel, Symfony, WordPress and Node. Suspend, custom directives, per-site `php.ini` overrides. |
| **SSL** | Let's Encrypt over HTTP-01 for every site *and* for the panel itself, custom certificates, expiry warnings. |
| **Databases** | MariaDB/MySQL, PostgreSQL and Redis. The database is provisioned on the server first, then recorded in the panel — the panel state never drifts from reality. |
| **Backups** | Full and per-site archives (`.tar.gz` + manifest), optional ChaCha20-Poly1305 encryption, verification and restore, to a local disk, SFTP or an S3-compatible store. |
| **Cron** | Five-field expressions with a human-readable summary, written to `/etc/cron.d/synthpanel`, with a run log and a run-now button. |
| **Logs** | Nginx access/error, PHP-FPM, panel and system auth logs, with filters and a live tail over a WebSocket. |
| **File manager** | A sandboxed browser for the web root, the panel configuration, the backups and the logs. Archives, permissions, uploads — path traversal is rejected. |
| **Security** | UFW rules, fail2ban jails, SSH keys, TOTP two-factor with a QR code, an audit log of every mutating action. |
| **Deployments** | A Git repository per site: clone, build, publish, and roll back automatically when a step fails. |
| **Alerts** | Webhook and e-mail channels with rules per event, a cooldown and a delivery history. |
| **Terminal** | A root shell in the browser. It runs in a pseudo-terminal inside the core — the panel deliberately does not open an SSH port. |
| **Updates** | A signed self-update: download, verify the Ed25519 signature and the SHA-256 checksum, back up the binaries, replace them, health-check, roll back on failure. |
| **API keys** | `sp_<prefix>_<secret>` bearer tokens with scopes, stored as SHA-256 hashes. |
| **PWA** | A web application manifest and a service worker, so the panel installs and opens instantly. |

Everything above is available in the **UI**, the **REST API** and the **CLI**.

## Install

### Debian / Ubuntu

```sh
curl -sSfL https://get.synthpanel.io | sudo bash
```

The installer detects the distribution, installs Nginx, PHP-FPM, MariaDB,
PostgreSQL and Redis, creates the `synthpanel` system user, writes the
systemd units, applies a firewall baseline and bootstraps an admin account.

### From a package

```sh
sudo dpkg -i synthpanel_0.1.0_amd64.deb
```

or, once the apt repository is configured:

```sh
sudo apt install synthpanel
```

### From source

```sh
git clone https://github.com/binkapS/synthpanel
cd synthpanel
cargo build --release --workspace
VERSION=0.1.0 packaging/build-deb.sh
sudo dpkg -i target/debian/synthpanel_0.1.0_amd64.deb
```

The panel needs a frontend build before the web crate compiles — it embeds
the SPA:

```sh
cd crates/synthpanel-web/frontend && npm ci && npm run build
```

### Requirements

- Debian 12 or Ubuntu 22.04/24.04, x86-64 or arm64
- Root access
- A hostname that resolves to the server (needed for a certificate)
- Ports 80 (certificate validation) and 8443 (the panel)

## First steps

The panel listens on `https://<hostname>:8443`. On the first start it
generates a self-signed certificate, so the browser will ask you to accept it.

Create the first administrator from the command line — the CLI talks to the
core over its Unix socket and therefore needs the IPC token:

```sh
sudo synthpanel users:create you@example.com --name "Your Name" --role admin
```

Then:

1. Open `https://<hostname>:8443/` and sign in.
2. **Settings → General** — set the hostname, the timezone and the locale.
3. **Settings → Panel certificate** — request a Let's Encrypt certificate for
   the panel itself.
4. **Sites → Add site** — your first domain gets a vhost and a PHP-FPM pool.
5. **Users → New user** — invite your team. Roles are `admin`, `operator` and
   `user`; quotas limit the sites and databases per account.

## The three interfaces

Every action exists in all three, driven by the same core.

**Web UI** — Dashboard, Sites, Databases, PHP, Security, Backups, Cron, Logs,
Files, Deployments, Alerts, Terminal, Users and Settings.

**REST API** — `/api/v1`, JSON, session cookie or bearer token:

```sh
curl -sk https://127.0.0.1:8443/api/v1/sites \
  -H "Authorization: Bearer sp_abc123_secret"
```

**CLI** — the same operations from a shell, with `--output json` for scripts:

```sh
sudo synthpanel status
sudo synthpanel sites:list
sudo synthpanel backup:run --site <site-id>
sudo synthpanel logs --type nginx-error --search timeout
sudo synthpanel update --check
```

The full reference lives in the [documentation site](./docs-site) and in
[`docs-site/reference/cli.md`](./docs-site/reference/cli.md).

## Architecture

```
                  ┌───────────────────────────────────────┐
   browser ──────▶│ synthpanel-web  (unprivileged user)   │
                  │  axum + rustls · REST API · SPA        │
                  └───────────────┬───────────────────────┘
                                  │  Unix socket + shared token
                                  │  (typed JSON, one request per line)
                  ┌───────────────▼───────────────────────┐
                  │ synthpanel-core  (root)               │
                  │  nginx · php-fpm · databases · backups  │
                  │  cron · logs · files · deployments      │
                  └───────────────────────────────────────┘
```

- Only the core runs as root. It listens on a Unix socket — never on TCP —
  and requires the shared token from `/etc/synthpanel/ipc-token` on every
  request.
- The web process cannot touch the system at all; it can only ask the core to
  do something through its typed API.
- The CLI is a third client of the same API.
- Every configuration change goes through the same pipeline:
  **render → lint → backup → write → validate → reload**, and a failed
  validation restores the previous file. A broken configuration never reaches
  the server.

## Development

```sh
# The frontend (the web crate embeds the built SPA)
cd crates/synthpanel-web/frontend && npm ci && npm run build && cd -

# The workspace
cargo build --workspace
cargo test --workspace
```

Run the core and the web process side by side without root — every path can be
overridden with an environment variable:

```sh
SYNTHPANEL_ETC_DIR=/tmp/sp/etc SYNTHPANEL_LIB_DIR=/tmp/sp/lib \
SYNTHPANEL_LOG_DIR=/tmp/sp/log SYNTHPANEL_RUN_DIR=/tmp/sp/run \
SYNTHPANEL_WWW_DIR=/tmp/sp/www SYNTHPANEL_BACKUP_DIR=/tmp/sp/backups \
cargo run -p synthpanel-core

SYNTHPANEL_TLS_DIR=/tmp/sp/tls SYNTHPANEL_CORE_SOCKET=/tmp/sp/run/core.sock \
SYNTHPANEL_IPC_TOKEN_FILE=/tmp/sp/etc/ipc-token SYNTHPANEL_WEB_ADDR=127.0.0.1:8443 \
SYNTHPANEL_SECURE_COOKIES=0 cargo run -p synthpanel-web
```

The environment variables are the same in production, so the installer and
the systemd units set them explicitly:

| Variable | Default |
| --- | --- |
| `SYNTHPANEL_ETC_DIR` | `/etc/synthpanel` |
| `SYNTHPANEL_LIB_DIR` | `/var/lib/synthpanel` |
| `SYNTHPANEL_LOG_DIR` | `/var/log/synthpanel` |
| `SYNTHPANEL_RUN_DIR` | `/run/synthpanel` |
| `SYNTHPANEL_WWW_DIR` | `/var/www` |
| `SYNTHPANEL_BACKUP_DIR` | `/var/backups/synthpanel` |
| `SYNTHPANEL_NGINX_DIR` | `/etc/synthpanel/nginx` |
| `SYNTHPANEL_PHP_DIR` | `/etc/synthpanel/php` |
| `SYNTHPANEL_CRON_DIR` | `/etc/cron.d` |
| `SYNTHPANEL_TLS_DIR` | `/etc/synthpanel/tls` |
| `SYNTHPANEL_SOCKET_PATH` | `/run/synthpanel/core.sock` |
| `SYNTHPANEL_IPC_TOKEN_FILE` | `/etc/synthpanel/ipc-token` |
| `SYNTHPANEL_WEB_ADDR` | `0.0.0.0:8443` |
| `SYNTHPANEL_WEB_PORT` | `8443` |
| `SYNTHPANEL_PANEL_HOSTNAME` | `localhost` |
| `SYNTHPANEL_SECURE_COOKIES` | `1` |

## Testing

| Command | What it covers |
| --- | --- |
| `cargo test --workspace` | Unit tests: config rendering, parsers, crypto, the alerting engine, the deployment engine, the terminal |
| `bash tests/smoke.sh` | The whole panel end to end: login, sites, backups, cron, logs, files, API keys, alerts, deployments, terminal |
| `bash tests/chaos.sh` | The apply pipeline: a failing `nginx -t` must never reach the server |
| `bash tests/update.sh` | The signed self-update: apply, history, tampered envelope rejection |
| `bash e2e/run.sh` | Playwright: the user journeys through a real browser |
| `npm run build` (frontend) | The bundle stays below 150 kB gzip |

The quality gates that run in CI: `cargo fmt --check`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo test --workspace`,
`cargo audit`, `cargo deny check`, the frontend build, the integration tests
and the docs build. See [`.github/workflows/ci.yml`](./.github/workflows/ci.yml).

## Repository layout

```
crates/
  synthpanel-ipc        the protocol shared by core, web and CLI
  synthpanel-core       the privileged daemon (root, Unix socket only)
  synthpanel-web        the REST API and the embedded Svelte 5 SPA
  synthpanel-cli        the `synthpanel` command-line interface
  synthpanel-installer  the one-command bootstrap
packaging/
  build-deb.sh          builds the .deb
  apt/publish.sh        publishes a flat apt repository
  sign-release.sh       signs a release with Ed25519
  systemd/              the unit files
docs-site/              the VitePress documentation site
e2e/                    the Playwright suite
tests/                  the smoke, chaos and update scripts
docs/roadmap.md         what is implemented and what is not
PLAN.md                 the design document
```

## Security

- Only `synthpanel-core` runs as root, and it speaks to nobody but a Unix
  socket guarded by a shared token.
- Passwords are hashed with argon2id; sessions live in memory; the browser
  gets an HttpOnly cookie and a double-submit CSRF token.
- Two-factor authentication is TOTP (RFC 6238) with a QR code.
- Every mutating action is written to the audit log.
- Self-updates are verified against an Ed25519 signature and a SHA-256
  checksum before a single byte is replaced, and a failed health check rolls
  the previous binaries back.
- Report a vulnerability privately to `security@synthpanel.io` instead of
  opening a public issue.

## License

AGPL-3.0-or-later — see [LICENSE.md](./LICENSE.md).

Because of the copyleft, if you run a modified version of SynthPanel as a
service for other people, you have to offer them the corresponding source of
your version.
