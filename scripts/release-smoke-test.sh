#!/bin/bash
set -euo pipefail

echo "=== Cortex v0.4.0 Release Smoke Test ==="

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

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

CORTEX_BIN="${CORTEX_BIN:-}"
if [[ -z "${CORTEX_BIN}" ]]; then
  if command -v cortex >/dev/null 2>&1; then
    CORTEX_BIN="cortex"
  elif [[ -x "${REPO_ROOT}/daemon-rs/target/release/cortex" ]]; then
    CORTEX_BIN="${REPO_ROOT}/daemon-rs/target/release/cortex"
  elif [[ -f "${REPO_ROOT}/daemon-rs/target/release/cortex.exe" ]]; then
    CORTEX_BIN="${REPO_ROOT}/daemon-rs/target/release/cortex.exe"
  fi
fi

if [[ -z "${CORTEX_BIN}" ]]; then
  echo "FAIL: cortex binary not found (set CORTEX_BIN or build daemon-rs first)"
  exit 1
fi

SMOKE_ROOT="$(mktemp -d)"
SMOKE_HOME="${SMOKE_ROOT}/home"
SMOKE_CORTEX_HOME="${SMOKE_HOME}/.cortex"
if [[ -z "${SMOKE_PORT:-}" ]]; then
  SMOKE_PORT="$((17000 + RANDOM % 1000))"
fi
mkdir -p "${SMOKE_HOME}"

cleanup() {
  if [ -f "${SMOKE_CORTEX_HOME}/cortex.pid" ]; then
    DAEMON_PID="$(cat "${SMOKE_CORTEX_HOME}/cortex.pid" 2>/dev/null || true)"
    if [ -n "${DAEMON_PID}" ]; then
      kill "${DAEMON_PID}" 2>/dev/null || true
    fi
  fi
  rm -rf "${SMOKE_ROOT}" 2>/dev/null || true
}
trap cleanup EXIT

run_cortex() {
  HOME="${SMOKE_HOME}" USERPROFILE="${SMOKE_HOME}" "${CORTEX_BIN}" "$@" --home "${SMOKE_CORTEX_HOME}" --port "${SMOKE_PORT}"
}

wait_for_health() {
  local url="$1"
  "${PYTHON_CMD[@]}" - "$url" <<'PY'
import json
import sys
import time
import urllib.request

url = sys.argv[1]
for _ in range(40):
    try:
        with urllib.request.urlopen(url, timeout=2) as response:
            data = json.loads(response.read().decode("utf-8"))
            if data.get("status") == "ok":
                sys.exit(0)
    except Exception:
        pass
    time.sleep(0.5)
sys.exit(1)
PY
}

kill_port_process() {
  local port="$1"
  if command -v netstat >/dev/null 2>&1; then
    local pids
    pids="$(netstat -ano 2>/dev/null | grep "127.0.0.1:${port}" | grep LISTENING | awk '{print $5}' | sort -u || true)"
    for pid in ${pids}; do
      taskkill //PID "${pid}" //F >/dev/null 2>&1 || kill "${pid}" 2>/dev/null || true
    done
  fi
}

# 1. Binary exists and runs
"${CORTEX_BIN}" --help || { echo "FAIL: cortex binary not found"; exit 1; }

# 2. Paths command works (REQUIRES 1B)
run_cortex paths --json | "${PYTHON_CMD[@]}" -c "import sys,json; d=json.load(sys.stdin); assert 'home' in d and 'db' in d and 'port' in d" \
  || { echo "FAIL: cortex paths --json invalid"; exit 1; }
echo "PASS: paths --json"

# 3. Ensure daemon starts (REQUIRES 1B)
run_cortex plugin ensure-daemon --agent smoke-test || { echo "FAIL: ensure-daemon"; exit 1; }
echo "PASS: ensure-daemon"

# 4. Health check
wait_for_health "http://127.0.0.1:${SMOKE_PORT}/health" \
  || { echo "FAIL: health check"; exit 1; }
echo "PASS: health check"

