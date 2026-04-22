#!/usr/bin/env bash
# clean_install_smoke.sh -- Verifies a fresh Cortex clone has no developer-specific leaks.
# Usage: bash scripts/clean_install_smoke.sh

set -eu
if (set -o pipefail) >/dev/null 2>&1; then
  set -o pipefail
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"
cd "${REPO_ROOT}"

PASS=0
FAIL=0
CARGO_CMD=""
PYTHON_CMD=""

report() {
  if [ "$1" = "ok" ]; then
    PASS=$((PASS + 1))
    echo "  [OK] $2"
  else
    FAIL=$((FAIL + 1))
    echo "  [FAIL] $2"
  fi
}

require_cmd() {
  local cmd="$1"
  if command -v "${cmd}" >/dev/null 2>&1; then
    report ok "Found required command: ${cmd}"
    return 0
  fi
  report fail "Missing required command: ${cmd}"
  return 1
}

resolve_cargo_cmd() {
  if command -v cargo >/dev/null 2>&1; then
    CARGO_CMD="cargo"
    return 0
  fi
  if command -v cargo.exe >/dev/null 2>&1; then
    CARGO_CMD="cargo.exe"
    return 0
  fi
  CARGO_CMD=""
  return 1
}

resolve_python_cmd() {
  if command -v python3 >/dev/null 2>&1; then
    PYTHON_CMD="python3"
    return 0
  fi
  if command -v python >/dev/null 2>&1; then
    PYTHON_CMD="python"
    return 0
  fi
  if command -v py >/dev/null 2>&1; then
    PYTHON_CMD="py -3"
    return 0
  fi
  PYTHON_CMD=""
  return 1
}

search_tree() {
  local pattern="$1"
  local path="$2"
  if command -v rg >/dev/null 2>&1; then
    rg -n \
      --glob '!**/graphify-out/**' \
      --glob '!**/target/**' \
      "${pattern}" \
      "${path}"
    return $?
  fi
  grep -rni \
    --exclude-dir=graphify-out \
    --exclude-dir=target \
    "${pattern}" \
    "${path}" \
    2>/dev/null
}

show_output_limited() {
  local output="$1"
  printf '%s\n' "${output}" | head -n 20 || true
}

check_pattern_absent() {
  local pattern="$1"
  local path="$2"
  local ok_message="$3"
  local fail_message="$4"
  local output=""
  local status=0

  set +e
  output="$(search_tree "${pattern}" "${path}" 2>&1)"
  status=$?
  set -e

  if [ "${status}" -eq 1 ]; then
    report ok "${ok_message}"
    return 0
  fi

  if [ "${status}" -eq 0 ]; then
    report fail "${fail_message}"
    show_output_limited "${output}"
    return 1
  fi

  report fail "Search failed (${status}) for pattern '${pattern}' in ${path}"
  show_output_limited "${output}"
  return 1
}

echo "=== Cortex clean-install smoke test ==="
echo ""

echo "[0/6] Preflight command gate"
missing_tools=0
for cmd in git; do
  if ! require_cmd "${cmd}"; then
    missing_tools=1
  fi
done
if resolve_cargo_cmd; then
  report ok "Found required command: ${CARGO_CMD}"
else
  report fail "Missing required command: cargo (or cargo.exe)"
  missing_tools=1
fi
if resolve_python_cmd; then
  report ok "Found required command: ${PYTHON_CMD}"
else
  report fail "Missing required command: python3/python/py"
  missing_tools=1
fi
if [ "${missing_tools}" -ne 0 ]; then
  echo ""
  echo "GATE: FAILED (missing required commands)"
  exit 1
fi

# 1. Source-code grep gate
echo ""
echo "[1/5] Source-code grep gate"

check_pattern_absent \
  "self-improvement-engine" \
  "daemon-rs/src/" \
  "No 'self-improvement-engine' in daemon-rs/src" \
  "daemon-rs/src contains 'self-improvement-engine'"

check_pattern_absent \
  "C:\\\\Users\\\\[A-Za-z0-9_.-]+|/Users/[A-Za-z0-9_.-]+" \
  "daemon-rs/src/" \
  "No hardcoded developer home paths in daemon-rs/src" \
  "daemon-rs/src contains hardcoded developer home path(s)"

