#!/usr/bin/env bash
set -euo pipefail

ALLOWED='is_benchmark_recall_scope|is_benchmark_event_source|is_benchmark_entry_type|is_benchmark_source_agent|BENCHMARK_ENTRY_TYPE|BENCHMARK_SOURCE_AGENT_PREFIX'

FORBIDDEN=$(grep -rn -E 'is_benchmark|benchmark_mode|BENCHMARK_MODE|bench_hint' crates/daemon/src/ \
  --include='*.rs' \
  | grep -vE "$ALLOWED" \
  | grep -vE '^[^:]+:[0-9]+:\s*//' \
  | grep -v '#\[test\]' \
  | grep -v '#\[cfg(test)\]' \
  || true)

if [[ -n "$FORBIDDEN" ]]; then
  echo "FAIL: daemon contains benchmark-mode branches beyond allowed whitelist:" >&2
  echo "$FORBIDDEN" >&2
  echo >&2
  echo "If this is a new legitimate benchmark path, update the ALLOWED" >&2
  echo "whitelist in tests/purity-gates/daemon-clean.sh AND document" >&2
  echo "the rationale in the commit message." >&2
  exit 1
fi

echo "PASS: daemon clean of benchmark-mode branches (whitelist only)"
