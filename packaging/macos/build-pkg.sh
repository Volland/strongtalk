#!/bin/bash
# Builds the macOS installer: installs `strongtalk` into /usr/local/bin.
#   packaging/macos/build-pkg.sh <universal binary> <version> <out.pkg>
set -euo pipefail
BIN="$1"; VERSION="$2"; OUT="$3"
HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/root/usr/local/bin" "$WORK/resources"
install -m 755 "$BIN" "$WORK/root/usr/local/bin/strongtalk"
export COPYFILE_DISABLE=1
xattr -cr "$WORK/root" 2>/dev/null || true
pkgbuild --root "$WORK/root" --identifier io.github.volland.strongtalk --version "$VERSION" \
  --install-location / "$WORK/strongtalk.pkg"
cp "$HERE/welcome.html" "$HERE/conclusion.html" "$WORK/resources/"
sed "s/@VERSION@/$VERSION/g" "$HERE/distribution.xml" > "$WORK/distribution.xml"
productbuild --distribution "$WORK/distribution.xml" --resources "$WORK/resources" \
  --package-path "$WORK" "$OUT"
echo "built $OUT"
