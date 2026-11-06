#!/bin/bash
# Double-click on a Mac to publish local changes to https://github.com/Volland/strongtalk.
# Rebuilds the WebAssembly VM, commits everything and pushes to main; GitHub Actions then
# redeploys the website at https://volland.github.io/strongtalk/ within a minute or two.
set -e
cd "$(dirname "$0")"
npm run build
git add -A
git commit -m "Update Strongtalk" || echo "Nothing new to commit."
git push origin main
echo
echo "Pushed. The website redeploys at https://volland.github.io/strongtalk/"
read -r -p "Press Return to close."
