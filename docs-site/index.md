# SynthPanel

A free, open-source control panel for single Linux servers. SynthPanel installs
and manages a complete LEMP stack — Nginx, multi-version PHP-FPM,
MariaDB/PostgreSQL, Redis — and exposes it through a clean web UI, a REST API
and a CLI.

## Why SynthPanel

- **One privileged core, one small surface.** The root daemon only speaks a
  typed JSON protocol over a Unix socket. The web process is unprivileged and
  never touches system state directly.
- **Never break the server.** Every config change is rendered, linted, written,
  validated with `nginx -t` / `php-fpm -t`, and rolled back automatically when
  validation fails.
- **Everything is scriptable.** Every UI action has an API endpoint and a CLI
  command.
- **Rust + SQLite.** A single static binary pair and an embedded database.

## Start here

- [Installation](/guide/installation)
- [Quickstart](/guide/quickstart)
- [CLI reference](/reference/cli)
- [REST API](/reference/api)
- [Security model](/reference/security)
