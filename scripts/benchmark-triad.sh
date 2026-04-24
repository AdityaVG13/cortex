#!/usr/bin/env bash
# Run tuned / base / pure triad against a dataset.
#
# Usage: bash scripts/benchmark-triad.sh <dataset-name>
# Example: bash scripts/benchmark-triad.sh longmemeval-s
#
# Writes three JSON result files to benchmarking/results/. The pure run
# is the canonical measurement; base is deprecated and kept for
# historical comparison; tuned runs the full helper-augmented adapter
# for regression testing of the tuning layer itself.
set -euo pipefail

DATASET="${1:-longmemeval-s}"
DATE_TAG="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="benchmarking/results"

mkdir -p "$OUT_DIR"

echo "=== Tuned adapter run (helper-augmented, regression only) ==="
python benchmarking/run_amb_cortex.py \
  --memory-backend cortex-http \
  --dataset "$DATASET" \
  --output "$OUT_DIR/tuned-$DATASET-$DATE_TAG.json"

echo "=== Base adapter run (partial helpers -- deprecated) ==="
python benchmarking/run_amb_cortex.py \
  --memory-backend cortex-http-base \
  --dataset "$DATASET" \
  --output "$OUT_DIR/base-$DATASET-$DATE_TAG.json"

echo "=== Pure adapter run (zero helpers -- canonical baseline) ==="
# Strip any helper env vars from the parent shell so the pure run is
# guaranteed unadulterated. The adapter itself also refuses to initialize
# when it detects helper prefixes, but belt-and-suspenders.
for v in $(env | grep -E '^(CORTEX_BENCHMARK_|CORTEX_HELPER_|CORTEX_RERANK_|CORTEX_EXPAND_|CORTEX_LONGMEMEVAL_)' | cut -d= -f1); do
  unset "$v"
done
CORTEX_BENCHMARK_MODE=pure python benchmarking/run_amb_cortex.py \
  --memory-backend cortex-http-pure \
  --dataset "$DATASET" \
  --output "$OUT_DIR/pure-$DATASET-$DATE_TAG.json"

echo "=== Triad complete. Results: ==="
ls -la "$OUT_DIR/"{tuned,base,pure}-$DATASET-$DATE_TAG.json
