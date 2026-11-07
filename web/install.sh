#!/bin/sh
# Installs the latest Strongtalk for macOS or Linux:
#   curl -fsSL https://volland.github.io/strongtalk/install.sh | sh
# Puts `strongtalk` in $STRONGTALK_INSTALL_DIR (default: ~/.local/bin).
set -eu

REPO="Volland/strongtalk"
DIR="${STRONGTALK_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
  Darwin) ASSET="strongtalk-macos-universal.tar.gz" ;;
  Linux)
    case "$(uname -m)" in
      x86_64|amd64) ASSET="strongtalk-linux-x64.tar.gz" ;;
      aarch64|arm64) ASSET="strongtalk-linux-arm64.tar.gz" ;;
      *) echo "No Strongtalk build for $(uname -m) yet; see https://github.com/$REPO#building" >&2; exit 1 ;;
    esac ;;
  *) echo "On Windows, use: irm https://volland.github.io/strongtalk/install.ps1 | iex" >&2; exit 1 ;;
esac

URL="https://github.com/$REPO/releases/latest/download/$ASSET"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
echo "Downloading $URL"
if command -v curl >/dev/null 2>&1; then curl -fsSL "$URL" -o "$TMP/st.tar.gz"
else wget -qO "$TMP/st.tar.gz" "$URL"; fi
tar -xzf "$TMP/st.tar.gz" -C "$TMP"
mkdir -p "$DIR"
install -m 755 "$TMP/strongtalk" "$DIR/strongtalk"
echo "Installed $("$DIR/strongtalk" --version) to $DIR/strongtalk"
case ":$PATH:" in
  *":$DIR:"*) ;;
  *) echo "Add it to your PATH:  export PATH=\"$DIR:\$PATH\"" ;;
esac
echo "Try:  strongtalk          (terminal workspace)"
echo "      strongtalk web      (the environment, in your browser)"
