# CLI reference

The `synthpanel` CLI talks to the core over `/run/synthpanel/core.sock` and
uses the token at `/run/synthpanel/ipc-token`. Run it with `sudo` on the server.

```text
synthpanel status
synthpanel metrics
synthpanel metrics:range --range 24h
synthpanel audit --limit 25
synthpanel settings

synthpanel sites:list
synthpanel sites:add example.com --aliases www.example.com --template php
synthpanel sites:show <id>
synthpanel sites:delete <id>

synthpanel users:list
synthpanel users:create you@example.com --name You --role admin

synthpanel db:list
synthpanel db:create app --engine mysql
synthpanel db:delete <id>

synthpanel security:firewall
synthpanel security:fail2ban
synthpanel security:ssh-keys

synthpanel backup:list
synthpanel backup:run [--site <id>] [--passphrase <secret>]
synthpanel backup:verify <id>
synthpanel backup:restore <id>

synthpanel cron:list
synthpanel cron:add "0 3 * * *" "php artisan schedule:run" --user www-data
synthpanel cron:run <id>
synthpanel cron:runs <id>

synthpanel logs --type panel --search error
synthpanel files:list --scope www --path site/
synthpanel files:read --scope www site/index.php
synthpanel files:write --scope www site/index.php --file index.php

synthpanel apikeys:list
synthpanel apikeys:create "CI deploy" --user <user-id> --scopes read,write
synthpanel apikeys:delete <id>

synthpanel update --check
synthpanel update:status
synthpanel update:rollback --id <update-id>

synthpanel alerts:list
synthpanel alerts:channels
synthpanel alerts:channel:add ops --kind webhook --config '{"url":"https://…"}'
synthpanel alerts:rule:add "failed backups" backup.failed --channel <id>
synthpanel alerts:test --channel <id>

synthpanel deploy:list
synthpanel deploy:add --site <site-id> --repo https://github.com/example/app.git
synthpanel deploy:run <deployment-id>
synthpanel deploy:runs <deployment-id>
```

All commands accept the global `--socket`, `--token-file` and
`--output json|table|yaml` options (see `synthpanel --help`).

## Extended verbs

These verbs operate on the extended feature set. The current per-feature status
is in `docs/roadmap.md` in the repository. The matching REST routes are
present in `/api/v1` — see the
[REST API](./api) and the [OpenAPI document](/openapi.json). Proxy site
creation is exposed through the ordinary site-create REST route with template
`proxy`; `user-sites:*` has no REST route in v1.

### Reverse-proxy sites

```sh
synthpanel sites:proxy app.example.com --target http://127.0.0.1:3000 \
  [--aliases www.app.example.com] [--ssl none|le|custom]
```

Creates a site with template `proxy` and a `proxy_target <url>;` directive.
The site UI exposes the proxy template.

### Per-site security controls

```sh
synthpanel sites:security:get --site <site-id>
synthpanel sites:security:set --site <site-id> \
  [--ip-block 10.0.0.8,192.0.2.0/24] [--bot-block wget,curl] \
  [--basic-auth-user deploy --basic-auth-password s3cret] \
  [--cloudflare-only]
```

`--ip-block` and `--bot-block` take comma-separated lists. The two basic-auth
options must be supplied together; supplying both enables basic auth. Only
fields actually supplied by the CLI are sent to the core.

### Per-site SSH users

```sh
synthpanel sites:ssh-users:list --site <site-id>
synthpanel sites:ssh-users:add --site <site-id> --username deploy \
  --public-key "$(cat id_ed25519.pub)"
synthpanel sites:ssh-users:remove --site <site-id> --username deploy
```

These accounts are key-only system users, separate from panel accounts.

### Custom certificate and database dumps

```sh
synthpanel ssl:cert:install --site <site-id> --cert site.crt --key site.key
synthpanel db:dump <db-id>
synthpanel db:import <db-id> --file dump.sql
```

`--cert` and `--key` are paths to local PEM files; their contents are sent over
IPC. `--file` is a local SQL dump sent as raw SQL (base64 is accepted by the
core as well).

### User-to-site assignments

```sh
synthpanel user-sites:list --user <user-id>
synthpanel user-sites:set --user <user-id> --sites <id-1>,<id-2>
# An empty --sites value clears the assignment.
```

These map to the core's `user_sites.*` methods. There is no CLI verb for
session revocation: sessions are listed and revoked in the web UI through
`GET /api/v1/auth/sessions` and `POST /api/v1/auth/sessions/revoke-others`.
