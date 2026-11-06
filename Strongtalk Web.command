#!/bin/bash
# Double-click on a Mac: builds the WebAssembly website (the first time), serves it on this
# computer only and opens it in your browser. Close this window to stop it.
cd "$(dirname "$0")"
if [ ! -f dist/index.html ]; then
  npm run build || { echo "Build failed"; read -r; exit 1; }
fi
PORT=8642
while lsof -i :"$PORT" >/dev/null 2>&1; do PORT=$((PORT + 1)); done
echo "Strongtalk is at http://127.0.0.1:$PORT/  (close this window to stop)"
(sleep 1; open "http://127.0.0.1:$PORT/") &
cd dist && exec python3 -m http.server "$PORT" --bind 127.0.0.1
