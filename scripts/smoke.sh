#!/usr/bin/env bash
# Prueft ein fertiges Programm von aussen, ohne etwas ausser einem eigenen Testordner anzufassen.
# Aufruf: scripts/smoke.sh PFAD_ZUM_PROGRAMM ERWARTETE_VERSION
# Mit RUNNER=qemu-aarch64-static laeuft das Programm in einem Emulator.
set -euo pipefail

bin=$1
version=$2
run() { ${RUNNER:-} "$bin" "$@"; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fixture="$work/fixture"
data="$work/data"

echo "== version"
out=$(run --version)
echo "$out"
[ "$out" = "storage-map $version" ]

echo "== help"
run --help | grep -q -- '--mcp'

echo "== unknown option is refused"
if run --gibt-es-nicht >/dev/null 2>&1; then
  echo "unknown option was accepted"
  exit 1
fi

echo "== scan of a test folder"
run --make-fixture "$fixture"
run --sandbox "$fixture" --data-dir "$data" --scan "$fixture"

echo "== scan outside the test folder is refused"
outside=/
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) outside='C:\' ;; esac
if run --sandbox "$fixture" --data-dir "$data" --scan "$outside" >/dev/null 2>&1; then
  echo "scan outside the sandbox was accepted"
  exit 1
fi

echo "== MCP over pipes"
reply=$(printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_files","arguments":{"query":"*.mp4"}}}' \
  | run --mcp --sandbox "$fixture" --data-dir "$data")
echo "$reply" | grep -q '"protocolVersion":"2025-06-18"'
echo "$reply" | grep -q 'urlaub-2019.mp4'

echo "== demo server answers"
port=$((47000 + RANDOM % 900))
run --demo --data-dir "$work/demo" --no-open --token t --port "$port" >/dev/null &
pid=$!
ok=0
for _ in $(seq 1 60); do
  if curl -fsS -H 'X-SM-Token: t' "http://127.0.0.1:$port/api/status" 2>/dev/null | grep -q '"mode":"demo"'; then
    ok=1
    break
  fi
  sleep 1
done
kill "$pid" 2>/dev/null || true
[ "$ok" = 1 ]

echo "smoke test passed"
