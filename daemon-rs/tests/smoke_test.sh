#!/bin/bash
# Smoke test for Cortex Rust daemon
set -e

BINARY="${1:-../target/release/cortex.exe}"
PORT=7437
TOKEN_FILE="$HOME/.cortex/cortex.token"

echo "=== Cortex Rust Daemon Smoke Test ==="

# Stop any running daemon
if curl -s http://localhost:$PORT/health > /dev/null 2>&1; then
    TOKEN=$(cat "$TOKEN_FILE" 2>/dev/null || echo "")
    if [ -n "$TOKEN" ]; then
        curl -s -X POST -H "Authorization: Bearer $TOKEN" http://localhost:$PORT/shutdown > /dev/null 2>&1 || true
        sleep 2
    fi
fi

# Start daemon in background
$BINARY serve &
DAEMON_PID=$!
sleep 3

# Read new token
TOKEN=$(cat "$TOKEN_FILE")

PASS=0
FAIL=0

run_test() {
    local name="$1"
    local cmd="$2"
    local expected="$3"

    result=$(eval "$cmd" 2>/dev/null || echo "CURL_FAILED")
    if echo "$result" | grep -q "$expected"; then
        echo "  ✓ $name"
        PASS=$((PASS + 1))
    else
        echo "  ✗ $name — expected '$expected', got: $(echo $result | head -c 200)"
        FAIL=$((FAIL + 1))
    fi
}

echo ""
echo "--- Core Endpoints ---"
run_test "GET /health" \
    "curl -s http://localhost:$PORT/health" \
    '"status":"ok"'

run_test "GET /boot" \
    "curl -s 'http://localhost:$PORT/boot?agent=test&budget=600'" \
    '"bootPrompt"'

run_test "GET /recall" \
    "curl -s 'http://localhost:$PORT/recall?q=cortex'" \
    '"results"'

run_test "GET /peek" \
    "curl -s 'http://localhost:$PORT/peek?q=cortex'" \
    '"matches"'

run_test "GET /digest" \
    "curl -s http://localhost:$PORT/digest" \
    '"oneliner"'

run_test "GET /savings" \
    "curl -s http://localhost:$PORT/savings" \
    '"summary"'

echo ""
echo "--- Auth-Required Endpoints ---"
run_test "POST /store" \
    "curl -s -X POST -H 'Authorization: Bearer $TOKEN' -H 'Content-Type: application/json' http://localhost:$PORT/store -d '{\"decision\":\"smoke test\",\"context\":\"integration test\"}'" \
    '"stored":true'

run_test "POST /store (no auth)" \
    "curl -s -X POST -H 'Content-Type: application/json' http://localhost:$PORT/store -d '{\"decision\":\"test\"}'" \
    '"error"'

run_test "GET /recall/budget" \
    "curl -s 'http://localhost:$PORT/recall/budget?q=smoke+test&budget=200'" \
    '"results"'

run_test "POST /forget" \
    "curl -s -X POST -H 'Authorization: Bearer $TOKEN' -H 'Content-Type: application/json' http://localhost:$PORT/forget -d '{\"keyword\":\"smoke test\"}'" \
    '"affected"'

run_test "GET /dump" \
    "curl -s -H 'Authorization: Bearer $TOKEN' http://localhost:$PORT/dump" \
    '"memories"'

echo ""
echo "--- Conductor Endpoints ---"
run_test "GET /sessions" \
    "curl -s -H 'Authorization: Bearer $TOKEN' http://localhost:$PORT/sessions" \
    '"sessions"'

run_test "GET /tasks" \
    "curl -s -H 'Authorization: Bearer $TOKEN' http://localhost:$PORT/tasks" \
    '"tasks"'

run_test "GET /locks" \
    "curl -s -H 'Authorization: Bearer $TOKEN' http://localhost:$PORT/locks" \
    '"locks"'

run_test "GET /feed" \
    "curl -s -H 'Authorization: Bearer $TOKEN' http://localhost:$PORT/feed" \
    '"entries"'

echo ""
echo "--- Results ---"
echo "  Passed: $PASS"
echo "  Failed: $FAIL"

# Shutdown
curl -s -X POST -H "Authorization: Bearer $TOKEN" http://localhost:$PORT/shutdown > /dev/null 2>&1
wait $DAEMON_PID 2>/dev/null

if [ $FAIL -gt 0 ]; then
    echo "SMOKE TEST FAILED"
    exit 1
else
    echo "ALL TESTS PASSED"
    exit 0
fi
