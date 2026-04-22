#!/bin/bash
set -euo pipefail

# WSL -> Windows binary networking on this host is unreliable for localhost health probes.
# Re-run under Git Bash when available to keep smoke behavior deterministic.
if [[ -z "${CORTEX_SMOKE_SKIP_WSL_REEXEC:-}" ]] \
  && grep -qi microsoft /proc/version 2>/dev/null \
  && command -v wslpath >/dev/null 2>&1; then
  GIT_BASH_EXE="/mnt/c/Program Files/Git/bin/bash.exe"
  if [[ -x "${GIT_BASH_EXE}" ]]; then
    WIN_SCRIPT_PATH="$(wslpath -w "$0")"
    CORTEX_SMOKE_SKIP_WSL_REEXEC=1 "${GIT_BASH_EXE}" "${WIN_SCRIPT_PATH}" "$@"
    exit $?
  fi
fi

echo "=== Cortex Release Smoke Test ==="

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
  if [[ -f "${REPO_ROOT}/daemon-rs/target/release/cortex.exe" ]]; then
    CORTEX_BIN="${REPO_ROOT}/daemon-rs/target/release/cortex.exe"
  elif [[ -x "${REPO_ROOT}/daemon-rs/target/release/cortex" ]]; then
    CORTEX_BIN="${REPO_ROOT}/daemon-rs/target/release/cortex"
  elif [[ -f "${REPO_ROOT}/daemon-rs/target/debug/cortex.exe" ]]; then
    CORTEX_BIN="${REPO_ROOT}/daemon-rs/target/debug/cortex.exe"
  elif [[ -x "${REPO_ROOT}/daemon-rs/target/debug/cortex" ]]; then
    CORTEX_BIN="${REPO_ROOT}/daemon-rs/target/debug/cortex"
  elif command -v cortex >/dev/null 2>&1; then
    CORTEX_BIN="cortex"
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
  for maybe_pid in "${DAEMON_PID:-}" "${MIGRATION_PID:-}" "${CONCURRENCY_DAEMON_PID:-}" "${PID_A:-}" "${PID_B:-}"; do
    if [[ -n "${maybe_pid}" ]]; then
      kill "${maybe_pid}" 2>/dev/null || true
    fi
  done
  for maybe_port in "${SMOKE_PORT:-}" "${MIGRATION_PORT:-}" "${CONCURRENCY_PORT:-}" "9999"; do
    if [[ -n "${maybe_port}" ]]; then
      kill_port_process "${maybe_port}" || true
    fi
  done
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
  HOME="${SMOKE_HOME}" \
  USERPROFILE="${SMOKE_HOME}" \
  CORTEX_HOME="${SMOKE_CORTEX_HOME}" \
  CORTEX_PORT="${SMOKE_PORT}" \
  CORTEX_GLOBAL_LOCK_HOME="${SMOKE_CORTEX_HOME}" \
  "${CORTEX_BIN}" "$@"
}

run_cortex_for_home() {
  local cortex_home="$1"
  shift
  local user_home
  user_home="$(dirname "${cortex_home}")"
  HOME="${user_home}" \
  USERPROFILE="${user_home}" \
  CORTEX_HOME="${cortex_home}" \
  CORTEX_GLOBAL_LOCK_HOME="${cortex_home}" \
  "${CORTEX_BIN}" "$@"
}

wait_for_health() {
  local url="$1"
  if [[ "${CORTEX_BIN}" == *.exe || "${CORTEX_BIN}" == *.EXE ]]; then
    CORTEX_HEALTH_URL="${url}" powershell.exe -NoProfile -NonInteractive -Command '
      $ErrorActionPreference = "Stop"
      $url = $env:CORTEX_HEALTH_URL
      for ($i = 0; $i -lt 40; $i++) {
        try {
          $resp = Invoke-RestMethod -Uri $url -TimeoutSec 2
          if ($resp.status -eq "ok") { exit 0 }
        } catch {}
        Start-Sleep -Milliseconds 500
      }
      exit 1
    '
    return
  fi
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
    pids="$(netstat -ano 2>/dev/null | awk -v port=":${port}" '$0 ~ port && $0 ~ /LISTENING/ {print $5}' | sort -u || true)"
    for pid in ${pids}; do
      taskkill //PID "${pid}" //F >/dev/null 2>&1 || kill "${pid}" 2>/dev/null || true
    done
  fi
}

