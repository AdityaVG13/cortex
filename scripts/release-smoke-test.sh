#!/bin/bash
set -euo pipefail

echo "=== Cortex v0.4.0 Release Smoke Test ==="

PYTHON_CMD=()
if command -v python3 >/dev/null 2>&1 && python3 -V >/dev/null 2>&1; then
  PYTHON_CMD=(python3)
elif command -v python >/dev/null 2>&1 && python -V >/dev/null 2>&1; then
  PYTHON_CMD=(python)
elif command -v py >/dev/null 2>&1; then
  PYTHON_CMD=(py -3)
else
  echo "FAIL: python3/python not found"
  exit 1
fi

# 1. Binary exists and runs
cortex --help || { echo "FAIL: cortex binary not found"; exit 1; }

# 2. Paths command works (REQUIRES 1B)
cortex paths --json | "${PYTHON_CMD[@]}" -c "import sys,json; d=json.load(sys.stdin); assert 'home' in d and 'db' in d and 'port' in d" \
  || { echo "FAIL: cortex paths --json invalid"; exit 1; }
echo "PASS: paths --json"

# 3. Ensure daemon starts (REQUIRES 1B)
cortex plugin ensure-daemon --agent smoke-test || { echo "FAIL: ensure-daemon"; exit 1; }
echo "PASS: ensure-daemon"

# 4. Health check
curl -sf http://127.0.0.1:7437/health | "${PYTHON_CMD[@]}" -c "import sys,json; d=json.load(sys.stdin); assert d['status']=='ok'" \
  || { echo "FAIL: health check"; exit 1; }
echo "PASS: health check"

# 5. Store + recall round-trip via MCP (REQUIRES 1B + 1C)
echo '{"jsonrpc":"2.0","method":"tools/call","params":{"name":"cortex_store","arguments":{"decision":"smoke test memory","context":"release verification"}},"id":1}' \
  | cortex plugin mcp --url http://127.0.0.1:7437 \
  | "${PYTHON_CMD[@]}" -c "import sys,json; d=json.load(sys.stdin); assert 'result' in d" \
  || { echo "FAIL: store via MCP"; exit 1; }
echo "PASS: store"

# 6. Recall what we just stored
echo '{"jsonrpc":"2.0","method":"tools/call","params":{"name":"cortex_recall","arguments":{"query":"smoke test","budget":100}},"id":2}' \
  | cortex plugin mcp --url http://127.0.0.1:7437 \
  | "${PYTHON_CMD[@]}" -c "import sys,json; d=json.load(sys.stdin); assert 'result' in d" \
  || { echo "FAIL: recall via MCP"; exit 1; }
echo "PASS: recall"

# 7. Custom port works (REQUIRES 1D + 1E)
CORTEX_PORT=9999 cortex serve &
DAEMON_PID=$!
sleep 2
curl -sf http://127.0.0.1:9999/health > /dev/null && echo "PASS: custom port" || echo "FAIL: custom port"
kill $DAEMON_PID 2>/dev/null

# 8. Legacy migration works (~/cortex -> ~/.cortex)
MIGRATION_ROOT="$(mktemp -d)"
MIGRATION_HOME="${MIGRATION_ROOT}/home"
mkdir -p "${MIGRATION_HOME}/cortex"
export MIGRATION_HOME
"${PYTHON_CMD[@]}" - <<'PY'
import os, sqlite3
home = os.environ["MIGRATION_HOME"]
path = os.path.join(home, "cortex", "cortex.db")
os.makedirs(os.path.dirname(path), exist_ok=True)
conn = sqlite3.connect(path)
conn.execute("CREATE TABLE IF NOT EXISTS migration_probe (id INTEGER PRIMARY KEY, note TEXT)")
conn.execute("INSERT INTO migration_probe(note) VALUES ('legacy')")
conn.commit()
conn.close()
PY
MIGRATION_PORT=9997
HOME="${MIGRATION_HOME}" USERPROFILE="${MIGRATION_HOME}" CORTEX_HOME="${MIGRATION_HOME}/.cortex" CORTEX_PORT="${MIGRATION_PORT}" cortex plugin ensure-daemon --agent smoke-migration \
  || { echo "FAIL: legacy migration ensure-daemon"; exit 1; }
test -f "${MIGRATION_HOME}/.cortex/cortex.db" || { echo "FAIL: migrated db missing at ~/.cortex/cortex.db"; exit 1; }
echo "PASS: legacy migration"
if [ -f "${MIGRATION_HOME}/.cortex/cortex.pid" ]; then
  MIGRATION_PID="$(cat "${MIGRATION_HOME}/.cortex/cortex.pid" 2>/dev/null || true)"
  if [ -n "${MIGRATION_PID}" ]; then
    kill "${MIGRATION_PID}" 2>/dev/null || true
  fi
fi
rm -rf "${MIGRATION_ROOT}"

# 9. Concurrent ensure-daemon startup is safe
CONCURRENCY_ROOT="$(mktemp -d)"
CONCURRENCY_HOME="${CONCURRENCY_ROOT}/home"
mkdir -p "${CONCURRENCY_HOME}"
CONCURRENCY_PORT=9996
HOME="${CONCURRENCY_HOME}" USERPROFILE="${CONCURRENCY_HOME}" CORTEX_HOME="${CONCURRENCY_HOME}/.cortex" CORTEX_PORT="${CONCURRENCY_PORT}" cortex plugin ensure-daemon --agent smoke-concurrency-A > /tmp/cortex-concurrency-a.log 2>&1 &
PID_A=$!
HOME="${CONCURRENCY_HOME}" USERPROFILE="${CONCURRENCY_HOME}" CORTEX_HOME="${CONCURRENCY_HOME}/.cortex" CORTEX_PORT="${CONCURRENCY_PORT}" cortex plugin ensure-daemon --agent smoke-concurrency-B > /tmp/cortex-concurrency-b.log 2>&1 &
PID_B=$!
wait "$PID_A" || { echo "FAIL: concurrent ensure-daemon A"; cat /tmp/cortex-concurrency-a.log; exit 1; }
wait "$PID_B" || { echo "FAIL: concurrent ensure-daemon B"; cat /tmp/cortex-concurrency-b.log; exit 1; }
curl -sf "http://127.0.0.1:${CONCURRENCY_PORT}/health" > /dev/null || { echo "FAIL: concurrent daemon health"; exit 1; }
echo "PASS: concurrent startup lock"
if [ -f "${CONCURRENCY_HOME}/.cortex/cortex.pid" ]; then
  CONCURRENCY_DAEMON_PID="$(cat "${CONCURRENCY_HOME}/.cortex/cortex.pid" 2>/dev/null || true)"
  if [ -n "${CONCURRENCY_DAEMON_PID}" ]; then
    kill "${CONCURRENCY_DAEMON_PID}" 2>/dev/null || true
  fi
fi
rm -f /tmp/cortex-concurrency-a.log /tmp/cortex-concurrency-b.log
rm -rf "${CONCURRENCY_ROOT}"

echo "=== All smoke tests passed ==="
