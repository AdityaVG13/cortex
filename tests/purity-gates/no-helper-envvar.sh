#!/usr/bin/env bash
set -euo pipefail

# Enforces by default: any run this gate is wired into (smoke lane, purity
# lane) must be free of helper/benchmark env vars. Opt out ONLY for explicit
# helper-augmented benchmark runs by setting CORTEX_PURITY_ALLOW_HELPER_ENV=1
# in that run's environment.

if [[ "${CORTEX_PURITY_ALLOW_HELPER_ENV:-0}" == "1" ]]; then
  echo "SKIP: helper env vars explicitly permitted via CORTEX_PURITY_ALLOW_HELPER_ENV=1"
  exit 0
fi

violations=$(env | grep -E '^(CORTEX_BENCHMARK_|CORTEX_HELPER_|CORTEX_RERANK_|CORTEX_EXPAND_|CORTEX_LONGMEMEVAL_)' | grep -v '^CORTEX_BENCHMARK_MODE=' || true)

if [[ -n "$violations" ]]; then
  echo "FAIL: helper env vars set during pure-mode run:" >&2
  echo "$violations" >&2
  exit 1
fi
echo "PASS: no helper env vars set"
