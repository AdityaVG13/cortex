# Benchmarking

Optional lab for recall-quality measurement. Normal Cortex use does not need this directory.

## What ships here

| File | Purpose |
|------|---------|
| `run_longmemeval.py` | One-shot runner (~170 LOC): isolated daemon + pure adapter + AMB |
| `adapters/cortex_http_pure_provider.py` | Zero-tuning HTTP adapter (~95 LOC) |
| `benchmarks.lock.json` | Pinned AMB commit |
| `setup-benchmarks.sh` / `setup-benchmarks.ps1` | Clone AMB into `tools/` (gitignored) |
| `runs/` | Raw run output (gitignored) |
| `results/` | Saved headline summaries we keep in git |

## Quick start

```bash
# 1. Clone the harness
bash benchmarking/setup-benchmarks.sh

# 2. Install AMB deps (from repo root)
cd benchmarking/tools/agent-memory-benchmark && uv sync && cd -

# 3. Build cortex (or set CORTEX_BIN)
cargo build -p cortex --manifest-path daemon-rs/Cargo.toml

# 4. Set a scorer LLM key (GEMINI_API_KEY, GROQ_API_KEY, or OPENAI_API_KEY)
export GROQ_API_KEY=...

# 5. Run LongMemEval-S (20 queries by default)
bash scripts/run-longmemeval.sh
# or: python benchmarking/run_longmemeval.py --query-limit 20
```

Each run writes `benchmarking/runs/<timestamp>/` with `summary.json`, `run-manifest.json`, and AMB outputs under `outputs/`.

## Measurement contract

- **`cortex-http-pure`** is the only adapter. One `GET /recall` per query; daemon ranking returned verbatim.
- Runs use an **isolated benchmark daemon** so benchmark corpora never mix with live user memory.
- Scored runs need an answer/judge LLM key. `--oracle` is diagnostics-only (gold-doc ceiling), not a headline score.
- Until funded LongMemEval-S validation exists, do not claim recall-quality lifts in release copy.

## Purity gate

`scripts/purity-gates/adapter-surface-check.sh` keeps the pure adapter under 150 LOC and free of tuning patterns.
