# REST API

Base URL: `https://<panel>:8443/api/v1`

## Authentication

- Browser: session cookie plus the `x-csrf-token` header on mutations
- Scripts: `Authorization: Bearer sp_<prefix>_<secret>`

API keys are created in **Settings → API keys** or with
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
| GET | `/metrics/snapshot` | Live metrics |
| GET | `/metrics/range?range=24h` | Metrics history |
| GET | `/services` | Service status |
| GET | `/audit?limit=50` | Audit log |
| GET/PATCH | `/settings` | Panel settings |
| GET/POST | `/sites` | List / create sites |
| GET/PATCH/DELETE | `/sites/{id}` | Site detail |
| POST | `/sites/{id}/ssl/issue` | Issue a certificate |
| GET | `/sites/{id}/ssl/status` | Certificate status |
| POST | `/sites/{id}/suspend` | Suspend a site |
| POST | `/sites/{id}/unsuspend` | Resume a site |
| GET | `/databases` | List databases |
| POST | `/databases` | Create a database |
| DELETE | `/databases/{id}` | Drop a database |
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
