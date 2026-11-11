#!/usr/bin/env bash
# Chaos test for the config apply pipeline.
#
# The panel promises that a broken configuration never reaches the server:
# render → lint → backup → write → validate → reload, with a rollback when
# the validation fails. This test forces the validation to fail by putting a
# fake `nginx` on the PATH and checks that:
#
#   1. an existing site keeps its previous vhost when a change fails,
#   2. a new site with a broken config leaves no vhost behind,
#   3. the panel reports the rollback to the caller,
#   4. the panel recovers once the configuration is valid again.
#
# Run: tests/chaos.sh
set -eu

cd "$(dirname "$0")/.."

WORK="$(mktemp -d /tmp/sp-chaos.XXXXXX)"
BIN="$WORK/bin"
mkdir -p "$BIN"

log() { printf '==> %s\n' "$*"; }
fail() {
  printf 'FAIL: %s\n' "$*" >&2
  echo "--- core log ---"
  tail -40 "$WORK/core.log" 2>/dev/null || true
  exit 1
}

assert_contains() {
  case "$1" in
    *"$2"*) return 0 ;;
    *) fail "missing '$2' in: $1" ;;
  esac
}

# A fake nginx: `nginx -t` succeeds while CHAOS_NGINX_OK is set, otherwise it
# reports a broken configuration.
write_fake_nginx() {
  cat > "$BIN/nginx" <<'FAKE'
#!/bin/sh
if [ -n "${CHAOS_NGINX_OK:-}" ]; then
  echo "nginx: configuration file /etc/nginx/nginx.conf test is successful"
  exit 0
fi
echo "nginx: [emerg] invalid directive \"chaos\" in /etc/nginx/conf.d/broken.conf" >&2
exit 1
FAKE
  chmod +x "$BIN/nginx"
}
write_fake_nginx

# A fake systemctl so the pipeline does not wait for a real systemd.
cat > "$BIN/systemctl" <<'FAKE'
#!/bin/sh
exit 0
FAKE
chmod +x "$BIN/systemctl"

log "building release binaries"
cargo build --release -p synthpanel-core -p synthpanel-cli >/dev/null

log "starting the core with the fake nginx on the PATH"
PATH="$BIN:$PATH" \
CHAOS_NGINX_OK=1 \
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
  target/release/synthpanel-core > "$WORK/core.log" 2>&1 &
CORE_PID=$!

cleanup() {
  kill "$CORE_PID" 2>/dev/null || true
  wait "$CORE_PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

for _ in $(seq 1 50); do
  [ -S "$WORK/run/core.sock" ] && break
  sleep 0.2
done
[ -S "$WORK/run/core.sock" ] || fail "the core did not bind its socket"

export SYNTHPANEL_CORE_SOCKET="$WORK/run/core.sock"
export SYNTHPANEL_IPC_TOKEN_FILE="$WORK/etc/ipc-token"

ipc() {
  python3 tests/ipc-call.py "$@"
}

site_id_of() {
  printf '%s' "$1" | python3 -c 'import json,sys; print(json.load(sys.stdin)["site"]["id"])'
}

log "creating a healthy site"
CREATED="$(ipc sites.create '{"domain":"chaos.example.com","template":"php"}')"
assert_contains "$CREATED" '"rolled_back":false'
SITE_ID="$(site_id_of "$CREATED")"
VHOST="$WORK/nginx/sites-available/synthpanel/$SITE_ID.conf"
[ -f "$VHOST" ] || fail "the vhost was not written"
assert_contains "$(cat "$VHOST")" "server_name chaos.example.com;"
log "vhost written: $VHOST"

# ---------------------------------------------------------------------------
# 1. A broken change keeps the previous vhost
# ---------------------------------------------------------------------------

log "breaking the nginx validation"
cat > "$BIN/nginx" <<'FAKE'
#!/bin/sh
echo "nginx: [emerg] invalid directive \"chaos\" in /etc/nginx/conf.d/broken.conf" >&2
exit 1
FAKE
chmod +x "$BIN/nginx"

log "updating the site while the validation fails"
UPDATED="$(ipc sites.update "{\"id\":\"$SITE_ID\",\"template\":\"laravel\"}")"
assert_contains "$UPDATED" '"rolled_back":true' || fail "the update was not rolled back: $UPDATED"
assert_contains "$(cat "$VHOST")" "server_name chaos.example.com;"
if grep -q "laravel" "$VHOST"; then
  fail "the broken configuration reached the server"
fi
log "the previous vhost survived the failed change"

# ---------------------------------------------------------------------------
# 2. A new site with a broken configuration leaves nothing behind
# ---------------------------------------------------------------------------

log "creating a site while the validation fails"
BROKEN="$(ipc sites.create '{"domain":"broken.example.com","template":"php"}')"
assert_contains "$BROKEN" '"rolled_back":true' || fail "the broken site was not rolled back: $BROKEN"
BROKEN_ID="$(site_id_of "$BROKEN")"
BROKEN_VHOST="$WORK/nginx/sites-available/synthpanel/$BROKEN_ID.conf"
if [ -f "$BROKEN_VHOST" ]; then
  fail "the broken vhost was left on the server: $BROKEN_VHOST"
fi
log "the broken vhost was removed again"

# ---------------------------------------------------------------------------
# 3. The panel recovers once the configuration is valid again
# ---------------------------------------------------------------------------

log "restoring a valid nginx"
write_fake_nginx
RECOVERED="$(ipc sites.create '{"domain":"recovered.example.com","template":"php"}')"
assert_contains "$RECOVERED" '"rolled_back":false'
RECOVERED_ID="$(site_id_of "$RECOVERED")"
RECOVERED_VHOST="$WORK/nginx/sites-available/synthpanel/$RECOVERED_ID.conf"
[ -f "$RECOVERED_VHOST" ] || fail "the panel did not recover"
assert_contains "$(cat "$RECOVERED_VHOST")" "server_name recovered.example.com;"
log "the panel recovered"

log "chaos test passed"
