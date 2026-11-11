#!/usr/bin/env bash
# Boot a throwaway SynthPanel stack and run the Playwright suite against it.
#
#   e2e/run.sh
#
# Requires: cargo, npm, and Playwright browsers (`npm run install-browsers`).
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

WORK=$(mktemp -d "${TMPDIR:-/tmp}/sp-e2e.XXXXXX")
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
BASE="https://127.0.0.1:$PORT"
EMAIL="admin@example.com"
PASSWORD="hunter2password-77"

CORE_LOG="$WORK/core.log"
WEB_LOG="$WORK/web.log"
CORE_PID=""
WEB_PID=""

cleanup() {
  [ -n "$WEB_PID" ] && kill "$WEB_PID" 2>/dev/null || true
  [ -n "$CORE_PID" ] && kill "$CORE_PID" 2>/dev/null || true
  wait 2>/dev/null || true
  if [ -n "${KEEP_LOGS:-}" ]; then
    echo "logs: $WORK"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT INT TERM

echo "==> building release binaries"
cargo build --release -p synthpanel-core -p synthpanel-web -p synthpanel-cli

echo "==> starting the core"
SYNTHPANEL_ETC_DIR="$WORK/etc" \
SYNTHPANEL_LIB_DIR="$WORK/lib" \
SYNTHPANEL_LOG_DIR="$WORK/log" \
SYNTHPANEL_RUN_DIR="$WORK/run" \
SYNTHPANEL_WWW_DIR="$WORK/www" \
SYNTHPANEL_BACKUP_DIR="$WORK/backups" \
SYNTHPANEL_NGINX_DIR="$WORK/nginx" \
SYNTHPANEL_PHP_DIR="$WORK/php" \
SYNTHPANEL_CRON_DIR="$WORK/cron" \
RUST_LOG=warn \
  target/release/synthpanel-core > "$CORE_LOG" 2>&1 &
CORE_PID=$!

for _ in $(seq 1 50); do
  [ -S "$WORK/run/core.sock" ] && break
  sleep 0.2
done
if [ ! -S "$WORK/run/core.sock" ]; then
  echo "core did not start" >&2
  cat "$CORE_LOG" >&2
  exit 1
fi

echo "==> creating the admin user"
SYNTHPANEL_CORE_SOCKET="$WORK/run/core.sock" \
SYNTHPANEL_IPC_TOKEN_FILE="$WORK/etc/ipc-token" \
  target/release/synthpanel users:create "$EMAIL" \
    --name "Admin" --password "$PASSWORD" --role admin > /dev/null

echo "==> starting the web UI"
SYNTHPANEL_TLS_DIR="$WORK/tls" \
SYNTHPANEL_CORE_SOCKET="$WORK/run/core.sock" \
SYNTHPANEL_IPC_TOKEN_FILE="$WORK/etc/ipc-token" \
SYNTHPANEL_WEB_ADDR="127.0.0.1:$PORT" \
SYNTHPANEL_SECURE_COOKIES=0 \
SYNTHPANEL_PANEL_HOSTNAME=panel.local \
RUST_LOG=warn \
  target/release/synthpanel-web > "$WEB_LOG" 2>&1 &
WEB_PID=$!

for _ in $(seq 1 120); do
  if curl -sSk -o /dev/null "$BASE/healthz"; then
    break
  fi
  sleep 0.5
done

cd "$ROOT/e2e"
if [ ! -d node_modules ]; then
  npm install --no-audit --no-fund
fi

SYNTHPANEL_E2E_BASE_URL="$BASE" \
SYNTHPANEL_E2E_EMAIL="$EMAIL" \
SYNTHPANEL_E2E_PASSWORD="$PASSWORD" \
  npx playwright test "$@"
