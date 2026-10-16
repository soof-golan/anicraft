#!/usr/bin/env bash
# Build the Linux installers (a .deb and an AppImage) from an already assembled package directory.
#
#   build-linux-packages.sh APP VERSION STAGE_DIR OUT_DIR
#
# APP is the program name (anicraft, charcraft); STAGE_DIR holds APP, APP-cli and anything they load next to
# themselves (the ONNX Runtime library, licences). Needs dpkg-deb and curl. The metadata comes from
# packaging/linux/APP.desktop, APP.svg and APP-mime.xml beside this script.
set -euo pipefail
umask 022

APP="$1"
VERSION="$2"
STAGE="$(cd "$3" && pwd)"
mkdir -p "$4"
OUT="$(cd "$4" && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"

case "$(uname -m)" in
  x86_64)
    ARCH=x86_64
    DEB_ARCH=amd64
    APPIMAGETOOL_SHA256="46fdd785094c7f6e545b61afcfb0f3d98d8eab243f644b4b17698c01d06083d1"
    ;;
  aarch64 | arm64)
    ARCH=aarch64
    DEB_ARCH=arm64
    APPIMAGETOOL_SHA256="04f45ea45b5aa07bb2b071aed9dbf7a5185d3953b11b47358c1311f11ea94a96"
    ;;
  *) echo "unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/1.9.0/appimagetool-${ARCH}.AppImage"

# Debian orders 0.1.0~rc.1 before 0.1.0; a non-numeric version (a branch build) gets a 0~ prefix.
DEB_VERSION="${VERSION/-/\~}"
case "$DEB_VERSION" in
  [0-9]*) ;;
  *) DEB_VERSION="0~${DEB_VERSION}" ;;
esac

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Lay out the files an install puts under a prefix, into $1/usr.
lay_out() {
  local root="$1"
  mkdir -p "$root/usr/lib/$APP" "$root/usr/bin" "$root/usr/share/applications" \
    "$root/usr/share/icons/hicolor/scalable/apps" "$root/usr/share/mime/packages"
  cp -a "$STAGE/." "$root/usr/lib/$APP/"
  chmod -R go-w "$root/usr/lib/$APP"
  ln -s "../lib/$APP/$APP" "$root/usr/bin/$APP"
  if [ -e "$STAGE/$APP-cli" ]; then ln -s "../lib/$APP/$APP-cli" "$root/usr/bin/$APP-cli"; fi
  cp "$HERE/$APP.desktop" "$root/usr/share/applications/$APP.desktop"
  cp "$HERE/$APP.svg" "$root/usr/share/icons/hicolor/scalable/apps/$APP.svg"
  cp "$HERE/$APP-mime.xml" "$root/usr/share/mime/packages/$APP.xml"
}

# ---- .deb ----
DEB_ROOT="$WORK/deb"
lay_out "$DEB_ROOT"
mkdir -p "$DEB_ROOT/usr/share/doc/$APP" "$DEB_ROOT/DEBIAN"
cp "$STAGE/LICENSE-MIT" "$DEB_ROOT/usr/share/doc/$APP/copyright"
SIZE_KB="$(du -sk "$DEB_ROOT/usr" | cut -f1)"
cat > "$DEB_ROOT/DEBIAN/control" <<EOF
Package: $APP
Version: $DEB_VERSION
Section: graphics
Priority: optional
Architecture: $DEB_ARCH
Installed-Size: $SIZE_KB
Maintainer: Soof Golan <hi@soofgolan.com>
Homepage: https://github.com/soof-golan/$APP
Depends: libc6 (>= 2.31), libgtk-3-0 | libgtk-3-0t64, libasound2 | libasound2t64, libxkbcommon0, libfontconfig1, libgl1, libx11-6, libxcursor1, libxrandr2, libxi6, libwayland-client0
Description: $(sed -n 's/^Comment=//p' "$HERE/$APP.desktop")
 Unsigned v$VERSION build. Installs $APP and ${APP}-cli.
EOF
find "$DEB_ROOT" -type d -exec chmod 755 {} +
dpkg-deb --root-owner-group --build "$DEB_ROOT" "$OUT/${APP}_${DEB_VERSION}_${DEB_ARCH}.deb" > /dev/null
echo "built $OUT/${APP}_${DEB_VERSION}_${DEB_ARCH}.deb"

# ---- AppImage ----
APPDIR="$WORK/$APP.AppDir"
lay_out "$APPDIR"
cp "$HERE/$APP.desktop" "$APPDIR/$APP.desktop"
cp "$HERE/$APP.svg" "$APPDIR/$APP.svg"
ln -s "$APP.svg" "$APPDIR/.DirIcon"
cat > "$APPDIR/AppRun" <<EOF
#!/bin/sh
HERE="\$(dirname "\$(readlink -f "\$0")")"
exec "\$HERE/usr/lib/$APP/$APP" "\$@"
EOF
chmod 755 "$APPDIR/AppRun"

TOOL="${APPIMAGETOOL:-$WORK/appimagetool}"
if [ ! -x "$TOOL" ]; then
  curl -fsSL --retry 5 --retry-delay 5 -o "$TOOL" "$APPIMAGETOOL_URL"
  echo "$APPIMAGETOOL_SHA256  $TOOL" | sha256sum -c -
  chmod 755 "$TOOL"
fi
# --appimage-extract-and-run: runners have no FUSE. The output is named by us, not guessed from the desktop file.
ARCH="$ARCH" "$TOOL" --appimage-extract-and-run --no-appstream "$APPDIR" "$OUT/${APP}-${VERSION}-linux-${ARCH}.AppImage" > /dev/null
echo "built $OUT/${APP}-${VERSION}-linux-${ARCH}.AppImage"
