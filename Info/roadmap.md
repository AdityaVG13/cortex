<p align="center"><a href="../README.md">← Back to README</a></p>

# Roadmap

What shipped and what is next.

Current release: **v0.6.0**. Source on `main` also includes the Clock-Quorum Recall cutover (model-free recall, `crates/models` removed, daemon/logic split). That work is documented in [CHANGELOG.md](../CHANGELOG.md) under Unreleased until the next tagged release.

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

Details: [ARCHITECTURE.md](../ARCHITECTURE.md).

---

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
