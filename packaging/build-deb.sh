#!/bin/sh
# Build a .deb for SynthPanel.
set -eu
cd "$(dirname "$0")"

VERSION="0.1.0"
NAME="synthpanel"
ARCH="amd64"
ROOT_DIR="$ROOT_DIR"
BUILD="${TMPDIR:-/tmp}/${NAME}_${VERSION}_${ARCH}"

rm -rf "$BUILD"
mkdir -p "$BUILD"
STAGE="$BUILD/${NAME}_${VERSION}_${ARCH}.deb"
PKG_DIR="$BUILD/package"
mkdir -p "$PKG_DIR/DEBIAN" \
         "$PKG_DIR/usr/local/bin" \
         "$PKG_DIR/etc/systemd/system" \
         "$PKG_DIR/etc/synthpanel"

# Copy the release binaries (assumes `cargo build --release` was run).
cp ../release/$NAME-core "$PKG_DIR/usr/local/bin/synthpanel-core"
cp ../release/$NAME-web   "$PKG_DIR/usr/local/bin/synthpanel-web"
cp ../release/$NAME       "$PKG_DIR/usr/local/bin/synthpanel"
chmod 0755 "$PKG_DIR"/usr/local/bin/*

# Systemd units
cp ../systemd/$NAME-core.service "$PKG_DIR/etc/systemd/system/$NAME-core.service"
cp ../systemd/$NAME-web.service  "$PKG_DIR/etc/systemd/system/$NAME-web.service"

# Control file
cp ../debian/control "$PKG_DIR/DEBIAN/control"

# postinst: create user, dirs and daemon.env
cat > "$PKG_DIR/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -eu
if ! getent group synthpanel >/dev/null 2>&1; then
  groupadd --system synthpanel
fi
if ! id synthpanel >/dev/null 2>&1; then
  useradd --system --no-create-home --shell /usr/sbin/nologin --gid synthpanel synthpanel
fi
mkdir -p /var/www /var/lib/synthpanel /var/log/synthpanel /var/backups/synthpanel /run/synthpanel
[ -f /etc/synthpanel/daemon.env ] || \
  install -m 0640 /dev/null /etc/synthpanel/daemon.env
chown -R synthpanel: /var/lib/synthpanel /var/log/synthpanel /var/backups/synthpanel
systemctl daemon-reload
systemctl enable $NAME-core.service $NAME-web.service 2>/dev/null || true
POSTINST
chmod 0755 "$PKG_DIR/DEBIAN/postinst"

# prerm: stop and disable services before removal
cat > "$PKG_DIR/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
systemctl disable $NAME-web.service 2>/dev/null || true
systemctl stop $NAME-web.service 2>/dev/null || true
systemctl disable $NAME-core.service 2>/dev/null || true
systemctl stop $NAME-core.service 2>/dev/null || true
PRERM
chmod 0755 "$PKG_DIR/DEBIAN/prerm"

dpkg-deb --build "$PKG_DIR" "$STAGE"
echo "built: $STAGE"
