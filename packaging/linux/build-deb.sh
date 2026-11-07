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
TOP="$HERE/../.."
# Debian keeps license information in /usr/share/doc/<package>/copyright
mkdir -p "$PKG/usr/share/doc/strongtalk"
{
  echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
  echo "Upstream-Name: strongtalk"
  echo "Source: https://github.com/Volland/strongtalk"
  echo
  echo "Files: *"
  echo "Copyright: 1994-2006 Sun Microsystems Inc."
  echo "           2026 Volodymyr Pavlyshyn and the contributors to the Strongtalk Rust edition"
  echo "License: BSD-3-Clause"
  echo
  echo "License: BSD-3-Clause"
  sed -e 's/^$/./' -e 's/^/ /' "$TOP/LICENSE"
} > "$PKG/usr/share/doc/strongtalk/copyright"
install -m 644 "$TOP/NOTICE" "$TOP/LICENSE-0BSD" "$TOP/THIRD-PARTY-NOTICES.txt" "$PKG/usr/share/doc/strongtalk/"
chmod 644 "$PKG/usr/share/doc/strongtalk/copyright"
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
dpkg-deb -Zxz --build --root-owner-group "$PKG" "$OUT"
