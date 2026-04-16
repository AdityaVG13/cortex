# Cortex Roadmap

This roadmap is public and contributor-focused: enough detail to start building, without exposing internal planning artifacts.

Release-status source of truth lives in `docs/internal/CORTEX-UNIFIED-STATUS-PLAN.md`.

---

## Current Release Track -- v0.5.0 Stabilization Closeout

Focus: ship a reliable, one-daemon, local-first release that is benchmark-honest and adapter-consistent.

### Shipped in the current track
- One-daemon lifecycle hardening and spawn-path guardrails
- Cross-surface adapter conformance + contract test coverage
- OpenAPI/version sweep + clippy/test release gates
- Chrome extension local-first MV3 companion + CI policy guardrails

### Remaining closeout items
- Startup matrix + troubleshooting refresh across public docs
- Team-mode security wording alignment for non-loopback deployment guidance
- Final release-facing roadmap/docs sync before v0.5.0 close

---

## v0.5.x -- Foundation Hardening (continued)

Goal: make Cortex robust for daily development workflows.

### Planned themes
1. **Lifecycle control**
   - TTL / hard expiration (`expires_at`) for short-lived facts
   - Session rollback primitives for bad agent runs
2. **Schema discipline**
   - Versioned migrations with explicit upgrade checks
   - Doctor-style validation command for schema/runtime consistency
3. **Derived-state repair**
   - Rebuild commands for indexes, embeddings, and crystallized state
4. **Memory quality**
   - Semantic dedup on store path
   - Boot prompt audit trail (which sources were used, and why)

### Contributor-ready tasks
- Add migration tests for new metadata columns
- Improve CLI UX around repair/reindex status output
- Add failure-mode tests for rollback and dedup edge cases

---

## v0.6.0 -- Governance & Economics

Goal: make team deployments predictable, auditable, and budget-aware.

### Planned themes
1. **Budget governance**
   - Per-endpoint limits (recall depth, boot budget, invocation rates)
2. **Retention classes**
   - Distinguish durable knowledge vs operational context vs ephemera
3. **Human review surfaces**
   - Queue/review flow for promoting shared knowledge
4. **Context quality**
   - Dynamic ranking so high-value memories are injected first
5. **Adapter conformance**
   - Shared contract tests across MCP + HTTP + SDKs

### Contributor-ready tasks
- Contract tests for tool parity across transports
- Config schema improvements for retention/budget policies
- Dashboard UX for review queues and budget visibility

---

## v0.7.0 -- Multi-Tenant Hardening

Goal: secure, fair, and operable team mode at larger scale.

### Planned themes
1. **Privacy and data control**
   - Deep erasure across core rows + derived indices
   - Crystal lineage for traceability and safe re-crystallization
2. **Auth hardening**
   - Stronger capability-scoped identity model for agent calls
3. **Fairness and resiliency**
   - Per-user quotas and admission control
   - Backup/restore and disaster-recovery workflows
4. **Isolation**
   - Namespace/team-aware embedding boundaries

### Contributor-ready tasks
- Visibility/isolation integration tests
- Backup and restore dry-run tooling
- Observability improvements for auth/quotas

---

## v0.8.0 -- Advanced Agent Support

Goal: improve multi-agent coordination and provenance.

### Planned themes
1. **Branch-aware memory relevance**
2. **Reasoning provenance and traceability**
3. **Deadlock-safe multi-agent task orchestration**
4. **Control Center task dispatch and live progress**

### Contributor-ready tasks
- Task graph UI/UX improvements
- Provenance metadata surfacing in recall responses
- Lock contention and deadlock simulation tests

---

## v1.0.0 -- AI Information Ingester

Goal: import and normalize knowledge from major AI platforms.

### Planned themes
1. **Export parsers**
   - ChatGPT, Claude, Gemini conversation ingestion
2. **Normalization pipeline**
   - Classify imported content into durable memory types
3. **Quality controls**
   - Dedup against existing memories
   - Confidence scoring for imported entries
4. **Operator tooling**
   - Bulk ingest CLI with preview + dry-run

### Contributor-ready tasks
- Parser fixtures and golden tests
- Classification quality benchmarks
- CLI progress/error reporting improvements

---

## Public Backlog (cross-milestone)

These are open contribution areas that may land in any release based on quality and urgency:

- Key rotation improvements and operational key hygiene workflows
- Optional at-rest encryption integration path
- Expanded adapter compatibility (OpenAI-style function interfaces)
- Additional diagnostics and memory quality metrics
- Documentation and onboarding UX improvements

---

## How to Contribute

1. Pick a roadmap item and open/claim an issue.
2. Propose a small implementation slice with clear acceptance criteria.
3. Link tests or verification output in your PR.

See [CONTRIBUTING.md](../CONTRIBUTING.md) for setup, checks, and PR expectations.
