#!/usr/bin/env bash
# End-to-end test for the self-update path: signed envelope, download,
# checksum, install, health check, history and signature rejection.
#
# Run: tests/update.sh
set -eu

cd "$(dirname "$0")/.."

WORK=$(mktemp -d "${TMPDIR:-/tmp}/sp-update-e2e.XXXXXX")
trap 'rm -rf "$WORK"' EXIT INT TERM

log() { printf '==> %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; echo "--- core log ---"; tail -40 "$WORK/core.log" 2>/dev/null; exit 1; }

assert_contains() {
  case "$1" in
    *"$2"*) return 0 ;;
    *) fail "missing '$2' in: $1" ;;
  esac
}

log "building release binaries"
cargo build --release -p synthpanel-core -p synthpanel-web -p synthpanel-cli >/dev/null

log "generating a release key"
openssl genpkey -algorithm ed25519 -out "$WORK/release.pem" 2>/dev/null
PUBKEY=$(openssl pkey -in "$WORK/release.pem" -pubout -outform DER | tail -c 32 | xxd -p -c 64)

log "building a fake 9.9.9 release archive"
mkdir -p "$WORK/release"
for binary in synthpanel-core synthpanel-web synthpanel; do
  cp "target/release/$binary" "$WORK/release/$binary"
done
tar -C "$WORK/release" -czf "$WORK/release.tar.gz" synthpanel-core synthpanel-web synthpanel

log "signing the release envelope"
OUT="$WORK/update.json" packaging/sign-release.sh \
  "$WORK/release.tar.gz" 9.9.9 "$WORK/release.tar.gz" \
  --key "$WORK/release.pem" >/dev/null
if [ ! -f "$WORK/update.json" ]; then
  fail "sign-release.sh did not write an envelope"
fi

mkdir -p "$WORK/bin"
for binary in synthpanel-core synthpanel-web synthpanel; do
  cp "target/release/$binary" "$WORK/bin/$binary"
done

log "starting the core"
SYNTHPANEL_ETC_DIR="$WORK/etc" \
SYNTHPANEL_LIB_DIR="$WORK/lib" \
SYNTHPANEL_LOG_DIR="$WORK/log" \
SYNTHPANEL_RUN_DIR="$WORK/run" \
SYNTHPANEL_WWW_DIR="$WORK/www" \
SYNTHPANEL_BACKUP_DIR="$WORK/backups" \
SYNTHPANEL_NGINX_DIR="$WORK/nginx" \
SYNTHPANEL_PHP_DIR="$WORK/php" \
SYNTHPANEL_CRON_DIR="$WORK/cron" \
SYNTHPANEL_UPDATE_URL="$WORK/update.json" \
SYNTHPANEL_UPDATE_PUBKEY="$PUBKEY" \
SYNTHPANEL_INSTALL_DIR="$WORK/bin" \
SYNTHPANEL_SKIP_SYSTEMD=1 \
RUST_LOG=warn \
  target/release/synthpanel-core > "$WORK/core.log" 2>&1 &
CORE_PID=$!

cleanup() { kill "$CORE_PID" 2>/dev/null || true; wait "$CORE_PID" 2>/dev/null || true; }
trap 'cleanup; rm -rf "$WORK"' EXIT INT TERM

for _ in $(seq 1 50); do
  [ -S "$WORK/run/core.sock" ] && break
  sleep 0.2
done
[ -S "$WORK/run/core.sock" ] || fail "core did not bind the socket"

cli() {
  SYNTHPANEL_CORE_SOCKET="$WORK/run/core.sock" \
  SYNTHPANEL_IPC_TOKEN_FILE="$WORK/etc/ipc-token" \
    target/release/synthpanel "$@"
}

log "checking for updates"
CHECK=$(cli update --check)
assert_contains "$CHECK" '"update_available":true'
assert_contains "$CHECK" '"version":"9.9.9"'

log "applying the update"
APPLIED=$(cli update)
assert_contains "$APPLIED" '"updated":true'
assert_contains "$APPLIED" '"to_version":"9.9.9"'
[ -f "$WORK/bin/synthpanel-core" ] || fail "the installed binary is missing"
[ -d "$WORK/lib/updates/rollback/rollback-0.1.0" ] || fail "the rollback copy is missing"

STATUS=$(cli update:status)
assert_contains "$STATUS" '"to_version":"9.9.9"'
assert_contains "$STATUS" '"status":"ok"'
log "update applied and recorded"

log "rejecting a tampered envelope"
python3 - "$WORK/update.json" "$WORK/tampered.json" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    envelope = json.load(handle)
envelope["payload"] = envelope["payload"].replace("9.9.9", "8.8.8")
with open(sys.argv[2], "w", encoding="utf-8") as handle:
    json.dump(envelope, handle)
PY
cp "$WORK/tampered.json" "$WORK/update.json"
if cli update --check > "$WORK/tampered.out" 2>&1; then
  fail "a tampered envelope was accepted"
fi
assert_contains "$(cat "$WORK/tampered.out")" "signature is invalid"
log "tampered envelope rejected"

log "update test passed"
