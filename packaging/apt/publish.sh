#!/bin/sh
# Publish .deb files into a flat apt repository.
#
#   packaging/apt/publish.sh target/debian/*.deb
#
# Produces:
#   $REPO_DIR/dists/stable/main/binary-<arch>/Packages(.gz)
#   $REPO_DIR/dists/stable/Release
#   $REPO_DIR/pool/main/*.deb
#
# Set GPG_KEY to also produce Release.gpg and InRelease.
set -eu

if [ "$#" -lt 1 ]; then
    echo "usage: $0 <package.deb>..." >&2
    exit 1
fi

REPO_DIR="${REPO_DIR:-$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)/target/apt-repo}"
ARCH="${ARCH:-$(dpkg --print-architecture 2>/dev/null || echo amd64)}"
SUITE="${SUITE:-stable}"
COMPONENT="main"

POOL="$REPO_DIR/pool/$COMPONENT"
DIST="$REPO_DIR/dists/$SUITE/$COMPONENT/binary-$ARCH"
mkdir -p "$POOL" "$DIST"

for package in "$@"; do
    cp "$package" "$POOL/"
done

( cd "$REPO_DIR" && apt-ftparchive packages "pool/$COMPONENT" > "dists/$SUITE/$COMPONENT/binary-$ARCH/Packages" )
gzip -kf "$DIST/Packages"

( cd "$REPO_DIR/dists/$SUITE" && apt-ftparchive \
    -o APT::FTPArchive::Release::Origin=SynthPanel \
    -o APT::FTPArchive::Release::Label=SynthPanel \
    -o APT::FTPArchive::Release::Suite="$SUITE" \
    -o APT::FTPArchive::Release::Codename="$SUITE" \
    -o APT::FTPArchive::Release::Architectures="$ARCH" \
    -o APT::FTPArchive::Release::Components="$COMPONENT" \
    release . > Release )

if [ -n "${GPG_KEY:-}" ]; then
    ( cd "$REPO_DIR/dists/$SUITE" && \
        gpg --batch --yes --armor --detach-sign --local-user "$GPG_KEY" -o Release.gpg Release && \
        gpg --batch --yes --clearsign --local-user "$GPG_KEY" -o InRelease Release )
fi

echo "repository: $REPO_DIR"
echo "client: deb [signed-by=/usr/share/keyrings/synthpanel.gpg] https://apt.synthpanel.io $SUITE $COMPONENT"
