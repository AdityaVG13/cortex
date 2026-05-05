<p align="center"><a href="../README.md">← Back to README</a></p>

# Roadmap

> What shipped, what's next, and what's further out. Enough detail to start contributing, without internal planning artifacts.

---

## v0.5.0 — Stabilization &nbsp; `shipped`

> Reliable, one-daemon, local-first release. Benchmark-honest and adapter-consistent.

- One-daemon lifecycle hardening and spawn-path guardrails
- Cross-surface adapter conformance + contract test coverage
- OpenAPI / version sweep + clippy / test release gates
- Retrieval: RRF, crystal family recall, synonym parity
- Control Center: analytics, agents, Monte Carlo projections
- Agent telemetry, conflict detection, client permissions
- Public docs, CHANGELOG, security policy, and release verification
- TTL / hard expiration for temporal facts
- Schema migration framework with `cortex doctor` validation
- Derived-state repair CLIs: `reindex`, `re-embed`, `recrystallize`
- Semantic dedup on store path
- Recall feedback loop — agent usage tunes future ranking
- Embedding profile selection (`all-MiniLM-L6-v2` default, `L12-v2` opt-in)

---

## v0.6.0 — Accessibility, Governance & Recall Quality &nbsp; `shipped`

> Makes Cortex more usable day-to-day, more manageable at team scale, and more disciplined about recall measurement.

| Theme | Details |
|-------|---------|
| **Accessibility & Settings** | First-class Settings panel with accessibility preferences for high contrast, reduced motion, keyboard hints, and compact navigation. Includes stronger focus states, semantics, live-region handling, contrast checks, and narrow reflow gates. |
| **Motion system** | Unified sidebar, panel, tab, and numeric transitions with shared motion tokens and reduced-motion bypasses. |
| **Recall quality** | Phase 0 purity (`cortex-http-pure` adapter, 5 CI gates, CAS-100 + triangle judge). Phase 1 embedding upgrade (`bge-base-en-v1.5` default). Phase 2 cross-encoder reranker (`ms-marco-MiniLM-L-6-v2` int8) is shipped default-off behind shadow/primary gates while public benchmark claims remain gated on LongMemEval/API-backed evidence. |
| **Budget governance** | Local per-endpoint limits for store, recall-family, boot, and MCP calls, plus Control Center budget status and a Tauri-only local budget editor. |
| **Retention classes** | Durable knowledge vs operational context vs audit vs ephemera. Prereq for budget governance. |
| **Context ranking** | Dynamic ranking in injectors — top-N by activeness × relevance, not fixed set. |
| **Adapter conformance** | Shared contract tests across HTTP, MCP-RPC, Python SDK, and TypeScript SDK surfaces. |
| **Foundation carryovers** | Session rollback CLI (`cortex admin rollback`). Boot prompt audit trail. Score-adaptive truncation for boot. `DEFAULT_CORTEX_PORT` consolidation. |

<details>
<summary>Release follow-ups</summary>

- Capture manual screen-reader walkthrough evidence for NVDA+Firefox, VoiceOver+Safari, and Narrator+Edge before making formal accessibility conformance claims.
- Capture browser-harness-based automated accessibility evidence for main flows.
- Commit LongMemEval/API-backed recall benchmark artifacts before making public quality-gain claims.
- Refresh public screenshots and release artifacts for the final v0.6.0 cut.

</details>

---

## v0.7.0 — Multi-Tenant Hardening &nbsp; `next`

> Privacy, fairness, and auth for team deployments.

| Theme | Details |
|-------|---------|
| **Privacy** | Deep erasure across core rows + derived indices. Crystal lineage tracking. |
| **Auth hardening** | Capability-scoped identity model for agent calls (IBCTs). |
| **Fairness** | Per-user quotas, admission control, backup/restore workflows. |
| **Isolation** | Namespace / team-aware embedding boundaries. |
| **Query expansion** | HyDE-style query rewriting after the v0.6.0 default-off reranker path has benchmark evidence. |
| **External memory bridges** | First read-only bridge (ChatGPT import) against the v0.6.0 acceptance gate spec. |

<details>
<summary>Contributor-ready tasks</summary>

- Visibility/isolation integration tests
- Backup and restore dry-run tooling
- Observability improvements for auth/quotas

</details>

---

## v0.8.0 — Advanced Agent Support &nbsp; `planned`

> Improve multi-agent coordination and provenance.

| Theme | Details |
|-------|---------|
| **Branch-aware relevance** | Memory relevance tied to active branch context |
| **Reasoning provenance** | Traceability from recall result back to original source |
| **Multi-agent orchestration** | Deadlock-safe task coordination |
| **Control Center dispatch** | Task dispatch and live progress from the dashboard |

---

## v1.0.0 — AI Information Ingester &nbsp; `future`

> Import and normalize knowledge from major AI platforms.

| Theme | Details |
|-------|---------|
| **Export parsers** | ChatGPT, Claude, Gemini conversation ingestion |
| **Normalization** | Classify imported content into durable memory types |
| **Quality controls** | Dedup against existing memories, confidence scoring |
| **Operator tooling** | Bulk ingest CLI with preview + dry-run |

---

## Cross-milestone backlog

These are open contribution areas that may land in any release:

- Key rotation and operational key hygiene workflows
- Optional at-rest encryption integration path
- Expanded adapter compatibility (OpenAI-style function interfaces)
- Additional diagnostics and memory quality metrics
- Documentation and onboarding UX improvements

---

## Contributing

1. Pick a roadmap item and open/claim an issue.
2. Propose a small implementation slice with clear acceptance criteria.
3. Link tests or verification output in your PR.

See **[CONTRIBUTING.md](../CONTRIBUTING.md)** for setup, checks, and PR expectations.
