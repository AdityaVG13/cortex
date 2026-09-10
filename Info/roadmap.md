<p align="center"><a href="../README.md">← Back to README</a></p>

# Roadmap

What shipped and what is next.

Current release: **v0.6.0**. Source on `main` also includes the Clock-Quorum Recall cutover (model-free recall, `crates/models` removed, daemon/logic split). That work is documented in [CHANGELOG.md](../CHANGELOG.md) under Unreleased until the next tagged release.

---

## V5 native cycle and remaining release gates

Unreleased source now implements exact registered intake, resumable file inventory,
version-pinned host fixture normalization, scoped reverse any-cue needs, context-bound
preparation, and opt-in lineage-deduplicated associations with reversible named feedback.
CLI and optional plugin-sidecar entry points are documented in README. Existing CQR
semantic APIs stay separate from raw attributed observations.

Verification: 27 targeted Rust contracts and three plugin bridge tests passed; omitted
reverse-index maintenance and weakened independent-support thresholds were detected by
regressions. `tests/examples/observation_bench.rs` measures the native Path-A subset.
On RCH worker `spark-1672` (Linux aarch64, 20 logical CPUs, debug profile), 64-document
fixture measurements were capture p50/p95 5795/8886 us, full indexed API p50/p95
7238/7978 us, and cold open-plus-query p50/p95 33207/52778 us (eight cold samples).
The scan-only reference omits policy/receipt work, so this is not a speedup comparison
or release acceptance. Log: `/tmp/cortex-v5-measurements.log`.
An additional eight-process CLI capture probe retained all eight acknowledgements
in 298900 us wall time including startup; `/tmp/cortex-v5-concurrent-measurements.log`.

The ready literal pull path now reads postings without temporary subscriptions
or a writer reservation. Two failure-first contracts prove zero subscription
churn and successful reads while another connection holds the WAL writer lock;
both reject a deliberately disabled candidate join. After this change, the same
64-document debug workload on `spark-1672` reported full-API p50/p95
1284/1321 us (64 samples), versus the earlier 7238/7978 us observation above.
Eight concurrent CLI captures again retained all eight acknowledgements. These
are observed fixture timings, not a normalized speedup or release acceptance.
Logs: `/tmp/cortex-v5-readonly-tests.log`, `/tmp/cortex-v5-readonly-measurements.log`.

An isolated installed **Claude Code 2.1.260** probe now covers native
UserPromptSubmit, Bash PostToolUse and Stop emission plus acceptance of
`additionalContext`. External networking and writes to the user's home were
sandbox-denied; a deterministic loopback Messages stub drove the tool/final
sequence. Its token/cost counters are synthetic, not reader-quality evidence.
Artifacts: `/tmp/cortex-v5-host-KgzWM2/events.jsonl` and
`/tmp/cortex-v5-host-KgzWM2/loopback-bAZzvQ/loopback-result.json`.

The observed shapes exposed and fixed three concrete gaps: prompt IDs differ
from transcript UUIDs, Bash responses are structured objects, and non-evidence
control records otherwise stall raw-tail capture. The pinned native adapter now
preserves prompt/tool identity, full reported Bash fields and transactional
metadata markers. Opt-in native user/Bash hook configuration derives invocation
metadata without model memory commands. Final live capture still needs its
original message UUID; transcript catch-up remains available. Other tool/private
shapes are not silently accepted. This is protocol-subset validation, not a fully
deployed Cortex/host or real-model task certification.

**Not complete as a V5 release:** broader installed-host capture/delivery validation,
held-out reader/task quality and token savings, power-loss tests, large-scale
contention, and a matched release-performance acceptance band remain
unverified. No host settings or production data were changed to manufacture that
proof; fixtures and debug timings cannot substitute for it.

## Progress on the 2026-09-06 integration state note

