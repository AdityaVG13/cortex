# Benchmarking Workspace

This folder is the staging area for external benchmark harnesses, Cortex adapters, and run outputs.

Layout:

- `tools/` stores cloned third-party benchmark repos. It is ignored by git so we can refresh upstream copies without polluting the repo history.
- `runs/` stores local benchmark outputs and scratch artifacts. It is also ignored by git.
- `setup-benchmarks.ps1` clones or updates the benchmark harnesses we want to evaluate Cortex against before we start recording formal results.

Current benchmark tool plan:

- `vectorize-io/agent-memory-benchmark`
  - Primary evaluation harness for LongMemEval-style and decision-memory runs.
- `snap-research/locomo`
  - Official LoCoMo benchmark/dataset repository.

Next expected additions:

- `adapters/`
  - Cortex-specific benchmark adapters and runner glue.
- `manifests/`
  - Frozen benchmark versions, run metadata, and environment notes for reproducibility.

Usage:

```powershell
cd C:\Users\aditya\cortex\benchmarking
.\setup-benchmarks.ps1
```

After setup completes, we can add the Cortex adapter and run the benchmark plan from `docs/internal/CORTEX-EVOLUTION-PLAN.md`.
