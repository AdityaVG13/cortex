# Benchmarking Workspace

This directory is the reproducible home for external benchmark harnesses, Cortex adapters, configs, and saved benchmark outputs.

It intentionally separates:

- `benchmark/`: existing in-repo Cortex recall/metric scripts
- `benchmarking/`: external benchmark suites plus the glue needed to run Cortex against them

## Benchmark modes (v0.6.0+)

Cortex ships three measurement adapters. Pick based on what you want to measure.

| Mode | Adapter | When to use | What it measures |
|------|---------|-------------|------------------|
| **Pure** | `cortex-http-pure` | **Default. Every new quality claim.** | Core daemon recall quality only. Zero helpers. Zero pre-/post-processing. Single `/recall` call per query. |
| Base (deprecated) | `cortex-http-base` | Historical comparison only. | Partial helpers. Retained for pre-v0.6.0 continuity. Do not use for new claims. |
| Tuned | `cortex-http` | Regression testing only. | Full helper stack. Not a core-quality claim. Shows what's possible with adapter-layer tuning; not representative of the daemon itself. |

### Run a triad

```bash
bash scripts/benchmark-triad.sh longmemeval-s
```

Writes three JSON result files to `benchmarking/results/`. Pure is the canonical measurement.

### Purity pledge

Cortex commits to:

1. **Local-first** -- measurements run against a local daemon; no cloud fallbacks.
2. **No oracle leakage** -- queries never carry metadata the daemon would not see in production.
3. **No scoring without credentials** -- scored runs require explicit answerer/judge API keys.
4. **No helper inflation** -- every public recall-quality score claim comes from `cortex-http-pure`.

Five CI gates enforce these in `scripts/purity-gates/`. `CODEOWNERS` protects the canonical adapter, `CHANGELOG.md`, and the gate scripts themselves from silent drift.

### Adversarial suite + triangle judge

- `benchmarking/adversarial/cas-100.jsonl` -- 100-item Cortex Adversarial Suite (15 categories). Wilson 95% CI half-width ±7pp at N=100.
- `benchmarking/judges/triangle.py` -- three-judge cross-family protocol (GPT-4o + Claude + local Qwen3-30B via Ollama). Pairwise Cohen's κ + Fleiss' κ. `--answerer-family` overlap refuses to run.

See `benchmarking/adversarial/cas-100.spec.md` for suite format + usage.

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
- Retrieval budget defaults to `300` tokens per query (`--recall-budget`), with strict quality gating enabled by default.
- Each run emits `retrieval-metrics.jsonl` and `gate-report.json` under the run directory for auditability.
- The default gate mode is `dynamic` (`--token-gate-mode dynamic`), which applies provider-aware token limits (`--provider-profile auto|claude|openai|codex|gemini|groq|default`).
- Saved baseline gates are loaded from `benchmarking/configs/token-gate-baselines.json` by default:
  - keyed by provider profile + dataset/split/mode/category scenario
  - enforced as non-regression floors/ceilings on top of dynamic profile limits
  - ignored only when `--disable-baseline-gates` is set (diagnostics only)
- The quality gate fails the run if:
  - accuracy is below `--min-accuracy` (default `0.90`)
  - token gates fail (dynamic provider profile limits by default, or fixed limits in `absolute` mode)
  - recall token telemetry is missing (unless `--allow-missing-recall-metrics` is explicitly set)
- Use `--token-gate-mode absolute` for fixed limit enforcement (`--max-recall-tokens`, `--max-avg-recall-tokens`).
- Use `--token-gate-mode off` only for diagnostics when you explicitly want quality-only gating.
- Baselines auto-tighten after passing runs (`--no-auto-tighten-baseline` to disable), but only when:
  - enforcement is enabled
  - token gating is on
  - query volume meets `--min-queries-for-baseline-update` (default `20`)
  - run is not scoped to `--query-limit` / `--query-id`
- `--no-enforce-gate` is diagnostics-only and must not be used for headline benchmark claims.
