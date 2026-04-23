<p align="center"><a href="../README.md">← Back to README</a></p>

# Roadmap

> What shipped, what's next, and what's further out. Enough detail to start contributing, without internal planning artifacts.

---

## v0.5.0 — Stabilization &nbsp; `shipped`

> Reliable, one-daemon, local-first release. Benchmark-honest and adapter-consistent.

- One-daemon lifecycle hardening and spawn-path guardrails
- Cross-surface adapter conformance + contract test coverage
- OpenAPI / version sweep + clippy / test release gates
- Chrome extension local-first MV3 companion + CI policy guardrails
- Retrieval: RRF, crystal family recall, synonym parity
- Control Center: analytics, agents, Monte Carlo projections
- Agent telemetry, conflict detection, client permissions
- Public docs, CHANGELOG, security policy, and release verification

---

## v0.6.0 — Foundation Hardening &nbsp; `next`

> Make Cortex robust for daily development workflows.

| Theme | Details |
|-------|---------|
| **Lifecycle control** | TTL / hard expiration for short-lived facts. Session rollback for bad agent runs. |
| **Schema discipline** | Versioned migrations with explicit upgrade checks. Doctor-style validation command. |
| **Derived-state repair** | Rebuild commands for indexes, embeddings, and crystallized state. |
| **Memory quality** | Semantic dedup on store path. Boot prompt audit trail. |

<details>
<summary>Contributor-ready tasks</summary>

- Add migration tests for new metadata columns
- Improve CLI UX around repair/reindex status output
- Add failure-mode tests for rollback and dedup edge cases

</details>

---

## v0.7.0 — Governance & Economics &nbsp; `planned`

> Make team deployments predictable, auditable, and budget-aware.

| Theme | Details |
|-------|---------|
| **Budget governance** | Per-endpoint limits (recall depth, boot budget, invocation rates) |
| **Retention classes** | Durable knowledge vs operational context vs ephemera |
| **Human review** | Queue/review flow for promoting shared knowledge |
| **Context quality** | Dynamic ranking — high-value memories injected first |
| **Adapter conformance** | Shared contract tests across MCP + HTTP + SDKs |

<details>
<summary>Contributor-ready tasks</summary>

- Contract tests for tool parity across transports
- Config schema improvements for retention/budget policies
- Dashboard UX for review queues and budget visibility

</details>

---

## v0.8.0 — Multi-Tenant Hardening &nbsp; `planned`

> Secure, fair, and operable team mode at larger scale.

| Theme | Details |
|-------|---------|
| **Privacy** | Deep erasure across core rows + derived indices. Crystal lineage. |
| **Auth hardening** | Capability-scoped identity model for agent calls |
| **Fairness** | Per-user quotas, admission control, backup/restore workflows |
| **Isolation** | Namespace / team-aware embedding boundaries |

<details>
<summary>Contributor-ready tasks</summary>

- Visibility/isolation integration tests
- Backup and restore dry-run tooling
- Observability improvements for auth/quotas

</details>

---

## v0.9.0 — Advanced Agent Support &nbsp; `planned`

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
