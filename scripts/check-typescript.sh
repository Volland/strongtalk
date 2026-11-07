#!/bin/bash
# Checks the Strongtalk → TypeScript translator against the VM: every line of
# vm/tests/typescript/cases.txt and every .dlt file there must print the same in both,
# and every translation must type-check with tsc. Needs Node 22+ (and npx for tsc).
set -uo pipefail
cd "$(dirname "$0")/.."
ST="${ST:-vm/target/release/strongtalk}"
DIR="$(mktemp -d)"
trap 'rm -rf "$DIR"' EXIT
pass=0; fail=0; i=0
check() { # name, expected, ts file
  local got
  got="$(node --no-warnings "$3" 2>&1)"
  if [ "$2" = "$got" ]; then pass=$((pass+1)); else
    fail=$((fail+1)); echo "DIFFERENT: $1"; echo "  vm: $2" | head -5; echo "  ts: $got" | head -5
  fi
}
while IFS= read -r line; do
  [ -z "$line" ] && continue
  i=$((i+1))
  "$ST" --to-ts -e "$line" -o "$DIR/case$i.ts" 2>/dev/null
  check "$line" "$("$ST" -e "$line" 2>&1)" "$DIR/case$i.ts"
done < vm/tests/typescript/cases.txt
for f in vm/tests/typescript/*.dlt; do
  n="$(basename "$f" .dlt)"
  "$ST" --to-ts "$f" -o "$DIR/$n.ts" 2>/dev/null
  check "$f" "$("$ST" "$f" 2>&1)" "$DIR/$n.ts"
done
"$ST" --to-ts vm/lib/demos.dlt -o "$DIR/demos.ts" 2>/dev/null
echo "same output: $pass, different: $fail"
if command -v npm >/dev/null 2>&1; then
  (cd "$DIR" && npm install --silent --no-save typescript@5.6.3 >/dev/null 2>&1)
  bad=0
  for f in "$DIR"/*.ts; do  # one at a time: each translation is a separate program
    "$DIR/node_modules/.bin/tsc" --noEmit --target es2022 --lib es2022,dom --skipLibCheck "$f" || bad=$((bad+1))
  done
  if [ "$bad" -eq 0 ]; then echo "tsc: all translations type-check"; else echo "tsc: $bad file(s) with type errors"; fail=$((fail+1)); fi
fi
[ "$fail" -eq 0 ]
