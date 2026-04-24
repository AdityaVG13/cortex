#!/usr/bin/env bash
# Gate: every benchmark claim in CHANGELOG.md should be tied to a pure
# measurement JSON in benchmarking/results/ (advisory in v0.6.0, blocking
# once pure results land).
#
# Heuristic: lines containing an accuracy / hit rate / precision number
# should also mention 'pure' or 'cortex-http-pure' or a pure-* result file.
# Historical (pre-v0.6.0) claims tagged with '(helper-augmented)' are
# exempt.
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
  # Advisory in v0.6.0. Flip to `exit 1` once v0.6.0 ships with all-clean claims.
fi

echo "PASS: CHANGELOG truthfulness check (advisory)"
