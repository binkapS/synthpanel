#!/bin/sh
# Sign a release archive and emit the update envelope.
#
#   packaging/sign-release.sh release.tar.gz 0.2.0 https://…/release.tar.gz \
#       --key release-ed25519.pem [--channel stable] [--notes "…"]
#
# The private key is an Ed25519 PEM (`openssl genpkey -algorithm ed25519`).
# The printed public key is the hex value to set as SYNTHPANEL_UPDATE_PUBKEY.
set -eu

if [ "$#" -lt 3 ]; then
    echo "usage: $0 <archive> <version> <url> --key <private.pem> [--channel stable]" >&2
    exit 1
fi

ARCHIVE=$1
VERSION=$2
URL=$3
shift 3

KEY=""
CHANNEL="stable"
NOTES=""
PUBLISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)

while [ "$#" -gt 0 ]; do
    case "$1" in
        --key) KEY=$2; shift 2 ;;
        --channel) CHANNEL=$2; shift 2 ;;
        --notes) NOTES=$2; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 1 ;;
    esac
done

[ -n "$KEY" ] || { echo "--key is required" >&2; exit 1; }

WORK=$(mktemp -d "${TMPDIR:-/tmp}/sp-sign.XXXXXX")
trap 'rm -rf "$WORK"' EXIT INT TERM

SHA256=$(sha256sum "$ARCHIVE" | cut -d' ' -f1)

ARCHIVE="$ARCHIVE" VERSION="$VERSION" URL="$URL" SHA256="$SHA256" \
CHANNEL="$CHANNEL" NOTES="$NOTES" PUBLISHED_AT="$PUBLISHED_AT" \
python3 - "$WORK/payload.json" <<'PY'
import json, os, sys
payload = {
    "version": os.environ["VERSION"],
    "channel": os.environ["CHANNEL"],
    "url": os.environ["URL"],
    "sha256": os.environ["SHA256"],
    "notes": os.environ["NOTES"],
    "published_at": os.environ["PUBLISHED_AT"],
}
with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump(payload, handle, separators=(",", ":"))
PY

openssl pkeyutl -sign -inkey "$KEY" -rawin -in "$WORK/payload.json" -out "$WORK/signature.bin"

python3 - "$WORK/payload.json" "$WORK/signature.bin" "$WORK/envelope.json" <<'PY'
import base64, json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    payload = handle.read()
with open(sys.argv[2], "rb") as handle:
    signature = base64.b64encode(handle.read()).decode("ascii")
with open(sys.argv[3], "w", encoding="utf-8") as handle:
    json.dump({"payload": payload, "signature": signature}, handle, indent=2)
PY

PUBKEY=$(openssl pkey -in "$KEY" -pubout -outform DER | tail -c 32 | xxd -p -c 64)

echo "envelope: $WORK/envelope.json"
cat "$WORK/envelope.json"
echo
echo "public key (hex): $PUBKEY"
cp "$WORK/envelope.json" "${OUT:-./update.json}"
echo "written: ${OUT:-./update.json}"
