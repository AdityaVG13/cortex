#!/bin/bash
# Post-build smoke: spawn daemon -> health -> one store -> one recall -> shutdown.
# Isolated by construction: dedicated temp CORTEX_HOME + reserved ephemeral port,
# mirroring tests/support/harness.rs (spawn_daemon / reserve_port / wait_for_health).
# It never touches a live $HOME/.cortex and never shuts down a foreign daemon.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT) BIN_NAME="cortex.exe" ;;
    *) BIN_NAME="cortex" ;;
esac
BINARY="${1:-$ROOT/target/debug/$BIN_NAME}"

# Port: SMOKE_PORT env wins; otherwise reserve an ephemeral port the same way
# tests/support/harness.rs::reserve_port() does (bind 127.0.0.1:0, read, release).
# Fallback 7493 is deliberately != the daemon default 7437.
if [ -n "${SMOKE_PORT:-}" ]; then
    PORT="$SMOKE_PORT"
elif command -v python3 >/dev/null 2>&1; then
    PORT="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
else
    PORT=7493
fi

# Temp home; left behind intentionally (no deletions from a test script).
SMOKE_HOME="$(mktemp -d "${TMPDIR:-/tmp}/cortex-smoke-XXXXXX")"
TOKEN_FILE="$SMOKE_HOME/cortex.token"
DAEMON_PID=""
TOKEN=""

echo "=== Cortex Rust Daemon Smoke Test ==="
echo "  binary: $BINARY"
echo "  port:   $PORT"
echo "  home:   $SMOKE_HOME"

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

if [ ! -x "$BINARY" ]; then
    echo "SMOKE TEST FAILED: binary not found or not executable: $BINARY" >&2
    echo "build it with: cargo build -p cortex-daemon --bin cortex" >&2
    exit 1
fi

# Same flags + env contract as tests/support/harness.rs::spawn_daemon. The test
# bypass only exists in debug builds, hence the target/debug default binary.
CORTEX_SINGLE_DAEMON_TEST_BYPASS=1 CORTEX_BIND=127.0.0.1 \
    "$BINARY" serve --home "$SMOKE_HOME" --port "$PORT" &
DAEMON_PID=$!

# Poll health with the harness deadline (30s, 250ms interval, exit-detection).
HEALTH_OK=0
for _ in $(seq 1 120); do
    if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
        echo "daemon exited before becoming healthy (pid $DAEMON_PID)" >&2
        exit 1
    fi
    if curl -fsS "http://127.0.0.1:$PORT/health" > /dev/null 2>&1; then
        HEALTH_OK=1
        break
    fi
    sleep 0.25
done
if [ "$HEALTH_OK" -ne 1 ]; then
    echo "daemon did not become healthy on port $PORT within 30s" >&2
    exit 1
fi

TOKEN=""
for _ in $(seq 1 120); do
    if [ -s "$TOKEN_FILE" ]; then
        TOKEN="$(cat "$TOKEN_FILE")"
        [ -n "$TOKEN" ] && break
    fi
    sleep 0.25
done
if [ -z "$TOKEN" ]; then
    echo "Token file not created: $TOKEN_FILE" >&2
    exit 1
fi

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
    '"matches"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/peek?q=cortex"

run_test "GET /digest" \
    '"oneliner"' \
    curl -s "${auth_headers[@]}" "http://localhost:$PORT/digest"

run_test "GET /savings" \
    '"totals"' \
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
