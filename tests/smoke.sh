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

SYNTHPANEL_ETC_DIR="$WORK/etc" \
SYNTHPANEL_LIB_DIR="$WORK/lib" \
SYNTHPANEL_LOG_DIR="$WORK/log" \
SYNTHPANEL_RUN_DIR="$WORK/run" \
SYNTHPANEL_WWW_DIR="$WORK/www" \
SYNTHPANEL_BACKUP_DIR="$WORK/backups" \
SYNTHPANEL_NGINX_DIR="$WORK/nginx" \
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

# Invalid domains are rejected.
BAD="$(curl -sSk -o /dev/null -w '%{http_code}' -b "$COOKIES" -X POST \
  -H 'content-type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -d '{"domain":"not a domain"}' \
  $BASE/api/v1/sites)"
assert_eq "$BAD" "400"
log "invalid domain -> 400"

log "deleting the site"
DELETED="$(curl -sSk -b "$COOKIES" -X DELETE \
  -H "x-csrf-token: $CSRF" \
  "$BASE/api/v1/sites/$SITE_ID")"
assert_contains "$DELETED" '"removed":true'

if [ -f "$VHOST" ]; then
  fail "the vhost was not removed"
fi
log "site deleted and vhost removed"

# The audit log must contain the site operations.
AUDIT="$(auth_get "$BASE/api/v1/audit?limit=50")"
assert_contains "$AUDIT" "sites.create"
assert_contains "$AUDIT" "sites.delete"
log "audit log recorded the operations"

log "smoke test passed"
exit 0
