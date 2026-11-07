#!/bin/bash
# Builds a Debian package.
#   packaging/linux/build-deb.sh <binary> <version> <amd64|arm64> <out.deb>
set -euo pipefail
BIN="$1"; VERSION="$2"; ARCH="$3"; OUT="$4"
HERE="$(cd "$(dirname "$0")" && pwd)"
PKG="$(mktemp -d)"
trap 'rm -rf "$PKG"' EXIT
install -Dm755 "$BIN" "$PKG/usr/bin/strongtalk"
install -Dm644 "$HERE/strongtalk.desktop" "$PKG/usr/share/applications/strongtalk.desktop"
install -Dm644 "$HERE/../icon.svg" "$PKG/usr/share/icons/hicolor/scalable/apps/strongtalk.svg"
mkdir -p "$PKG/DEBIAN"
cat > "$PKG/DEBIAN/control" <<CONTROL
Package: strongtalk
Version: $VERSION
Architecture: $ARCH
Maintainer: Strongtalk Rust edition <https://github.com/Volland/strongtalk>
Section: devel
Priority: optional
Homepage: https://volland.github.io/strongtalk/
Description: Strongtalk, the typed Smalltalk, re-implemented in Rust
 A terminal workspace and type checker, a Strongtalk to TypeScript translator
 (strongtalk --to-ts), and the full graphical environment, which
 "strongtalk web" opens in a browser, offline.
CONTROL
dpkg-deb --build --root-owner-group "$PKG" "$OUT"
