#!/usr/bin/env bash
# Minimal LongMemEval benchmark against isolated Cortex (pure adapter).
#
# Prerequisites:
#   1. bash benchmarking/setup-benchmarks.sh   # clone AMB
#   2. cd benchmarking/tools/agent-memory-benchmark && uv sync
#   3. Build or install cortex; set GEMINI_API_KEY or GROQ_API_KEY for scoring
#
# Usage:
#   bash scripts/run-longmemeval.sh
#   bash scripts/run-longmemeval.sh --query-limit 5
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python "$ROOT/benchmarking/run_longmemeval.py" "$@"
