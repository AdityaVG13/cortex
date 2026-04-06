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

echo "=== All smoke tests passed ==="
