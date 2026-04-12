# Benchmarking Workspace

This directory is the reproducible home for external benchmark harnesses, Cortex adapters, configs, and saved benchmark outputs.

It intentionally separates:

- `benchmark/`: existing in-repo Cortex recall/metric scripts
- `benchmarking/`: external benchmark suites plus the glue needed to run Cortex against them

## Layout

- `benchmarks.lock.json`: pinned external benchmark repos and their target commits
- `setup-benchmarks.ps1`: clone/update the external suites into `benchmarking/tools/`
- `tools/`: ignored local clones of external benchmark repos
- `adapters/`: tracked Cortex adapters and harness glue
- `configs/`: tracked benchmark configs and run manifests
- `results/`: tracked summary outputs we want to keep in git
- `runs/`: ignored raw run logs, temp files, datasets, and scratch outputs

## Current External Suites

- `agent-memory-benchmark`: primary harness for baseline adapter work
- `LongMemEval`: long-context memory evaluation
- `locomo`: long conversation memory benchmark
- `MemoryAgentBench`: agent-memory benchmark suite

## Deferred / unresolved

- `MemBench` does not need a separate clone right now because AMB already has first-class support for it.
- `DMR` is still unresolved as a standalone canonical repo.
- Until we confirm a better upstream source, treat DMR coverage as something we may implement through the AMB adapter layer instead of cloning an unknown repo.

## Setup

From repo root:

```powershell
powershell -ExecutionPolicy Bypass -File benchmarking\setup-benchmarks.ps1
```

That will clone or update the pinned suites into `benchmarking/tools/` without polluting git history, because `benchmarking/tools/` is ignored.

## Cortex Adapter

The first tracked adapter lives in `benchmarking/adapters/` and is driven by:

- `python benchmarking\run_amb_cortex.py smoke`
- `python benchmarking\run_amb_cortex.py run --dataset longmemeval --split s --query-limit 20`

Important constraints:

- The runner uses an isolated benchmark daemon by default.
- It does not point AMB at the live app daemon, so benchmark data never mixes with real user memory.
- Every run writes `run-manifest.json` into its run directory with the Cortex git commit, benchmark tool commits, dataset/mode settings, and whether oracle mode was used.
- Every scored run also records the answer/judge provider selected from `OMB_ANSWER_LLM` / `OMB_JUDGE_LLM`, or auto-detected from available API keys.
- `--oracle` is allowed only as a diagnostic ceiling. It should not be treated as a headline score.
- A real scored run still needs one configured model provider key: `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `OPENAI_API_KEY`, or `GROQ_API_KEY`.
