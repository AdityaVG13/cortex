#!/bin/bash
set -euo pipefail

BINARY="${1:-../target/release/cortex.exe}"
PORT=7437
TOKEN_FILE="$HOME/.cortex/cortex.token"
DAEMON_PID=""
TOKEN=""

echo "=== Cortex Rust Daemon Smoke Test ==="

cleanup() {
    local token="${TOKEN:-}"
    if [ -z "$token" ] && [ -f "$TOKEN_FILE" ]; then
        token=$(cat "$TOKEN_FILE" 2>/dev/null || true)
    fi
    if [ -n "$token" ]; then
        curl -s -X POST \
          -H "Authorization: Bearer $token" \
          -H "X-Cortex-Request: true" \
          "http://localhost:$PORT/shutdown" > /dev/null 2>&1 || true
    fi
    if [ -n "${DAEMON_PID:-}" ]; then
        for _ in 1 2 3 4 5 6 7 8 9 10; do
            if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
                wait "$DAEMON_PID" 2>/dev/null || true
                return
            fi
            sleep 0.5
        done
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

if curl -s "http://localhost:$PORT/health" > /dev/null 2>&1; then
    TOKEN=$(cat "$TOKEN_FILE" 2>/dev/null || echo "")
    if [ -n "$TOKEN" ]; then
        curl -s -X POST \
          -H "Authorization: Bearer $TOKEN" \
          -H "X-Cortex-Request: true" \
          "http://localhost:$PORT/shutdown" > /dev/null 2>&1 || true
        sleep 2
    fi
fi

"$BINARY" serve &
DAEMON_PID=$!
sleep 3

if [ ! -s "$TOKEN_FILE" ]; then
    echo "Token file not created: $TOKEN_FILE"
    exit 1
fi
TOKEN=$(cat "$TOKEN_FILE")

PASS=0
FAIL=0

run_test() {
    local name="$1"
    local expected="$2"
    shift 2

    local result
    if ! result=$("$@" 2>/dev/null); then
        result="CURL_FAILED"
    fi
    if printf '%s' "$result" | grep -Fq "$expected"; then
        echo "  ✓ $name"
        PASS=$((PASS + 1))
    else
        echo "  ✗ $name — expected '$expected', got: $(printf '%s' "$result" | head -c 200)"
        FAIL=$((FAIL + 1))
    fi
}

auth_headers=(-H "Authorization: Bearer $TOKEN" -H "X-Cortex-Request: true")

echo ""
echo "--- Core Endpoints ---"
run_test "GET /health" \
    '"status":"ok"' \
    curl -s "http://localhost:$PORT/health"

run_test "GET /boot" \
    '"bootPrompt"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/boot?agent=test&budget=600"

run_test "GET /recall" \
    '"results"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/recall?q=cortex"

run_test "GET /peek" \
    '"results"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/peek?q=cortex"

run_test "GET /digest" \
    '"oneliner"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/digest"

run_test "GET /savings" \
    '"summary"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/savings"

echo ""
echo "--- Auth-Required Endpoints ---"
run_test "POST /store" \
    '"stored":true' \
    curl -s -X POST "${auth_headers[@]}" -H "Content-Type: application/json" "http://localhost:$PORT/store" -d '{"decision":"smoke test","context":"integration test"}'

run_test "POST /store (no auth)" \
    '"error"' \
    curl -s -X POST -H "X-Cortex-Request: true" -H "Content-Type: application/json" "http://localhost:$PORT/store" -d '{"decision":"test"}'

run_test "GET /recall/budget" \
    '"results"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/recall/budget?q=smoke+test&budget=200"

run_test "POST /forget" \
    '"affected"' \
    curl -s -X POST "${auth_headers[@]}" -H "Content-Type: application/json" "http://localhost:$PORT/forget" -d '{"source":"smoke test"}'

run_test "GET /dump" \
    '"memories"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/dump"

echo ""
echo "--- Conductor Endpoints ---"
run_test "GET /sessions" \
    '"sessions"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/sessions"

run_test "GET /tasks" \
    '"tasks"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/tasks"

run_test "GET /locks" \
    '"locks"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/locks"

run_test "GET /feed" \
    '"entries"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/feed"

echo ""
echo "--- Results ---"
echo "  Passed: $PASS"
echo "  Failed: $FAIL"

if [ $FAIL -gt 0 ]; then
    echo "SMOKE TEST FAILED"
    exit 1
else
    echo "ALL TESTS PASSED"
    exit 0
fi