# 1. Binary exists and runs
"${CORTEX_BIN}" --help || { echo "FAIL: cortex binary not found"; exit 1; }

# 2. Paths command works and is scoped to smoke home (no host path leaks)
PATHS_JSON_FILE="${SMOKE_ROOT}/paths.json"
SMOKE_SENTINEL="$(basename "${SMOKE_ROOT}")"
run_cortex paths --json --home "${SMOKE_CORTEX_HOME}" > "${PATHS_JSON_FILE}" \
  || { echo "FAIL: cortex paths --json failed"; exit 1; }
"${PYTHON_CMD[@]}" - "${SMOKE_SENTINEL}" "${PATHS_JSON_FILE}" <<'PY' \
  || { echo "FAIL: cortex paths --json invalid or leaked host paths"; exit 1; }
import json
import sys

smoke_sentinel = sys.argv[1].lower()
payload_path = sys.argv[2]
with open(payload_path, "r", encoding="utf-8") as handle:
    payload = json.load(handle)
for required in ("home", "db", "token", "pid", "port"):
    assert required in payload, f"missing field: {required}"

def norm(path_value):
    return str(path_value).replace("\\", "/").lower().rstrip("/")

home = norm(payload["home"])
assert smoke_sentinel in home, f"home path missing smoke sentinel: {home}"
assert home.endswith("/.cortex"), f"home path not scoped to .cortex: {home}"

for field, suffix in (
    ("db", "/cortex.db"),
    ("token", "/cortex.token"),
    ("pid", "/cortex.pid"),
    ("models", "/models"),
):
    assert field in payload, f"missing field: {field}"
    value = norm(payload[field])
    assert smoke_sentinel in value, f"{field} path missing smoke sentinel: {value}"
    assert value.startswith(home + "/"), f"{field} path leaked outside smoke home: {value}"
    assert value.endswith(suffix), f"{field} path has unexpected suffix: {value}"
PY
echo "PASS: paths --json scoped to smoke home"

# 3. Serve starts the daemon
run_cortex serve --home "${SMOKE_CORTEX_HOME}" --port "${SMOKE_PORT}" &
DAEMON_PID=$!

# 4. Health check
wait_for_health "http://127.0.0.1:${SMOKE_PORT}/health" \
  || { echo "FAIL: health check"; exit 1; }
echo "PASS: health check"

# 5. Duplicate daemon start is rejected (one-daemon invariant)
set +e
DUPLICATE_START_OUTPUT="$(run_cortex serve 2>&1)"
DUPLICATE_START_STATUS=$?
set -e
if [ "${DUPLICATE_START_STATUS}" -eq 0 ]; then
  if echo "${DUPLICATE_START_OUTPUT}" | grep -Eiq "already healthy on port|exiting cleanly"; then
    echo "PASS: duplicate serve idempotent attach"
  else
    echo "FAIL: duplicate serve unexpectedly succeeded"
    echo "${DUPLICATE_START_OUTPUT}"
    exit 1
  fi
elif echo "${DUPLICATE_START_OUTPUT}" | grep -Eiq "already|active daemon process|another cortex instance|startup denied"; then
  echo "PASS: duplicate serve rejected"
else
  echo "FAIL: duplicate serve rejection message missing expected markers"
  echo "${DUPLICATE_START_OUTPUT}"
  exit 1
fi

# 6. ensure-daemon attaches to the already-running daemon
run_cortex plugin ensure-daemon --agent smoke-test --home "${SMOKE_CORTEX_HOME}" --port "${SMOKE_PORT}" \
  || { echo "FAIL: ensure-daemon attach"; exit 1; }
echo "PASS: ensure-daemon attach"

# 7. Store + recall round-trip via MCP (REQUIRES 1B + 1C)
STORE_RESPONSE="$(
  echo '{"jsonrpc":"2.0","method":"tools/call","params":{"name":"cortex_store","arguments":{"decision":"smoke test memory","context":"release verification"}},"id":1}' \
    | run_cortex plugin mcp --agent smoke-test
)"
echo "${STORE_RESPONSE}" \
  | "${PYTHON_CMD[@]}" -c "import sys,json; d=json.load(sys.stdin); assert 'result' in d" \
  || { echo "FAIL: store via MCP"; echo "Store response: ${STORE_RESPONSE}"; exit 1; }