check_pattern_absent \
  "C:\\\\Users\\\\[A-Za-z0-9_.-]+|/Users/[A-Za-z0-9_.-]+" \
  "plugins/cortex-plugin/scripts/" \
  "No hardcoded developer home paths in plugin startup scripts" \
  "Plugin startup scripts contain hardcoded developer home path(s)"

check_pattern_absent \
  "CORTEX_SINGLE_DAEMON_TEST_BYPASS.?=.?1" \
  "plugins/cortex-plugin/scripts/" \
  "No singleton test-bypass env forcing in plugin startup scripts" \
  "Plugin startup scripts contain forced singleton test-bypass env"

# 2. Personal tracked files
echo ""
echo "[2/5] Personal file gate"

PERSONAL_FILES=(
  "CLAUDE.md"
  "AGENTS.md"
  "GEMINI.md"
  ".cursorrules"
  "PLAN.md"
  "RECON.md"
  "cortex-profiles.json"
  "CHANGELOG_v0.3.0_section.md"
  "cortex-start.bat"
  "cortex-app.bat"
  "cortex-dashboard.bat"
  "cortex-mcp.cmd"
  ".planning/config.json"
  ".cursor/rules/005-lean-ctx-shell.mdc"
)

PERSONAL_HITS=()
for file in "${PERSONAL_FILES[@]}"; do
  if git ls-files --error-unmatch "${file}" >/dev/null 2>&1; then
    PERSONAL_HITS+=("${file}")
  fi
done

if [ "${#PERSONAL_HITS[@]}" -eq 0 ]; then
  report ok "Zero personal config files tracked in git"
else
  report fail "${#PERSONAL_HITS[@]} personal file(s) still tracked"
  printf '    %s\n' "${PERSONAL_HITS[@]}"
fi

# 3. Build
echo ""
echo "[3/5] Build (cargo clippy + cargo test)"

if (cd daemon-rs && "${CARGO_CMD}" clippy -- -D warnings >/dev/null 2>&1); then
  report ok "cargo clippy clean"
else
  report fail "cargo clippy has warnings/errors"
fi

if (cd daemon-rs && "${CARGO_CMD}" test >/dev/null 2>&1); then
  report ok "cargo test passes"
else
  report fail "cargo test failed"
fi

# 4. No hardcoded knowledge paths in source
echo ""
echo "[4/5] No hardcoded source paths"

check_pattern_absent \
  "knowledge-sources|extended-knowledge" \
  "daemon-rs/src/" \
  "Zero hardcoded knowledge paths in daemon-rs/src" \
  "daemon-rs/src still contains hardcoded knowledge paths"

if grep -q 'index_custom_sources' daemon-rs/src/indexer.rs &&
   grep -q 'sources.toml' daemon-rs/src/indexer.rs; then
  report ok "Custom sources config system present in indexer.rs"
else
  report fail "Missing custom sources config in indexer.rs"
fi

# 5. README documents custom sources
echo ""
echo "[5/6] README documentation"

if grep -Eq 'sources\.toml|CORTEX_EXTRA_SOURCES' README.md; then
  report ok "Custom sources documented in README.md"
else
  if [ "${CORTEX_ENFORCE_PUBLIC_README:-0}" = "1" ]; then
    report fail "Custom sources missing from README.md"
  else
    report ok "Public README check deferred (set CORTEX_ENFORCE_PUBLIC_README=1 to enforce)"
  fi
fi

# 6. Strict daemon spawn-path audit
echo ""
echo "[6/6] Spawn-path strict audit"
set +e
AUDIT_OUTPUT="$(${PYTHON_CMD} tools/audit_spawn_paths.py --strict 2>&1)"
AUDIT_STATUS=$?
set -e
if [ "${AUDIT_STATUS}" -eq 0 ]; then
  report ok "tools/audit_spawn_paths.py --strict passes"
else
  report fail "tools/audit_spawn_paths.py --strict failed"
  show_output_limited "${AUDIT_OUTPUT}"
fi

# Summary
echo ""
echo "=== Results: ${PASS} passed, ${FAIL} failed ==="
if [ "${FAIL}" -gt 0 ]; then
  echo "GATE: FAILED"
  exit 1
fi

echo "GATE: PASSED"
exit 0
