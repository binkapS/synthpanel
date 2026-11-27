# REST API

Base URL: `https://<panel>:8443/api/v1`

A machine-readable OpenAPI 3.1 document covering every route registered in
`crates/synthpanel-web/src/api/mod.rs` is available at
[`/openapi.json`](/openapi.json). It includes request schemas with optional
fields, role/scope requirements and the WebSocket upgrade paths as info
extensions.

## Authentication

- Browser: `synthpanel_session` cookie plus the `x-csrf-token` header (echoing
  the `synthpanel_csrf` cookie) on mutations
- Scripts: `Authorization: Bearer sp_<prefix>_<secret>`. A Bearer key takes
  precedence over the cookie and is exempt from CSRF.

`/auth/login` and `/auth/2fa/verify` are public; every other endpoint needs one
of the credentials above. Roles and API-key scopes are listed per operation in
the OpenAPI document. API keys are created in **Settings → API keys** or with
`synthpanel apikeys:create`. Only the SHA-256 hash is stored.

## Errors

```json
{ "error": { "code": "bad_request", "message": "…" } }
```

## Endpoints

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/auth/login` | Sign in |
| POST | `/auth/logout` | Sign out |
| GET | `/auth/me` | Current user |
| POST | `/auth/2fa/setup` · `/auth/2fa/enable` · `/auth/2fa/verify` · `/auth/2fa/disable` | TOTP setup and login flow |
| GET | `/auth/sessions` | List browser sessions (session cookie only) |
| POST | `/auth/sessions/revoke-others` | Revoke every other browser session |
| GET | `/metrics/snapshot` | Live metrics |
| GET | `/metrics/range?range=24h` | Metrics history |
| GET | `/services` | Service status |
| GET | `/audit?limit=50` | Audit log |
| GET/PATCH | `/settings` | Panel settings |
| GET/POST | `/sites` | List / create sites |
| GET/PATCH/DELETE | `/sites/{id}` | Site detail |
| POST | `/sites/{id}/ssl/issue` | Issue a certificate |
| GET | `/sites/{id}/ssl/status` | Certificate status |
| POST | `/sites/{id}/ssl/cert` | Install a custom certificate (PEM `cert`/`key`, optional `chain`) |
| GET/POST | `/sites/{id}/security` | Per-site IP/bot blocks, basic auth, Cloudflare-only |
| GET/POST | `/sites/{id}/ssh-users` | Per-site SSH users |
| DELETE | `/sites/{id}/ssh-users/{username}` | Remove a per-site SSH user |
| POST | `/sites/{id}/suspend` | Suspend a site |
| POST | `/sites/{id}/unsuspend` | Resume a site |
| GET | `/php/versions` · `/php/status` | PHP versions and installation status |
| GET | `/databases` | List databases |
| POST | `/databases` | Create a database |
| GET/DELETE | `/databases/{id}` | Database detail / drop |
| POST | `/databases/{id}/dump` | Dump a database on demand |
| POST | `/databases/{id}/import` | Import a dump (`data`: base64 or raw SQL) |
| GET/POST | `/databases/{id}/users` | Database users |
| DELETE | `/databases/users/{id}` | Delete a database user |
| GET | `/security/firewall` | Firewall rules |
| GET | `/backups` | Archives and schedules |
| POST | `/backups/run` | Run a backup |
| POST | `/backups/{id}/verify` | Verify an archive |
| POST | `/backups/{id}/restore` | Restore an archive |
| GET | `/backups/{id}/download` | Download an archive |
| GET/POST | `/cron` | List / create cron jobs |
| POST | `/cron/{id}/run` | Run now |
| GET | `/cron/{id}/runs` | Run history |
| GET | `/logs?type=…` | Read a log source |
| GET | `/files?scope=…&path=…` | List a directory |
| GET/PUT | `/files/content` | Read / write a file |
| GET/POST | `/api-keys` | List / create API keys |
| DELETE | `/api-keys/{id}` | Revoke an API key |
| GET | `/alerts` | Alert history |
| GET/POST | `/alerts/channels` | Alert channels |
| DELETE | `/alerts/channels/{id}` | Delete a channel |
| GET/POST | `/alerts/rules` | Alert rules |
| DELETE | `/alerts/rules/{id}` | Delete a rule |
| POST | `/alerts/test` | Send a test alert |
| GET/POST | `/deployments` | Git deployments |
| PATCH/DELETE | `/deployments/{id}` | Change or delete a deployment |
| POST | `/deployments/{id}/run` | Run a deployment |
| GET | `/deployments/{id}/runs` | Run history |
| POST | `/terminal/open` | Open a terminal session |
| GET | `/terminal/{id}/read` | Read the terminal output |
| POST | `/terminal/{id}/write` | Send input to the terminal |
| POST | `/terminal/{id}/resize` | Resize the terminal |
| DELETE | `/terminal/{id}` | Close the session |
| GET | `/update` | Update status |
| POST | `/update/check` | Check for updates |
| POST | `/update/apply` | Install an update |
| POST | `/update/rollback` | Roll back |
