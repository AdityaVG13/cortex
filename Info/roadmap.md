<p align="center"><a href="../README.md">← Back to README</a></p>

# Roadmap

What shipped, what is next, and what is still an open product decision.

Current release: **v0.6.0**. Source on `main` also includes the Clock-Quorum Recall cutover (model-free recall, `crates/models` removed, daemon/logic split). That work is documented in [CHANGELOG.md](../CHANGELOG.md) under Unreleased until the next tagged release.

This page is **not** a committed schedule. Items under “Open product decisions” need a design choice before implementation.

---

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

Details: [ARCHITECTURE.md](../ARCHITECTURE.md), [research.md](research.md).

---

## Next (v0.7 direction)

Privacy, fairness, and team-mode hardening. These are the least controversial follow-ons because they extend existing surfaces.

| Theme | Details |
|-------|---------|
| **Privacy** | Deep erasure across core rows and derived indices |
| **Auth** | Capability-scoped identity for agent calls |
| **Fairness** | Per-user quotas, backup / restore workflows |
| **Isolation** | Namespace / team-aware recall (ACL already exists; this is the remaining edge) |

Contributor-sized slices: visibility/isolation contracts, backup dry-run, auth/quota observability.

Query expansion (alias / path / task-context) that used to sit here **already shipped** in Unreleased CQR.

---

## Open product decisions

These used to be listed as v0.8 / v1.0 as if they were scheduled. They are **not** committed. They need a product choice. Comparison and paper notes: [memory-landscape.md](memory-landscape.md).

| Decision | Why it is open | What “done” would look like |
|----------|----------------|-----------------------------|
| **Verified working board** | Recuris shows WM-only beats skill libraries. Cortex boot still snapshots tasks; nothing commits `done` from a receipt | A `board` target in `/boot` and `/pack`; `done` only from a tool/test/user receipt |
| **Pack as the default loop** | Boot exists; agents still start with `/recall`. `ee pack` / OptMem `wake` are the better product shape | `/pack` (or boot-by-default) with a deterministic hash |
| **Offline distill** | Traces stay chatty. Mem0 extracts; Cortex refuses to summarize on the hot path — and then never summarizes offline either | Steward pass proposes typed heads (`rule`, `scar`, `anti-pattern`); promotion is audited |
| **Event-shaped recall** | Recall is `q=`. Edit/`src/auth.rs` should not parse “how does auth work” | `event=edit\|test\|boot` + path/board/scars |
| **Outcome-gated admission** | `used_with` exists; harmful feedback cannot strip `strong_lexical` | Harm cannot admit alone; helpful cannot resurrect superseded |
| **External ingest** | ChatGPT / Claude / Gemini import is a product, not a retrieval problem | Read-only parsers, dry-run, dedup against traces |
| **Branch-aware relevance** | Useful for coding agents; not designed | Memory scoped to git branch without breaking as-of |

Until those are chosen, do not treat them as issue fodder for drive-by PRs.

---

## Cross-cutting backlog (anytime)

- Key rotation and operational key hygiene
- Optional at-rest encryption path
- Documentation and onboarding UX
- Accessibility evidence (screen-reader walkthroughs) before any conformance claim
- Funded LongMemEval run before any public quality-gain claim — and expect CQR to lose unconstrained paraphrase vs embedding systems

---

## Contributing

1. Prefer a shipped-surface bug or a v0.7 isolation/backup slice.
2. Do not open PRs that reinstall embeddings, ONNX, or an LLM on the hot path.
3. If you want an open product decision, write a short design note first.

See [CONTRIBUTING.md](../CONTRIBUTING.md).
