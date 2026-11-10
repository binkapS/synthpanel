#!/bin/sh
# Copyright (c) 2026 SynthPanel contributors
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Bootstrap script: fetch the latest SynthPanel release binaries, then
# hand off to the Rust installer. Designed for `curl | sudo bash`.
#
#   curl -sSfL https://get.synthpanel.io | sudo bash -s -- \
#     --unattended --hostname panel.example.com \
#     --admin-email admin@example.com

set -eu

CHANNEL="${SYNTHPANEL_CHANNEL:-stable}"
API="https://releases.synthpanel.io/${CHANNEL}"

echo "==> Detecting platform"
if [ "$(uname -s)" != "Linux" ]; then
  echo "synthpanel: only Linux is supported" >&2
  exit 1
fi
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) ARCH="amd64" ;;
  aarch64) ARCH="arm64" ;;
  *) echo "synthpanel: unsupported architecture $ARCH" >&2; exit 1 ;;
esac

echo "==> Downloading release index from ${API}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM
curl -fsSL -o "$TMP/manifest.json" "${API}/manifest.json"
VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$TMP/manifest.json")"

echo "==> Downloading SynthPanel ${VERSION} for ${ARCH}"
TARBALL="${API}/synthpanel-${VERSION}-linux-${ARCH}.tar.gz"
curl -fsSL -o "$TMP/synthpanel.tar.gz" "$TARBALL"

echo "==> Extracting binaries"
mkdir -p /opt/synthpanel
tar -xzf "$TMP/synthpanel.tar.gz" -C /opt/synthpanel

if [ ! -x /opt/synthpanel/synthpanel-installer ]; then
  echo "synthpanel-installer is missing from the release archive" >&2
  exit 1
fi

echo "==> Running the installer"
exec /opt/synthpanel/synthpanel-installer "$@"
