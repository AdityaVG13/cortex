#!/usr/bin/env bash
# Gate: cortex-http-pure adapter must stay minimal.
#
# Enforces:
#   - File exists at the canonical path
#   - <= 150 lines of non-blank, non-comment code (helpers creeping in
#     would show up as fresh branching logic)
#   - No forbidden patterns that signal adapter-side tuning
set -euo pipefail

FILE="benchmarking/adapters/cortex_http_pure_provider.py"
LIMIT=150

if [[ ! -f "$FILE" ]]; then
  echo "FAIL: $FILE missing" >&2
  exit 1
fi

LOC=$(grep -cvE '^\s*($|#)' "$FILE")
if (( LOC > LIMIT )); then
  echo "FAIL: $FILE has $LOC LOC (limit $LIMIT). Helpers likely creeping in." >&2
  exit 1
fi

forbidden_patterns=(
  "_rerank_"
  "_expand_"
  "_promote_"
  "_boost"
  "detail_bonus"
  "query_intent"
  "sibling"
  "family_key"
)
for pattern in "${forbidden_patterns[@]}"; do
  if grep -q "$pattern" "$FILE"; then
    echo "FAIL: $FILE contains forbidden pattern: $pattern" >&2
    exit 1
  fi
done

echo "PASS: pure adapter is minimal ($LOC LOC, no forbidden patterns)"
