# SynthPanel vs CloudPanel

SynthPanel is inspired by CloudPanel's idea of a small, focused panel for one
Linux server. This page states what is actually in this tree today, what is
deliberately out of scope, and where SynthPanel takes a different approach. For
the exact per-feature status, see [`docs/roadmap.md`](../../docs/roadmap.md) and
[PLAN.md §4](../../PLAN.md#4-feature-map).

## Shipped today

| Area | In this tree |
| --- | --- |
| Web stack | Nginx-only vhosts, per-site PHP-FPM pools and dedicated system users, multiple side-by-side PHP versions, MariaDB/MySQL, PostgreSQL and Redis |
| Sites | Create, edit, suspend/unsuspend, delete; custom Nginx directives validated before apply; static and PHP sites |
| SSL | Let's Encrypt over HTTP-01 for sites and for the panel itself, expiry status and warnings |
| Databases | Create/drop databases and users, generated credentials, connection strings and `.env` snippets, engine health; the server is provisioned before the panel records it |
| Security | UFW firewall rules, fail2ban jails and unban, SSH keys, TOTP 2FA with a QR code, rate-limited login, audit log |
| Operations | Encrypted/verified/restorable backups (local, SFTP, S3-compatible), cron with run history, log viewers with a live WebSocket tail, sandboxed file manager |
| Automation | Git deployments with automatic rollback, webhook/e-mail alerts with rules and cooldowns, a browser terminal, signed self-update with rollback, REST API v1, CLI and API keys |
| Architecture | Unprivileged web process + root core over a token-guarded Unix socket; apply pipeline that lints, backs up, validates and rolls back a bad config |

## In progress (do not assume these are complete)

These are the current CloudPanel-parity workstream. A CLI verb or an IPC method
is **not** proof of a finished UI screen.

| Feature | State |
| --- | --- |
| Reverse-proxy site type | CLI verb and vhost renderer landed; IPC/UI wiring in progress |
| Per-site IP/bot blocks, basic auth, Cloudflare-only | CLI, IPC and REST routes landed; UI wiring in progress |
| Per-site SSH users | CLI, IPC and REST routes landed; UI wiring in progress |
| Custom certificate install | CLI, IPC and REST route landed; UI upload in progress |
| On-demand DB dump/import | CLI, IPC and REST routes landed; UI wiring in progress |
| Browser session list and revoke-others | REST routes landed; web UI in progress; no CLI verb (by design) |
| Per-site user ACL | CLI and core work in progress; no REST route in v1 |
| **Python and Node application sites** | **In progress.** The CLI accepts template names and the renderer has static/PHP paths, but do not treat Python/Node app sites as shipped until the runtime templates and UI options land. |

## Intentionally out of scope

These are explicit non-goals, not oversights (see PLAN §2.2):

- **Apache** and `.htaccess`: Nginx only, so there is one template and test surface.
- **A mail server** (Postfix/Dovecot): use an external provider.
- **Docker/container management**: available through SSH if you want it, not in the panel.
- **Multi-server management in v1**: the IPC is designed to be agent-ready, but the v1 panel manages one server.
- **Windows/macOS and RHEL-family packaging**: Debian 12 and Ubuntu LTS only for now.
- **cPanel-style breadth**: no billing, DNS zones, phpMyAdmin/pgAdmin bundles or plugin API in v1.
- **A code editor with syntax highlighting** in the file manager, and **OS package updates** from the UI: planned, not shipped.

Several other features are partial by design: the site quota is enforced while
the database quota is still being wired; `maintenance_mode` can be stored but
is not enforced; the SFTP/S3 backup destinations are implemented but have not
been exercised against a live server in CI.

## Where SynthPanel takes a different path

These are design choices, not claims that the other panel lacks them:

- **Memory-safe core.** Rust backend; the privileged daemon exposes only typed
  IPC operations over a local Unix socket, with no shell passthrough and no
  TCP listener. The unprivileged web process owns sessions, CSRF and validation.
- **Never break the server.** Every config change goes through
  render → lint → backup → write → validate → reload, with automatic rollback;
  `tests/chaos.sh` proves that a failing `nginx -t` never reaches the server.
- **Backups you can verify.** Archives are checksummed, optionally encrypted
  with ChaCha20-Poly1305, and can be verified and restored from the UI or CLI.
- **Signed updates.** Ed25519 signature plus SHA-256 checksum, binary backup,
  health check and rollback on failure.
- **No Node.js in production.** The UI is a Slint WebAssembly app served by the
  panel itself; a small JS loader bridge handles browser primitives only.
- **No second port for the terminal.** The shell is a pseudo-terminal inside the
  core, reachable through the authenticated API and restricted to admins.
- **API-first and scriptable.** The same operations are available in the UI,
  REST API and CLI; `--output json` and the published
  [OpenAPI 3.1 document](/openapi.json) make automation straightforward.
- **No telemetry, no paid tiers.** No install statistics or calls home; AGPL-3.0-or-later.

For a hands-on start, follow [Installation](./installation) and the
[Quickstart](./quickstart).
