# Cortex v3 Roadmap — The Intelligent Brain

**Source:** 8 Gemini deep research documents (224KB), 100+ academic/production sources, state.md pending items, live DB corruption analysis.
**Created:** 2026-03-30
**Approach:** Milestone-based. Each milestone ships something usable. Priority: data integrity → intelligence → coordination → performance.

---

## Milestone 1: Brain Health & Memory Quality
*Priority: URGENT — today we lost 15 decisions to DB corruption*
*Effort: 1 week | Dependencies: None*

### Why First
The Rust daemon write-loss bug caused real data loss today. Before adding features, the brain must not lose memories. This milestone also adds the lowest-effort upgrades from the research synthesis.

### Deliverables

| # | Feature | Source | Effort | Description |
|---|---------|--------|--------|-------------|
| 1.1 | **DB integrity checks** | Today's incident | Low | PRAGMA integrity_check on daemon startup. Alert + auto-backup if corrupt. WAL mode + `PRAGMA synchronous = NORMAL`. |
| 1.2 | **Rust write-loss fix** | Known issue | Medium | Diagnose why Rust daemon loses writes on shutdown. Likely missing flush/sync before process exit. |
| 1.3 | **Ebbinghaus decay scoring** | Gemini Doc 3 | Low | `score = importance × e^(-0.03 × days_since_access)`. SQLite-native. Tiered decay rates by type: preferences (slow), decisions (medium), sessions (fast), traces (immediate). |
| 1.4 | **Semantic dedup on writes** | Gemini Docs 1,3 | Medium | On every `cortex_store()`: embed + cosine similarity check. If sim > 0.92, UPDATE existing instead of INSERT. Prevents memory spam. |
| 1.5 | **Memory type system** | Gemini Doc 3 | Low | Enforce types: `preference`, `decision`, `observation`, `rule`, `trace`. Different decay rates per type. |
| 1.6 | **Temporal validity** | Gemini Docs 1,3 | Low | Triple-date metadata: `created_at`, `last_accessed`, `relative_date`. Enables time-aware retrieval. |

### Success Criteria
- `PRAGMA integrity_check` returns `ok` on every boot
- Zero data loss over 7 days of normal use
- Duplicate stores (same content) produce UPDATE not INSERT
- Decay scoring visible in recall results

---

## Milestone 2: Ambient Intelligence
*Priority: HIGH — biggest capability jump*
*Effort: 2 weeks | Dependencies: M1 (dedup prevents observation spam)*

### Why Second
Right now, agents only store what they're explicitly told to store. Ambient capture means the brain learns automatically from every tool call, every decision, every failure — without interrupting the agent's work.

### Deliverables

| # | Feature | Source | Effort | Description |
|---|---------|--------|--------|-------------|
| 2.1 | **PostToolUse Observer hook** | Gemini Doc 1 | Medium | Fire-and-forget async hook. Intercepts tool outputs, streams to Cortex via HTTP. Non-blocking — never delays the agent. |
| 2.2 | **Qwen-1.5B fact extraction** | Gemini Docs 1,7 | Medium | Local model (Qwen-2.5-Coder-1.5B via Ollama) extracts structured facts from raw tool output. 100+ tok/sec on CPU. |
| 2.3 | **Tiered compression** | Gemini Docs 1,3 | High | Raw observations → Tier 2 episodic summaries (30-40k tokens) → Tier 3 semantic reflections (100k+ compression). Observer does T1→T2, Phi-4 does T2→T3. |
| 2.4 | **NLI confidence gating** | Gemini Doc 1 | Medium | DeBERTa-v3 or local model cross-checks new observations against existing memories. Rejects contradictions, flags conflicts. |
| 2.5 | **Failure-to-rule promotion** | Gemini Doc 1 | Medium | 3+ episodic failures with same pattern → auto-promote to canonical rule injected into boot prompt. The brain learns from mistakes. |
| 2.6 | **cortex-dream --execute** | Pending | Medium | Run the dream compaction pipeline. Consolidates raw observations into compressed knowledge during idle time. |
| 2.7 | **Focus pruning tools** | Gemini Doc 7 | Low | `start_focus` (checkpoint) and `complete_focus` (summarize & delete raw logs). Agents manage their own context window. 22.7% token reduction. |

### Success Criteria
- Tool outputs automatically captured without agent code changes
- Compression ratio ≥ 5x on raw observations
- False positive rate < 5% on contradiction detection
- At least 3 auto-promoted rules after 1 week of use

---

## Milestone 3: Multi-Agent Coordination
*Priority: HIGH — enables Claude + Droid + Gemini ecosystem*
*Effort: 2 weeks | Dependencies: M1 (integrity), M2 (ambient capture feeds coordination)*

### Deliverables

