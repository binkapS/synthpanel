#!/usr/bin/env bash
# End-to-end smoke test for SynthPanel.
#
# Starts the core and web binaries with environment overrides so the test can
# run without root, waits for the panel to come up, then exercises:
#   - the SPA entry point
#   - unauthenticated response codes
#   - an authenticated login through the real API
#   - the dashboard endpoints
#   - the full site lifecycle (create → list → show → delete)
#
# Run:  tests/smoke.sh

set -eu

cd "$(dirname "$0")/.."

WORK="$(mktemp -d /tmp/sp-smoke.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT INT TERM

log() { printf '==> %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; echo "--- web log ---"; tail -30 "$WEB" 2>/dev/null; echo "--- core log ---"; tail -30 "$CORE" 2>/dev/null; exit 1; }

# Capture only the status code so curl does not abort on 4xx/5xx.
code() { curl -sSk -o /dev/null -w '%{http_code}' "$@"; }

assert_contains() {
  case "$1" in
    *"$2"*) return 0 ;;
    *) fail "missing '$2' in: $1" ;;
  esac
}

assert_eq() {
  [ "$1" = "$2" ] || fail "expected '$2', got '$1'"
}

# Pick a free TCP port so parallel or repeated runs never collide.
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
BASE="https://127.0.0.1:$PORT"
log "using port $PORT"

log "building release binaries"
cargo build --release -p synthpanel-core -p synthpanel-web -p synthpanel-cli

CORE="$WORK/core.log"
WEB="$WORK/web.log"

# The apply pipeline validates the configuration with `nginx -t` and reloads
# it with `systemctl`. Both are stubbed so the test behaves the same on a
# developer machine, in CI and on a real server.
STUBS="$WORK/stubs"
mkdir -p "$STUBS"
cat > "$STUBS/nginx" <<'STUB'
#!/bin/sh
echo "nginx: the configuration file test is successful"
exit 0
STUB
cat > "$STUBS/systemctl" <<'STUB'
#!/bin/sh
exit 0
STUB
chmod +x "$STUBS/nginx" "$STUBS/systemctl"

PATH="$STUBS:$PATH" \
SYNTHPANEL_ETC_DIR="$WORK/etc" \
SYNTHPANEL_LIB_DIR="$WORK/lib" \
SYNTHPANEL_LOG_DIR="$WORK/log" \
SYNTHPANEL_RUN_DIR="$WORK/run" \
SYNTHPANEL_WWW_DIR="$WORK/www" \
SYNTHPANEL_BACKUP_DIR="$WORK/backups" \
SYNTHPANEL_NGINX_DIR="$WORK/nginx" \
SYNTHPANEL_PHP_DIR="$WORK/php" \
SYNTHPANEL_CRON_DIR="$WORK/cron" \
RUST_LOG=info \
  target/release/synthpanel-core > "$CORE" 2>&1 &
CORE_PID=$!

for i in $(seq 1 50); do
  [ -S "$WORK/run/core.sock" ] && break
  sleep 0.2
done
if [ ! -S "$WORK/run/core.sock" ]; then
  log "core did not bind the socket"; cat "$CORE"; exit 1
fi
sleep 0.5
log "core socket up"

SYNTHPANEL_TLS_DIR="$WORK/tls" \
SYNTHPANEL_CORE_SOCKET="$WORK/run/core.sock" \
SYNTHPANEL_IPC_TOKEN_FILE="$WORK/etc/ipc-token" \
SYNTHPANEL_WEB_ADDR="127.0.0.1:$PORT" \
SYNTHPANEL_SECURE_COOKIES=0 \
SYNTHPANEL_PANEL_HOSTNAME=panel.local \
RUST_LOG=info \
  target/release/synthpanel-web > "$WEB" 2>&1 &
WEB_PID=$!

log "waiting for $BASE to accept connections"
for i in $(seq 1 120); do
  if [ "$(code $BASE/healthz)" = "200" ]; then
    break
  fi
  sleep 0.5
done

if [ "$(code $BASE/healthz)" != "200" ]; then
  log "panel did not become reachable in 60s"; cat "$WEB"; exit 1
fi
log "panel reachable"