# 5. Store + recall round-trip via MCP (REQUIRES 1B + 1C)
STORE_RESPONSE="$(
  echo '{"jsonrpc":"2.0","method":"tools/call","params":{"name":"cortex_store","arguments":{"decision":"smoke test memory","context":"release verification"}},"id":1}' \
    | run_cortex plugin mcp --url "http://127.0.0.1:${SMOKE_PORT}"
)"
echo "${STORE_RESPONSE}" \
  | "${PYTHON_CMD[@]}" -c "import sys,json; d=json.load(sys.stdin); assert 'result' in d" \
  || { echo "FAIL: store via MCP"; echo "Store response: ${STORE_RESPONSE}"; exit 1; }
echo "PASS: store"

# 6. Recall what we just stored
RECALL_RESPONSE="$(
  echo '{"jsonrpc":"2.0","method":"tools/call","params":{"name":"cortex_recall","arguments":{"query":"smoke test","budget":100}},"id":2}' \
    | run_cortex plugin mcp --url "http://127.0.0.1:${SMOKE_PORT}"
)"
echo "${RECALL_RESPONSE}" \
  | "${PYTHON_CMD[@]}" -c "import sys,json; d=json.load(sys.stdin); assert 'result' in d" \
  || { echo "FAIL: recall via MCP"; echo "Recall response: ${RECALL_RESPONSE}"; exit 1; }
echo "PASS: recall"

# 7. Custom port works (REQUIRES 1D + 1E)
CUSTOM_HOME="${SMOKE_ROOT}/custom-home"
mkdir -p "${CUSTOM_HOME}/.cortex"
"${CORTEX_BIN}" serve --home "${CUSTOM_HOME}/.cortex" --port 9999 &
DAEMON_PID=$!
wait_for_health "http://127.0.0.1:9999/health" && echo "PASS: custom port" || echo "FAIL: custom port"
kill $DAEMON_PID 2>/dev/null
kill_port_process 9999

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
HOME="${MIGRATION_HOME}" USERPROFILE="${MIGRATION_HOME}" "${CORTEX_BIN}" plugin ensure-daemon --agent smoke-migration --home "${MIGRATION_HOME}/.cortex" --port "${MIGRATION_PORT}" \
  || { echo "FAIL: legacy migration ensure-daemon"; exit 1; }
test -f "${MIGRATION_HOME}/.cortex/cortex.db" || { echo "FAIL: migrated db missing at ~/.cortex/cortex.db"; exit 1; }
echo "PASS: legacy migration"
kill_port_process "${MIGRATION_PORT}"
rm -rf "${MIGRATION_ROOT}" || true

# 9. Concurrent ensure-daemon startup is safe
CONCURRENCY_ROOT="$(mktemp -d)"
CONCURRENCY_HOME="${CONCURRENCY_ROOT}/home"
mkdir -p "${CONCURRENCY_HOME}"
CONCURRENCY_PORT=9996
HOME="${CONCURRENCY_HOME}" USERPROFILE="${CONCURRENCY_HOME}" "${CORTEX_BIN}" plugin ensure-daemon --agent smoke-concurrency-A --home "${CONCURRENCY_HOME}/.cortex" --port "${CONCURRENCY_PORT}" > /tmp/cortex-concurrency-a.log 2>&1 &
PID_A=$!
HOME="${CONCURRENCY_HOME}" USERPROFILE="${CONCURRENCY_HOME}" "${CORTEX_BIN}" plugin ensure-daemon --agent smoke-concurrency-B --home "${CONCURRENCY_HOME}/.cortex" --port "${CONCURRENCY_PORT}" > /tmp/cortex-concurrency-b.log 2>&1 &
PID_B=$!
wait "$PID_A" || { echo "FAIL: concurrent ensure-daemon A"; cat /tmp/cortex-concurrency-a.log; exit 1; }
wait "$PID_B" || { echo "FAIL: concurrent ensure-daemon B"; cat /tmp/cortex-concurrency-b.log; exit 1; }
wait_for_health "http://127.0.0.1:${CONCURRENCY_PORT}/health" || { echo "FAIL: concurrent daemon health"; exit 1; }
echo "PASS: concurrent startup lock"
kill_port_process "${CONCURRENCY_PORT}"
rm -f /tmp/cortex-concurrency-a.log /tmp/cortex-concurrency-b.log
rm -rf "${CONCURRENCY_ROOT}" || true

echo "=== All smoke tests passed ==="