echo "PASS: store"

# 8. Recall what we just stored
RECALL_RESPONSE="$(
  echo '{"jsonrpc":"2.0","method":"tools/call","params":{"name":"cortex_recall","arguments":{"query":"smoke test","budget":100}},"id":2}' \
    | run_cortex plugin mcp --agent smoke-test
)"
echo "${RECALL_RESPONSE}" \
  | "${PYTHON_CMD[@]}" -c "import sys,json; d=json.load(sys.stdin); assert 'result' in d" \
  || { echo "FAIL: recall via MCP"; echo "Recall response: ${RECALL_RESPONSE}"; exit 1; }
echo "PASS: recall"
kill "${DAEMON_PID}" 2>/dev/null || true
kill_port_process "${SMOKE_PORT}"

# 9. Custom port works (REQUIRES 1D + 1E)
CUSTOM_HOME="${SMOKE_ROOT}/custom-home"
mkdir -p "${CUSTOM_HOME}/.cortex"
run_cortex_for_home "${CUSTOM_HOME}/.cortex" serve --home "${CUSTOM_HOME}/.cortex" --port 9999 &
DAEMON_PID=$!
wait_for_health "http://127.0.0.1:9999/health" && echo "PASS: custom port" || echo "FAIL: custom port"
kill $DAEMON_PID 2>/dev/null
kill_port_process 9999

# 10. Legacy migration works (~/cortex -> ~/.cortex)
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
run_cortex_for_home "${MIGRATION_HOME}/.cortex" serve --home "${MIGRATION_HOME}/.cortex" --port "${MIGRATION_PORT}" &
MIGRATION_PID=$!
wait_for_health "http://127.0.0.1:${MIGRATION_PORT}/health" || { echo "FAIL: legacy migration health"; exit 1; }
test -f "${MIGRATION_HOME}/.cortex/cortex.db" || { echo "FAIL: migrated db missing at ~/.cortex/cortex.db"; exit 1; }
echo "PASS: legacy migration"
kill "${MIGRATION_PID}" 2>/dev/null || true
kill_port_process "${MIGRATION_PORT}"
rm -rf "${MIGRATION_ROOT}" || true

# 11. Concurrent ensure-daemon attach is safe once daemon is already healthy
CONCURRENCY_ROOT="$(mktemp -d)"
CONCURRENCY_HOME="${CONCURRENCY_ROOT}/home"
mkdir -p "${CONCURRENCY_HOME}"
CONCURRENCY_PORT=9996
run_cortex_for_home "${CONCURRENCY_HOME}/.cortex" serve --home "${CONCURRENCY_HOME}/.cortex" --port "${CONCURRENCY_PORT}" &
CONCURRENCY_DAEMON_PID=$!
wait_for_health "http://127.0.0.1:${CONCURRENCY_PORT}/health" || { echo "FAIL: concurrency daemon health"; exit 1; }
run_cortex_for_home "${CONCURRENCY_HOME}/.cortex" plugin ensure-daemon --agent smoke-concurrency-A --home "${CONCURRENCY_HOME}/.cortex" --port "${CONCURRENCY_PORT}" > /tmp/cortex-concurrency-a.log 2>&1 &
PID_A=$!
run_cortex_for_home "${CONCURRENCY_HOME}/.cortex" plugin ensure-daemon --agent smoke-concurrency-B --home "${CONCURRENCY_HOME}/.cortex" --port "${CONCURRENCY_PORT}" > /tmp/cortex-concurrency-b.log 2>&1 &
PID_B=$!
wait "$PID_A" || { echo "FAIL: concurrent ensure-daemon A"; cat /tmp/cortex-concurrency-a.log; exit 1; }
wait "$PID_B" || { echo "FAIL: concurrent ensure-daemon B"; cat /tmp/cortex-concurrency-b.log; exit 1; }
echo "PASS: concurrent attach check"
kill "${CONCURRENCY_DAEMON_PID}" 2>/dev/null || true
kill_port_process "${CONCURRENCY_PORT}"
rm -f /tmp/cortex-concurrency-a.log /tmp/cortex-concurrency-b.log
rm -rf "${CONCURRENCY_ROOT}" || true

echo "=== All smoke tests passed ==="
