#!/usr/bin/env bash
# End-to-end smoke test for Phase 0.
#
# Starts the core and web binaries with environment overrides so the test
# can run without root, waits for the panel to come up, then exercises the
# unauthenticated response paths against the real HTTP server.
#
# Run:  tests/smoke.sh

set -eu

cd "$(dirname "$0")/.."

WORK="$(mktemp -d /tmp/sp-smoke.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT INT TERM

log() { printf '==> %s\n' "$*"; }

# Capture only the status code so curl does not abort on 4xx/5xx.
code() { curl -sSk -o /dev/null -w '%{http_code}' "$@"; }

log "building release binaries"
cargo build --release -p synthpanel-core -p synthpanel-web

CORE="$WORK/core.log"
WEB="$WORK/web.log"

SYNTHPANEL_ETC_DIR="$WORK/etc" \
SYNTHPANEL_LIB_DIR="$WORK/lib" \
SYNTHPANEL_LOG_DIR="$WORK/log" \
SYNTHPANEL_RUN_DIR="$WORK/run" \
SYNTHPANEL_WWW_DIR="$WORK/www" \
SYNTHPANEL_BACKUP_DIR="$WORK/backups" \
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
SYNTHPANEL_WEB_ADDR="127.0.0.1:8443" \
SYNTHPANEL_SECURE_COOKIES=0 \
SYNTHPANEL_PANEL_HOSTNAME=panel.local \
RUST_LOG=info \
  target/release/synthpanel-web > "$WEB" 2>&1 &
WEB_PID=$!

log "waiting for https://127.0.0.1:8443 to accept connections"
for i in $(seq 1 120); do
  if [ "$(code https://127.0.0.1:8443/healthz)" = "200" ]; then
    break
  fi
  sleep 0.5
done

if [ "$(code https://127.0.0.1:8443/healthz)" != "200" ]; then
  log "panel did not become reachable in 60s"; cat "$WEB"; exit 1
fi
log "panel reachable"

assert_contains() {
  case "$1" in
    *"$2"*) return 0 ;;
    *) log "missing '$2' in: $1"; exit 1 ;;
  esac
}

# SPA entry point
SPARK="$(curl -sSk https://127.0.0.1:8443/)"
log "SPA title: $(printf '%s' "$SPARK" | grep -oE '<title>[^<]*</title>')"
assert_contains "$SPARK" "SynthPanel"

# 401 on authenticated endpoints without a session cookie
UNAUTH_CODE="$(code https://127.0.0.1:8443/api/v1/metrics/snapshot)"
log "unauth metrics/snapshot status: $UNAUTH_CODE"
assert_contains "$UNAUTH_CODE" "401"

# 404 for an unknown API path
API_404="$(code https://127.0.0.1:8443/api/v1/nope)"
log "unknown API path status: $API_404"
assert_contains "$API_404" "404"

# 403 for unauthorised mutating endpoints (CSRF + session guard)
POST_CODE="$(code -X POST https://127.0.0.1:8443/api/v1/users)"
log "unauth POST /api/v1/users status: $POST_CODE"
assert_contains "$POST_CODE" "403"

log "smoke test passed"

# Tear down
kill "$WEB_PID" "$CORE_PID" 2>/dev/null || true
wait "$WEB_PID" "$CORE_PID" 2>/dev/null || true
exit 0
