#!/usr/bin/env bash
# Gate: daemon-rs/src/ must not branch on benchmark-mode detection in
# ways that change recall/scoring behavior.
#
# ALLOWED (whitelisted below) -- these are documented benchmark-path
# functions that preserve ingestion fidelity or emit analytics without
# changing recall ranking:
#
#   is_benchmark_recall_scope       -- analytics-tagging on recall
#   is_benchmark_event_source       -- event-bus source filter
#   is_benchmark_entry_type         -- store-path: skip dedup/conflict
#                                      collapse on benchmark ingest
#   is_benchmark_source_agent       -- store-path: skip truncation
#                                      on benchmark ingest
#   BENCHMARK_ENTRY_TYPE            -- const used by the above
#   BENCHMARK_SOURCE_AGENT_PREFIX   -- const used by the above
#
# Adding a new benchmark-mode branch requires updating both this
# whitelist and getting maintainer sign-off (CODEOWNERS enforces this
# for scripts/purity-gates/). Everything else -- benchmark_mode,
# BENCHMARK_MODE, bench_hint, or new is_benchmark_* function names --
# fails this gate.
set -euo pipefail

ALLOWED='is_benchmark_recall_scope|is_benchmark_event_source|is_benchmark_entry_type|is_benchmark_source_agent|BENCHMARK_ENTRY_TYPE|BENCHMARK_SOURCE_AGENT_PREFIX'

FORBIDDEN=$(grep -rn -E 'is_benchmark|benchmark_mode|BENCHMARK_MODE|bench_hint' daemon-rs/src/ \
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
  echo "whitelist in scripts/purity-gates/daemon-clean.sh AND document" >&2
  echo "the rationale in the commit message." >&2
  exit 1
fi

echo "PASS: daemon clean of benchmark-mode branches (whitelist only)"
