#!/usr/bin/env bash
# clean_install_smoke.sh -- Verifies a fresh Cortex clone has no developer-specific leaks.
# Run from repo root:  bash scripts/clean_install_smoke.sh
set -euo pipefail

PASS=0
FAIL=0
report() { if [ "$1" = "ok" ]; then PASS=$((PASS+1)); echo "  ✓ $2"; else FAIL=$((FAIL+1)); echo "  ✗ $2"; fi; }

echo "=== Cortex clean-install smoke test ==="
echo ""

# ── 1. Source-code grep gate ──────────────────────────────────────────────────
echo "[1/5] Source-code grep gate"

if grep -rni "self-improvement-engine" daemon-rs/src/ 2>/dev/null; then
  report fail "daemon-rs/src contains 'self-improvement-engine'"
else
  report ok   "No 'self-improvement-engine' in daemon-rs/src"
fi

if grep -rni "aditya" daemon-rs/src/ 2>/dev/null; then
  report fail "daemon-rs/src contains 'aditya'"
else
  report ok   "No 'aditya' in daemon-rs/src"
fi

# ── 2. Personal tracked files ────────────────────────────────────────────────
echo ""
echo "[2/5] Personal file gate"

PERSONAL_HITS=$(git ls-files \
  CLAUDE.md AGENTS.md GEMINI.md .cursorrules PLAN.md RECON.md \
  cortex-profiles.json CHANGELOG_v0.3.0_section.md \
  cortex-start.bat cortex-app.bat cortex-dashboard.bat cortex-mcp.cmd \
  .planning/config.json .cursor/rules/005-lean-ctx-shell.mdc 2>/dev/null | wc -l)

if [ "$PERSONAL_HITS" -eq 0 ]; then
  report ok "Zero personal config files tracked in git"
else
  report fail "$PERSONAL_HITS personal file(s) still tracked"
fi

# ── 3. Build ─────────────────────────────────────────────────────────────────
echo ""
echo "[3/5] Build (cargo clippy + cargo test)"

if (cd daemon-rs && cargo clippy -- -D warnings 2>&1); then
  report ok "cargo clippy clean"
else
  report fail "cargo clippy has warnings/errors"
fi

if (cd daemon-rs && cargo test 2>&1); then
  report ok "cargo test passes"
else
  report fail "cargo test failed"
fi

# ── 4. Fresh-start behavior (no CORTEX_INDEX_EXTENDED) ──────────────────────
echo ""
echo "[4/5] Extended indexer gating"

if grep -q 'CORTEX_INDEX_EXTENDED' daemon-rs/src/indexer.rs && \
   grep -q 'CORTEX_INDEX_EXTENDED' daemon-rs/src/compiler.rs; then
  report ok "CORTEX_INDEX_EXTENDED gate present in indexer.rs and compiler.rs"
else
  report fail "Missing CORTEX_INDEX_EXTENDED gate"
fi

# ── 5. README documents the env var ──────────────────────────────────────────
echo ""
echo "[5/5] README documentation"

if grep -q 'CORTEX_INDEX_EXTENDED' README.md; then
  report ok "CORTEX_INDEX_EXTENDED documented in README.md"
else
  report fail "CORTEX_INDEX_EXTENDED missing from README.md"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
  echo "GATE: FAILED"
  exit 1
else
  echo "GATE: PASSED"
  exit 0
fi