| # | Feature | Source | Effort | Description |
|---|---------|--------|--------|-------------|
| 3.1 | **Event sourcing** | Gemini Doc 2 | Medium | Append-only event log of all actions/observations. Enables deterministic replay, auditing, human-in-the-loop review. |
| 3.2 | **Semantic Rebase protocol** | Gemini Docs 2,4 | High | State = Base + Deltas. On conflict: archive rejected delta, fetch latest, trigger re-inference. Solves 36.9% of multi-agent failures. |
| 3.3 | **Git-branch awareness** | Gemini Doc 4 | Medium | Metadata check of `HEAD` on every query. Prevents cross-branch context drift (stale decisions from other branches). |
| 3.4 | **SSE → Tauri dashboard** | Pending | Medium | Wire existing SSE event stream into Tauri desktop app. Real-time visibility into multi-agent activity. |
| 3.5 | **TODO-Claim CRDT** | Gemini Docs 4,5 | Medium | Atomic task assignment via CRDTs. Prevents two agents from claiming the same task. |
| 3.6 | **MCP federation** | Gemini Doc 2 | Low | Expose Cortex as discoverable MCP server. Any agent can find and use Cortex services via standard protocol. |
| 3.7 | **Provenance citations** | Gemini Docs 1,4 | Medium | Every fact mapped to source (COMMIT_HASH, UTTERANCE_ID). Agents verify citations against current codebase before acting. |

### Success Criteria
- Two agents can work on same codebase without conflicting decisions
- Event log enables full replay of any agent session
- Branch-specific context: switching branches loads correct memory context
- Dashboard shows live multi-agent activity

---

## Milestone 4: Performance & Security (Rust Layer)
*Priority: MEDIUM — scale everything above*
*Effort: 3-4 weeks | Dependencies: M1 (fix write-loss first), M3 (protocols to implement in Rust)*
*Prerequisite: Install Rust + MSVC build tools*

### Deliverables

| # | Feature | Source | Effort | Description |
|---|---------|--------|--------|-------------|
| 4.1 | **Single-writer SQLite thread** | Gemini Docs 5,6 | High | Dedicated writer with mpsc channel. Eliminates SQLITE_BUSY errors. Target: 60k+ writes/sec. |
| 4.2 | **SimSIMD vector kernels** | Gemini Doc 5 | Medium | AVX-512/NEON cosine similarity. Sub-millisecond retrieval over 100k+ nodes. 10x speedup over current. |
| 4.3 | **ORT embedding engine** | Gemini Docs 5,7 | High | Bundled ONNX Runtime with all-MiniLM-L6-v2. Zero external API calls for embeddings. Local-first. |
| 4.4 | **Biscuit auth tokens** | Gemini Docs 4,5 | Medium | Ed25519-signed capability tokens. Datalog policies for scope attenuation. Blocks Confused Deputy attacks. |
| 4.5 | **Zenoh message bus** | Gemini Docs 4,5 | High | Replace SSE with Zenoh (13µs latency, 50Gbps throughput). Rust-native inter-process communication. |
| 4.6 | **DLP secrets scrubber** | Gemini Doc 4 | Medium | Bayesian filter to redact API keys, PII, secrets before they enter persistent memory. |

### Success Criteria
- Zero SQLITE_BUSY errors under concurrent load
- Retrieval latency < 1ms for 100k+ nodes
- Embeddings work offline without Ollama
- No secrets in memory database

---

## Design Principles (updated)

1. **Compound, don't accumulate.** Decay what's unused. Merge what overlaps. Promote what's proven.
2. **Push, don't pull.** Ambient capture eliminates voluntary reporting. Boot warm, not cold.
3. **Universal interface.** HTTP + MCP + A2A. Any AI, any language, any platform.
4. **Reliability over intelligence.** A brain that crashes or loses data is worse than no brain.
5. **Node for the kernel, Rust for the muscle, Python for the cortex.** Different jobs, different tools.
6. **Local-first autonomy.** 1.5B-14B models on-device for routine tasks. Cloud for reasoning.
7. **Verify at retrieval.** Every memory has provenance. Check it's still true before acting.
8. **Time-aware everything.** Semantic × temporal × frequency scoring beats simple recency filters.

---

## Key Metrics

| Metric | Current | M1 Target | M4 Target |
|--------|---------|-----------|-----------|
| Knowledge nodes | 215 | 300+ | 10,000+ |
| Data loss incidents | 1 (today) | 0 | 0 |
| Retrieval latency | ~50ms | ~20ms | < 1ms |
| Compression ratio | 1x (none) | 5x | 30x |
| Auto-captured observations | 0 | 50+/day | 500+/day |
| Token efficiency (boot) | 96% reduction | 97% | 99% |
| Concurrent agents | 2-3 (fragile) | 3-5 (stable) | 10+ |

---

## Research Foundation

All architectural decisions grounded in: `docs/Gemini Deep Research/` (8 documents, 40+ cited sources).
Key references: FadeMem, SimpleMem, TiMem, Focus Architecture, MAST failure audit, Biscuit auth, Zenoh, PROV-DM.