cleanup() {
  if [ -n "${KEEP_LOGS:-}" ]; then echo "logs: $WORK"; fi
  kill "$WEB_PID" "$CORE_PID" 2>/dev/null || true
  wait "$WEB_PID" "$CORE_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# ---------------------------------------------------------------------------
# Unauthenticated behaviour
# ---------------------------------------------------------------------------

SPARK="$(curl -sSk $BASE/)"
assert_contains "$SPARK" "SynthPanel"
log "SPA served"

assert_eq "$(code $BASE/api/v1/metrics/snapshot)" "401"
log "unauth metrics/snapshot -> 401"

assert_eq "$(code $BASE/api/v1/nope)" "404"
log "unknown API path -> 404"

assert_eq "$(code -X POST $BASE/api/v1/users)" "403"
log "unauth POST /api/v1/users -> 403"

# ---------------------------------------------------------------------------
# Bootstrap an admin user through the CLI (which talks to the core socket)
# ---------------------------------------------------------------------------

ADMIN_EMAIL="admin@example.com"
ADMIN_PASSWORD="hunter2password-77"

log "creating the admin user"
SYNTHPANEL_CORE_SOCKET="$WORK/run/core.sock" \
SYNTHPANEL_IPC_TOKEN_FILE="$WORK/etc/ipc-token" \
  target/release/synthpanel users:create "$ADMIN_EMAIL" \
    --name "Admin" \
    --password "$ADMIN_PASSWORD" \
    --role admin > "$WORK/user-create.json"
assert_contains "$(cat "$WORK/user-create.json")" "$ADMIN_EMAIL"
log "admin created"

# ---------------------------------------------------------------------------
# Authenticated flow
# ---------------------------------------------------------------------------

COOKIES="$WORK/cookies.txt"
rm -f "$COOKIES"

log "logging in"
LOGIN="$(curl -sSk -c "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -d "{\"email\":\"$ADMIN_EMAIL\",\"password\":\"$ADMIN_PASSWORD\"}" \
  $BASE/api/v1/auth/login)"
assert_contains "$LOGIN" '"requires_2fa":false'
log "login succeeded"

CSRF="$(awk '$6 == "synthpanel_csrf" { print $7 }' "$COOKIES")"
if [ -z "$CSRF" ]; then
  fail "no CSRF cookie was issued"
fi

auth_get() {
  curl -sSk -b "$COOKIES" "$@"
}

assert_eq "$(code -b "$COOKIES" $BASE/api/v1/auth/me)" "200"
log "auth/me -> 200"

ME="$(auth_get $BASE/api/v1/auth/me)"
assert_contains "$ME" "$ADMIN_EMAIL"
assert_contains "$ME" '"role":"admin"'

SNAPSHOT="$(auth_get $BASE/api/v1/metrics/snapshot)"
assert_contains "$SNAPSHOT" '"host"'
log "metrics snapshot returned"

SERVICES="$(auth_get $BASE/api/v1/services)"
assert_contains "$SERVICES" '"services"'

# ---------------------------------------------------------------------------
# Site lifecycle
# ---------------------------------------------------------------------------

log "creating a site"
SITE="$(curl -sSk -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{"domain":"example.com","aliases":["www.example.com"],"template":"php","php_version":"8.3","ssl_mode":"none"}' \
  $BASE/api/v1/sites)"
assert_contains "$SITE" '"domain":"example.com"'
assert_contains "$SITE" '"applied"'
log "site created"

SITE_ID="$(printf '%s' "$SITE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["site"]["id"])')"
log "site id: $SITE_ID"

LIST="$(auth_get $BASE/api/v1/sites)"
assert_contains "$LIST" "example.com"

SHOW="$(auth_get "$BASE/api/v1/sites/$SITE_ID")"
assert_contains "$SHOW" "example.com"
assert_contains "$SHOW" 'www.example.com'

# The generated vhost must exist on disk.
VHOST="$WORK/nginx/sites-available/synthpanel/$SITE_ID.conf"
if [ ! -f "$VHOST" ]; then
  fail "the vhost was not written to $VHOST"
fi
assert_contains "$(cat "$VHOST")" "server_name example.com www.example.com;"
assert_contains "$(cat "$VHOST")" "fastcgi_pass unix:/run/php/php8.3-sp-$SITE_ID.sock;"
log "vhost rendered correctly"

# ---------------------------------------------------------------------------
# Phase 3: operations — backups, cron, logs, files and API keys
# ---------------------------------------------------------------------------

DOC_ROOT="$WORK/www/sp_example.com"
mkdir -p "$DOC_ROOT"
printf 'hello from the backup\n' > "$DOC_ROOT/index.html"

log "running a site backup"
BACKUP="$(curl -sSk -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d "{\"kind\":\"site\",\"site_id\":\"$SITE_ID\"}" \
  $BASE/api/v1/backups/run)"