| Deliverable | Status on `main` |
|-------------|------------------|
| Transport cutover (plugin MCP/boot off HTTP) | Done — local `cortex mcp` / `hook-boot` stdio |
| Truthful discovery | Done — eight ops; `cortex_boot`→orient; removed names `UNKNOWN_TOOL` |
| Unified V5 access via ops | Done — `observations` on query/orient; `expand` `obs:<id>` |
| Claude coverage matrix | Partial — Stop packaged; Edit/Write legacy ToolResult; Read not native |
| Additional hosts | Not started — MCP interop first; capture only where hooks exist |
| Release evidence | Open — held-out quality, crash/power-loss, release perf |

## Shipped

### v0.5.0 — Stabilization

Reliable, one-daemon, local-first release.

- One-daemon lifecycle and spawn-path guardrails
- Adapter conformance and contract tests
- Control Center analytics, agents, Monte Carlo projections
- Agent telemetry, Jaccard conflict detection, client permissions
- TTL / hard expiration, schema migrations, `cortex doctor`
- Derived-state repair: `reindex`, `rebuild-anchors`, `recrystallize`

v0.5 also shipped a hybrid embedding retriever (MiniLM, RRF, sqlite-vec shadow). That path is **gone** from the live daemon. See Unreleased.

### v0.6.0 — Accessibility, governance, measurement

| Theme | Details |
|-------|---------|
| **Accessibility & Settings** | First-class Settings panel: high contrast, reduced motion, keyboard hints, compact navigation |
| **Budgets** | Local per-endpoint limits; Control Center editor for `budgets.toml` |
| **Retention classes** | Durable / operational / audit / ephemeral |
| **Boot audits** | `GET /boot/audit`, `cortex_boot_audit` |
| **Admin rollback** | `cortex admin rollback --session-id` |
| **Measurement floor** | `cortex-http-pure` adapter and purity gates. No public LongMemEval quality claim |

### Unreleased on `main` — Clock-Quorum Recall

Production recall is CQR only. No local embedding or reranker model.

| Theme | Details |
|-------|---------|
| **Single engine** | `/recall`, `/recall/semantic`, `/peek`, `/as-of`, MCP recall tools all call CQR |
| **Admit rule** | Hard anchor, two clocks, or strong lexical write. Otherwise empty |
| **Model-free home** | Empty install does not create `~/.cortex/models`. `crates/models` deleted |
| **Crate split** | `crates/daemon` + `crates/logic`. Tests in `tests/contracts/` |
| **Vocabulary mismatch** | Morphology, closed developer lexicon, sibling anchors, entity-seeded hops |
| **Honest miss** | Unconstrained paraphrase with no shared handle stays empty |

Details: [ARCHITECTURE.md](../ARCHITECTURE.md).

---

## Shipped vs proposed

Everything under `docs/architecture/next/` is a **proposal path**; a proposal
becomes "shipped" only when a contract under `tests/contracts/` holds it and
`ARCHITECTURE.md` lists the surface. Three guides describe the shipped
system: `docs/guides/user-guide.md`, `docs/guides/developer-guide.md`,
`docs/guides/operations-guide.md`.

## Next (v0.7 direction)

Privacy, fairness, and team-mode hardening.

| Theme | Details |
|-------|---------|
| **Privacy** | Deep erasure across core rows and derived indices |
| **Auth** | Capability-scoped identity for agent calls |
| **Fairness** | Per-user quotas, backup / restore workflows |
| **Isolation** | Namespace / team-aware recall (ACL already exists; this is the remaining edge) |

Contributor-sized slices: visibility/isolation contracts, backup dry-run, auth/quota observability.

Query expansion (alias / path / task-context) that used to sit here **already shipped** in Unreleased CQR.

---

## Cross-cutting backlog (anytime)

- Key rotation and operational key hygiene
- Optional at-rest encryption path
- Documentation and onboarding UX
- Accessibility evidence (screen-reader walkthroughs) before any conformance claim
- Funded LongMemEval run before any public quality-gain claim

---

## Contributing

1. Prefer a shipped-surface bug or a v0.7 isolation/backup slice.
2. Do not open PRs that reinstall embeddings, ONNX, or an LLM on the hot path.

See [CONTRIBUTING.md](../CONTRIBUTING.md).
