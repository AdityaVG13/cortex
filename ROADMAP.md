# Cortex Roadmap

What's coming next. Features organized by release milestone.

---

## v0.4.0 -- Public Launch (current)

Core daemon is stable. Security hardened. Ready for contributors.

- [ ] Version bump + GitHub release with installer
- [ ] README rewrite for external developers
- [ ] CONTRIBUTING.md + SECURITY.md
- [ ] Architecture docs audit (public vs internal)
- [ ] Recall quality baseline analysis
- [ ] Auto-generated CHANGELOG on version tags
- [ ] Desktop app (Control Center) end-to-end verification

---

## v0.5.0 -- Foundation Hardening

Schema discipline, data repair, and memory quality.

- [ ] **TTL / hard expiration** -- `expires_at` column for temporal facts that shouldn't decay slowly
- [ ] **Session-based rollback** -- archive all data from a faulty agent run in one command
- [ ] **Schema versioning** -- named migration epochs, `cortex doctor` verification command
- [ ] **Derived-state repair** -- `cortex reindex`, `cortex re-embed`, `cortex recrystallize` for when indexes drift
- [ ] **Semantic dedup at store time** -- distinguish "new fact" from "reinforcement of existing fact"
- [ ] **Boot prompt audit trail** -- record which sources were included in each compiled boot prompt

---

## v0.6.0 -- Governance & Economics

Budget controls, retention policies, and human review surfaces.

- [ ] **Budget governance** -- per-endpoint token limits, max recalls per turn, invocation frequency caps
- [ ] **Retention policy classes** -- durable knowledge vs operational context vs audit records vs ephemera
- [ ] **Human review workflows** -- inboxes for shared knowledge, review queues, promotion paths (private -> team -> shared)
- [ ] **Dynamic context ranking** -- rank injected memories by relevance, inject top 3-5 items only
- [ ] **Contradiction triage worker** -- background resolution tasks for disputed facts
- [ ] **Adapter conformance spec** -- canonical behavior tests for MCP, HTTP, Python SDK, TypeScript SDK

---

## v0.7.0 -- Multi-Tenant Hardening

Privacy, fairness, and agent identity for team deployments.

- [ ] **Deep erasure** (`DELETE /forget`) -- scrub row + FTS + embedding + re-crystallize affected crystals
- [ ] **Crystal lineage tracking** -- trace which memories built each crystal, re-crystallize on source deletion
- [ ] **Capability-scoped agent tokens** -- cryptographic agent identity with restricted write authority
- [ ] **Multi-tenant fairness** -- per-user quotas for store/recall/embedding throughput
- [ ] **Backup, restore, disaster recovery** -- source-of-truth backup, derived rebuild, encrypted key handling
- [ ] **Namespace-isolated embedding spaces** -- separate vector indexes per team for enterprise security

---

## v0.8.0 -- Advanced Agent Support

Branch awareness, provenance, and autonomous agent coordination.

- [ ] **Branch-aware filtering** -- prioritize memories from current git branch and ancestors
- [ ] **Reasoning provenance** -- every recalled memory includes its source (commit, session, parent decision)
- [ ] **Multi-agent deadlock detection** -- dependency graph on tasks with cycle detection
- [ ] **Chrome extension** -- inject Cortex context into claude.ai, chatgpt.com, gemini.google.com

---

## v1.0.0 -- AI Information Ingester

Import, classify, and index your data from any AI platform.

- [ ] ChatGPT export parser (conversations.json)
- [ ] Claude conversation ingester
- [ ] Gemini conversation ingester
- [ ] Intelligent separator (topic detection + classification into decisions/facts/preferences)
- [ ] Dedup against existing Cortex memories
- [ ] Confidence scoring for imported data
- [ ] Bulk import CLI (`cortex ingest <export.json>`)

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and PR guidelines. Pick any unchecked item, open an issue or PR to claim it.