assert_contains "$BACKUP" '"name"'
assert_contains "$BACKUP" '"size"'
BACKUP_ID="$(printf '%s' "$BACKUP" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"
log "backup id: $BACKUP_ID"

BACKUPS="$(auth_get $BASE/api/v1/backups)"
assert_contains "$BACKUPS" "$BACKUP_ID"

log "verifying the backup"
VERIFIED="$(curl -sSk -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{}' \
  "$BASE/api/v1/backups/$BACKUP_ID/verify")"
assert_contains "$VERIFIED" '"verified":true'
assert_contains "$VERIFIED" '"entries"'
log "backup verified"

DOWNLOAD_CODE="$(code -b "$COOKIES" "$BASE/api/v1/backups/$BACKUP_ID/download")"
assert_eq "$DOWNLOAD_CODE" "200"
log "backup download -> 200"

log "creating a cron job"
CRON="$(curl -sSk -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{"expression":"0 3 * * *","command":"echo smoke-cron","user":"nobody"}' \
  $BASE/api/v1/cron)"
assert_contains "$CRON" '"id"'
CRON_ID="$(printf '%s' "$CRON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"
JOBS="$(auth_get $BASE/api/v1/cron)"
assert_contains "$JOBS" 'echo smoke-cron'
assert_contains "$JOBS" '"next_run"'

CRON_RUN="$(curl -sSk -b "$COOKIES" -X POST \
  -H "x-csrf-token: $CSRF" \
  "$BASE/api/v1/cron/$CRON_ID/run")"
assert_contains "$CRON_RUN" '"exit_code":0'
assert_contains "$CRON_RUN" 'smoke-cron'
CRON_RUNS="$(auth_get "$BASE/api/v1/cron/$CRON_ID/runs")"
assert_contains "$CRON_RUNS" '"runs"'
log "cron job created, ran and recorded"

DESCRIBE="$(auth_get "$BASE/api/v1/cron/describe?expression=0%203%20*%20*%20*")"
assert_contains "$DESCRIBE" '"summary"'

log "reading the panel log"
mkdir -p "$WORK/log"
printf '[error] smoke test log line\n' >> "$WORK/log/synthpanel-web.log"
LOG_LINES="$(auth_get "$BASE/api/v1/logs?type=panel&search=smoke")"
assert_contains "$LOG_LINES" 'smoke test log line'
LOG_SOURCES="$(auth_get "$BASE/api/v1/logs/sources")"
assert_contains "$LOG_SOURCES" 'nginx-access'
log "log viewer returned filtered lines"

log "exercising the file manager"
mkdir -p "$WORK/www/files-smoke"
curl -sSk -b "$COOKIES" -X PUT \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{"scope":"www","path":"files-smoke/hello.txt","content":"file manager smoke"}' \
  "$BASE/api/v1/files/content" > /dev/null
FILE_READ="$(auth_get "$BASE/api/v1/files/content?scope=www&path=files-smoke/hello.txt")"
assert_contains "$FILE_READ" 'file manager smoke'
FILE_LIST="$(auth_get "$BASE/api/v1/files?scope=www&path=files-smoke")"
assert_contains "$FILE_LIST" 'hello.txt'
TRAVERSAL="$(code -b "$COOKIES" "$BASE/api/v1/files?scope=www&path=../etc")"
assert_eq "$TRAVERSAL" "400"
log "file manager read, wrote and rejected traversal"

log "creating an API key"
API_KEY="$(curl -sSk -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{"name":"smoke","scopes":["read"]}' \
  $BASE/api/v1/api-keys)"
assert_contains "$API_KEY" '"token"'
API_TOKEN="$(printf '%s' "$API_KEY" | python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')"
API_KEY_ID="$(printf '%s' "$API_KEY" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"

BEARER_ME="$(curl -sSk -H "Authorization: Bearer $API_TOKEN" $BASE/api/v1/auth/me)"
assert_contains "$BEARER_ME" "$ADMIN_EMAIL"
log "API key authenticated a request without a session"

REVOKED="$(curl -sSk -b "$COOKIES" -X DELETE \
  -H "x-csrf-token: $CSRF" \
  "$BASE/api/v1/api-keys/$API_KEY_ID")"
assert_contains "$REVOKED" '"removed":true'
BEARER_AFTER="$(code -H "Authorization: Bearer $API_TOKEN" $BASE/api/v1/auth/me)"
assert_eq "$BEARER_AFTER" "401"
log "revoked API key -> 401"

