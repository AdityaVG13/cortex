#!/usr/bin/env bash
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
  "/tests/purity-gates/"
  "/scripts/run-longmemeval.sh"
  "/benchmarking/run_longmemeval.py"
)

for path in "${REQUIRED_PATHS[@]}"; do
  if ! grep -qE "^${path}(\s|$)" "$CODEOWNERS"; then
    echo "FAIL: $path not protected in CODEOWNERS" >&2
    exit 1
  fi
done

echo "PASS: CODEOWNERS protects purity surface"
