#!/bin/bash
# Double-click on a Mac: builds (the first time) and starts Strongtalk in this Terminal window.
cd "$(dirname "$0")"
BIN=vm/target/release/strongtalk
if [ ! -x "$BIN" ] || [ -n "$(find vm/src vm/lib -newer "$BIN" -print -quit)" ]; then
  echo "Building Strongtalk (Rust)..."
  cargo build --release --manifest-path vm/Cargo.toml || { echo "Build failed - is Rust installed? See https://rustup.rs"; read -r; exit 1; }
fi
exec "$BIN" "$@"