# Invalid domains are rejected.
BAD="$(curl -sSk -o /dev/null -w '%{http_code}' -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{"domain":"not a domain"}' \
  $BASE/api/v1/sites)"
assert_eq "$BAD" "400"
log "invalid domain -> 400"

# ---------------------------------------------------------------------------
# Phase 5: alerting, deployments and the terminal
# ---------------------------------------------------------------------------

log "the PHP status is reported"
PHP_STATUS="$(auth_get $BASE/api/v1/php/status)"
assert_contains "$PHP_STATUS" '"versions"'
assert_contains "$PHP_STATUS" '"8.3"'

log "the panel certificate is reported"
PANEL_CERT="$(auth_get $BASE/api/v1/panel/ssl/status)"
assert_contains "$PANEL_CERT" '"hostname"'
assert_contains "$PANEL_CERT" '"path"'

log "the ACME challenge route rejects unknown tokens"
CHALLENGE_CODE="$(code "$BASE/.well-known/acme-challenge/does-not-exist")"
assert_eq "$CHALLENGE_CODE" "404"
CHALLENGE_TRAVERSAL="$(code "$BASE/.well-known/acme-challenge/..%2F..%2Fetc%2Fpasswd")"
assert_eq "$CHALLENGE_TRAVERSAL" "400"

log "creating an alert channel"
CHANNEL="$(curl -sSk -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{"name":"ops-webhook","kind":"webhook","config":{"url":"http://127.0.0.1:9/hook"}}' \
  $BASE/api/v1/alerts/channels)"
assert_contains "$CHANNEL" '"kind":"webhook"'
CHANNEL_ID="$(printf '%s' "$CHANNEL" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"

log "creating an alert rule"
RULE="$(curl -sSk -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d "{\"name\":\"failed backups\",\"event\":\"backup.failed\",\"severity\":\"critical\",\"channel_id\":\"$CHANNEL_ID\"}" \
  $BASE/api/v1/alerts/rules)"
assert_contains "$RULE" '"event":"backup.failed"'

log "listing the channels, rules and history"
CHANNELS="$(auth_get $BASE/api/v1/alerts/channels)"
assert_contains "$CHANNELS" "$CHANNEL_ID"
RULES="$(auth_get $BASE/api/v1/alerts/rules)"
assert_contains "$RULES" '"event":"backup.failed"'
ALERTS="$(auth_get $BASE/api/v1/alerts)"
assert_contains "$ALERTS" '"alerts"'

# An unreachable webhook must be reported instead of silently swallowed.
TEST_CODE="$(code -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d "{\"channel_id\":\"$CHANNEL_ID\"}" \
  $BASE/api/v1/alerts/test)"
assert_eq "$TEST_CODE" "500"
log "the alert test delivery reported the failure"

log "creating a deployment"
DEPLOY="$(curl -sSk -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d "{\"site_id\":\"$SITE_ID\",\"repo_url\":\"file:///nonexistent/repo.git\",\"branch\":\"main\",\"build_command\":\"\"}" \
  $BASE/api/v1/deployments)"
assert_contains "$DEPLOY" '"repo_url":"file:///nonexistent/repo.git"'
DEPLOY_ID="$(printf '%s' "$DEPLOY" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"

DEPLOYMENTS="$(auth_get $BASE/api/v1/deployments)"
assert_contains "$DEPLOYMENTS" "$DEPLOY_ID"

log "running the deployment against a missing repository"
RUN="$(curl -sSk -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{}' \
  "$BASE/api/v1/deployments/$DEPLOY_ID/run")"
assert_contains "$RUN" '"status":"failed"'
RUNS="$(auth_get "$BASE/api/v1/deployments/$DEPLOY_ID/runs")"
assert_contains "$RUNS" '"status":"failed"'
log "the failed deployment was recorded"

log "opening a terminal session"
SESSION="$(curl -sSk -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{"rows":24,"cols":80}' \
  $BASE/api/v1/terminal/open)"
assert_contains "$SESSION" '"id"'
SESSION_ID="$(printf '%s' "$SESSION" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"

log "running a command in the terminal"
curl -sSk -o /dev/null -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{"data":"echo synthpanel-terminal-ok\n"}' \
  "$BASE/api/v1/terminal/$SESSION_ID/write"

TERMINAL_OUT=""
for _ in $(seq 1 40); do
  TERMINAL_OUT="$(auth_get "$BASE/api/v1/terminal/$SESSION_ID/read")"
  case "$TERMINAL_OUT" in
    *synthpanel-terminal-ok*) break ;;
  esac
  sleep 0.25
