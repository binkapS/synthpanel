# SynthPanel

SynthPanel is a free, open-source control panel for a single Linux server.
It installs and manages a complete LEMP stack — Nginx, multi-version PHP-FPM,
MariaDB/PostgreSQL, Redis — and lets you operate it from a clean web UI, a
REST API, or a CLI.

> **Status: Phase 4 (v1.0 hardening).** Sites, databases, security, backups,
> cron, logs, the file manager, API keys and signed self-update are implemented.
> See [PLAN.md](./PLAN.md) for the full design and
> [docs/roadmap.md](./docs/roadmap.md) for what is implemented and what is
> still to come.

## Layout

```
crates/
  synthpanel-ipc       shared IPC protocol (core <-> web/cli)
  synthpanel-core      privileged daemon (root, Unix-socket only)
  synthpanel-web       web UI + REST API (unprivileged user)
  synthpanel-cli       user / root CLI
  synthpanel-installer one-command bootstrap
packaging/
  systemd/             unit files
  debian/              .deb control
  apt/                 flat apt repository publisher
  build-deb.sh         .deb builder
  sign-release.sh      Ed25519 release signer
  install.sh           curl | sudo bash bootstrap
e2e/                   Playwright end-to-end tests
docs-site/             VitePress documentation site
tests/                 smoke, update and integration scripts
frontend/              embedded Svelte 5 SPA (inside synthpanel-web)
```

## Quick start (developers)

```sh
# 1. Core (root or as a non-root user with env overrides for a sandbox):
SYNTHPANEL_ETC_DIR=/tmp/sp/etc \
SYNTHPANEL_LIB_DIR=/tmp/sp/lib \
SYNTHPANEL_LOG_DIR=/tmp/sp/log \
SYNTHPANEL_RUN_DIR=/tmp/sp/run \
SYNTHPANEL_WWW_DIR=/tmp/sp/www \
SYNTHPANEL_BACKUP_DIR=/tmp/sp/backups \
cargo run -p synthpanel-core

# 2. Web (in another shell):
SYNTHPANEL_TLS_DIR=/tmp/sp/tls \
SYNTHPANEL_CORE_SOCKET=/tmp/sp/run/core.sock \
SYNTHPANEL_IPC_TOKEN_FILE=/tmp/sp/etc/ipc-token \
SYNTHPANEL_WEB_ADDR=127.0.0.1:8443 \
SYNTHPANEL_SECURE_COOKIES=0 \
cargo run -p synthpanel-web

# 3. CLI:
SYNTHPANEL_CORE_SOCKET=/tmp/sp/run/core.sock \
SYNTHPANEL_IPC_TOKEN_FILE=/tmp/sp/etc/ipc-token \
cargo run -p synthpanel-cli -- status
```

## Quality gates

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo audit`
- `cargo deny check`

See [.github/workflows/ci.yml](./.github/workflows/ci.yml).
