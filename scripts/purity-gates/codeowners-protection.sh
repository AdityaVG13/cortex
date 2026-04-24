#!/usr/bin/env bash
# Gate: critical purity-surface files must be protected in CODEOWNERS.
#
# CODEOWNERS ownership on these paths blocks silent helper reintroduction
# and CHANGELOG drift.
set -euo pipefail

CODEOWNERS="${GITHUB_WORKSPACE:-.}/CODEOWNERS"
if [[ ! -f "$CODEOWNERS" ]]; then
  CODEOWNERS=".github/CODEOWNERS"
fi
if [[ ! -f "$CODEOWNERS" ]]; then
  echo "FAIL: CODEOWNERS file missing" >&2
  exit 1
fi

REQUIRED_PATHS=(
  "/benchmarking/adapters/cortex_http_pure_provider.py"
  "/CHANGELOG.md"
  "/scripts/purity-gates/"
  "/scripts/benchmark-triad.sh"
)

for path in "${REQUIRED_PATHS[@]}"; do
  if ! grep -qE "^${path}(\s|$)" "$CODEOWNERS"; then
    echo "FAIL: $path not protected in CODEOWNERS" >&2
    exit 1
  fi
done

echo "PASS: CODEOWNERS protects purity surface"
