#!/usr/bin/env bash
# Gate: a pure-mode run must have zero helper env vars set.
#
# Scoped: only enforces when CORTEX_BENCHMARK_MODE=pure is in the caller's
# environment. When another mode is active (or when running CI lint without
# that flag), this script prints SKIP and exits 0.
set -euo pipefail

MODE="${CORTEX_BENCHMARK_MODE:-}"
if [[ "$MODE" != "pure" ]]; then
  echo "SKIP: gate only enforces when CORTEX_BENCHMARK_MODE=pure (got: ${MODE:-<unset>})"
  exit 0
fi

violations=$(env | grep -E '^(CORTEX_BENCHMARK_|CORTEX_HELPER_|CORTEX_RERANK_|CORTEX_EXPAND_|CORTEX_LONGMEMEVAL_)' | grep -v '^CORTEX_BENCHMARK_MODE=' || true)

if [[ -n "$violations" ]]; then
  echo "FAIL: helper env vars set during pure-mode run:" >&2
  echo "$violations" >&2
  exit 1
fi
echo "PASS: no helper env vars set"