done
assert_contains "$TERMINAL_OUT" "synthpanel-terminal-ok"

CLOSED="$(curl -sSk -b "$COOKIES" -X DELETE \
  -H "x-csrf-token: $CSRF" \
  "$BASE/api/v1/terminal/$SESSION_ID")"
assert_contains "$CLOSED" '"closed":true'
log "the terminal ran a command and was closed"

log "deleting the site"
DELETED="$(curl -sSk -b "$COOKIES" -X DELETE \
  -H "x-csrf-token: $CSRF" \
  "$BASE/api/v1/sites/$SITE_ID")"
assert_contains "$DELETED" '"removed":true'

if [ -f "$VHOST" ]; then
  fail "the vhost was not removed"
fi
log "site deleted and vhost removed"

# ---------------------------------------------------------------------------
# Restore the site backup after deleting the site
# ---------------------------------------------------------------------------

log "restoring the site backup"
RESTORED="$(curl -sSk -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{}' \
  "$BASE/api/v1/backups/$BACKUP_ID/restore")"
assert_contains "$RESTORED" '"restored"'

if [ ! -f "$VHOST" ]; then
  fail "the restore did not recreate the vhost at $VHOST"
fi
if [ ! -f "$DOC_ROOT/index.html" ]; then
  fail "the restore did not recreate the document root file"
fi
log "backup → delete site → restore works"

# The audit log must contain the site operations.
AUDIT="$(auth_get "$BASE/api/v1/audit?limit=50")"
assert_contains "$AUDIT" "sites.create"
assert_contains "$AUDIT" "sites.delete"
log "audit log recorded the operations"

# ---------------------------------------------------------------------------
# PHP versions, SSL status and the WebSocket channel
# ---------------------------------------------------------------------------

PHP="$(auth_get $BASE/api/v1/php/versions)"
assert_contains "$PHP" '"8.3"'
log "php versions listed"

# ---------------------------------------------------------------------------
# Databases and security
# ---------------------------------------------------------------------------

DBS="$(auth_get $BASE/api/v1/databases)"
assert_contains "$DBS" '"databases"'
assert_contains "$DBS" '"engines"'
log "databases listed"

# A database with an invalid name must be rejected before anything is
# provisioned on the server.
BAD_DB="$(curl -sSk -o /dev/null -w '%{http_code}' -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{"engine":"mysql","name":"bad name; DROP DATABASE"}' \
  $BASE/api/v1/databases)"
assert_eq "$BAD_DB" "400"
log "invalid database name -> 400"

FIREWALL="$(auth_get $BASE/api/v1/security/firewall)"
assert_contains "$FIREWALL" '"available"'

FAIL2BAN="$(auth_get $BASE/api/v1/security/fail2ban)"
assert_contains "$FAIL2BAN" '"jails"'

SSH_KEYS="$(auth_get $BASE/api/v1/security/ssh-keys)"
assert_contains "$SSH_KEYS" '"keys"'
log "security endpoints answered"

SSL="$(auth_get "$BASE/api/v1/sites/$SITE_ID/ssl/status" 2>/dev/null || true)"
# The site was deleted, so the status endpoint must answer 404.
SSL_CODE="$(code -b "$COOKIES" "$BASE/api/v1/sites/$SITE_ID/ssl/status")"
assert_eq "$SSL_CODE" "404"
log "ssl status for a missing site -> 404"

# The WebSocket requires an authenticated session; an anonymous upgrade must
# be rejected.
# The websocket keeps the connection open, so the request is aborted after a
# moment; curl still reports the handshake status.
ws_code() {
  curl -sSk --http1.1 --max-time 5 -o /dev/null -w '%{http_code}' "$@" 2>/dev/null || true
}

WS_CODE="$(ws_code -b "$COOKIES" -H "Connection: Upgrade" -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
  "$BASE/ws/dashboard")"
if [ "$WS_CODE" = "101" ] || [ "$WS_CODE" = "400" ]; then
  log "websocket upgrade answered $WS_CODE"
else
  fail "unexpected websocket status $WS_CODE"
fi

# An unauthenticated websocket upgrade must not succeed.
WS_ANON="$(ws_code -H "Connection: Upgrade" -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
  "$BASE/ws/dashboard")"
if [ "$WS_ANON" = "101" ]; then
  fail "the websocket accepted an unauthenticated client"
fi
log "unauthenticated websocket rejected"

log "smoke test passed"
exit 0
