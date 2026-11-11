#!/bin/sh
# Build a SynthPanel .deb from the release binaries.
#
#   VERSION=0.2.0 packaging/build-deb.sh
#
# Expects `cargo build --release` to have produced target/release/synthpanel-*.
set -eu

HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ROOT=$(CDPATH= cd -- "$HERE/.." && pwd)

VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)}"
VERSION="${VERSION:-0.1.0}"
ARCH="${ARCH:-$(dpkg --print-architecture 2>/dev/null || echo amd64)}"
BIN_DIR="${BIN_DIR:-$ROOT/target/release}"
OUT_DIR="${OUT_DIR:-$ROOT/target/debian}"

NAME="synthpanel"
BUILD=$(mktemp -d "${TMPDIR:-/tmp}/${NAME}-deb.XXXXXX")
trap 'rm -rf "$BUILD"' EXIT INT TERM

PKG="$BUILD/pkg"
mkdir -p "$PKG/DEBIAN" \
         "$PKG/usr/local/bin" \
         "$PKG/lib/systemd/system" \
         "$PKG/usr/share/doc/$NAME"

for binary in synthpanel-core synthpanel-web synthpanel synthpanel-installer; do
    if [ ! -f "$BIN_DIR/$binary" ]; then
        echo "missing $BIN_DIR/$binary — run 'cargo build --release' first" >&2
        exit 1
    fi
done

install -m 0755 "$BIN_DIR/synthpanel-core" "$PKG/usr/local/bin/synthpanel-core"
install -m 0755 "$BIN_DIR/synthpanel-web" "$PKG/usr/local/bin/synthpanel-web"
install -m 0755 "$BIN_DIR/synthpanel" "$PKG/usr/local/bin/synthpanel"
install -m 0755 "$BIN_DIR/synthpanel-installer" "$PKG/usr/local/bin/synthpanel-installer"

cp "$HERE/systemd/synthpanel-core.service" "$PKG/lib/systemd/system/synthpanel-core.service"
cp "$HERE/systemd/synthpanel-web.service" "$PKG/lib/systemd/system/synthpanel-web.service"
cp "$ROOT/LICENSE.md" "$PKG/usr/share/doc/$NAME/copyright"
cp "$ROOT/README.md" "$PKG/usr/share/doc/$NAME/README.md"

sed -e "s/^Version:.*/Version: $VERSION/" \
    -e "s/^Architecture:.*/Architecture: $ARCH/" \
    "$HERE/debian/control" > "$PKG/DEBIAN/control"

cat > "$PKG/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -eu
if ! getent group synthpanel >/dev/null 2>&1; then
    groupadd --system synthpanel
fi
if ! id synthpanel >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin --gid synthpanel synthpanel
fi
mkdir -p /etc/synthpanel /var/www /var/lib/synthpanel /var/log/synthpanel \
         /var/backups/synthpanel /run/synthpanel
if [ ! -f /etc/synthpanel/daemon.env ]; then
    install -m 0640 /dev/null /etc/synthpanel/daemon.env
fi
chown -R synthpanel: /var/lib/synthpanel /var/log/synthpanel /var/backups/synthpanel
# The web process writes the panel certificate into the TLS directory, so it
# has to belong to the unprivileged web user.
install -d -o synthpanel -g synthpanel -m 0750 /etc/synthpanel/tls
systemctl daemon-reload || true
systemctl enable synthpanel-core.service synthpanel-web.service 2>/dev/null || true
if [ -n "${SYNTHPANEL_RESTART:-}" ]; then
    systemctl restart synthpanel-core.service synthpanel-web.service || true
fi
POSTINST
chmod 0755 "$PKG/DEBIAN/postinst"

cat > "$PKG/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
set -eu
systemctl disable synthpanel-web.service 2>/dev/null || true
systemctl stop synthpanel-web.service 2>/dev/null || true
systemctl disable synthpanel-core.service 2>/dev/null || true
systemctl stop synthpanel-core.service 2>/dev/null || true
PRERM
chmod 0755 "$PKG/DEBIAN/prerm"

cat > "$PKG/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -eu
if [ "${1:-}" = "purge" ]; then
    rm -rf /var/lib/synthpanel /var/log/synthpanel /var/backups/synthpanel
fi
systemctl daemon-reload || true
POSTRM
chmod 0755 "$PKG/DEBIAN/postrm"

mkdir -p "$OUT_DIR"
STAGE="$OUT_DIR/${NAME}_${VERSION}_${ARCH}.deb"
dpkg-deb --build --root-owner-group "$PKG" "$STAGE"
echo "built: $STAGE"
