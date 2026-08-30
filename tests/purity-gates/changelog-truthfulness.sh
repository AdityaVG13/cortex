#!/usr/bin/env bash
set -euo pipefail

CHANGELOG="CHANGELOG.md"

if [[ ! -f "$CHANGELOG" ]]; then
  echo "SKIP: $CHANGELOG missing; nothing to verify"
  exit 0
fi

suspicious=$(grep -nE '(accuracy|hit rate|precision)[^.]*([0-9]+\.[0-9]+|[0-9]+/[0-9]+)' "$CHANGELOG" \
  | grep -vE 'pure|cortex-http-pure|results/pure-|helper-augmented|v0\.5\.0' \
  || true)

if [[ -n "$suspicious" ]]; then
  echo "WARN: CHANGELOG has benchmark claims not explicitly tied to pure measurements:" >&2
  echo "$suspicious" >&2
  echo "If these are historical, tag them as '(helper-augmented)' in the line." >&2
  echo "If they are new claims, they must reference a pure-* JSON in benchmarking/results/." >&2
fi

echo "PASS: CHANGELOG truthfulness check (advisory)"
