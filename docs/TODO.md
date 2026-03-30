# Cortex TODO

Single source of truth for what to build, in what order, and who's doing it.
Both AIs read this FIRST before starting work.

**Last updated:** 2026-03-29

---

## Done

- [x] **Phase 0: File Locking & Inter-Agent Comms** — POST /lock, /unlock, GET /locks, POST /activity, /message, boot injection. 17/18 tests passing. (droid + claude)
- [x] **Daemon lifecycle fix** — brain-boot.js is connect-only, cortex-start.bat is the launcher. No more EADDRINUSE races. (claude)
- [x] **Session Bus** — POST /session/start, /heartbeat, /end, GET /sessions. 2min TTL with heartbeat. Boot injection shows active agents. 24/27 tests passing. (claude, 2026-03-29)
- [x] **Dashboard MVP** — Streamlit at localhost:3333. Shows: agent presence, active locks, task board (template), activity feed, memory stats. 5 tabs: Dashboard, Agents & Locks, Activity, Memory Explorer, Actions. (droid, 2026-03-29)
- [x] **Dashboard Task Board + Messages** — Wired Task Board to real /tasks endpoint (pending/claimed/completed). Added Messages tab for inter-agent communication (send/receive). 6 tabs total. (droid, 2026-03-29)
- [x] **Task Board** — POST /tasks, GET /tasks, /tasks/next, /tasks/claim, /tasks/complete, /tasks/abandon. Priority routing, capability filtering, boot injection. 33/39 tests passing. (claude, 2026-03-29)
- [x] **MCP stdio transport fix** — Fixed stdout write function capture for Windows. MCP tools now load correctly: cortex_boot, cortex_recall, cortex_store, cortex_health, cortex_digest, cortex_forget, cortex_resolve. (droid, 2026-03-29)
- [x] **Unified Smart Recall** — Budget-aware recall with token budgeting, cost ladder (headlines → balanced → full), context dedup. Replaces old keyword-only search. (claude, 2026-03-29)
- [x] **Predictive Cache with Co-occurrence Matrix** — Tracks which memories are recalled together, preloads likely-needed context. co_occurrence table + predictive preloading on recall. (claude, 2026-03-29)
- [x] **Token Savings Analytics** — /savings endpoint, boot_savings event tracking, cumulative + daily stats surfaced in health and digest. (claude, 2026-03-29)
- [x] **3D Brain Visualizer** — Interactive Three.js memory graph in Tauri desktop app. Nodes represent memories, edges show co-occurrence relationships. (droid, 2026-03-29)
- [x] **Jarvis-Inspired Desktop UI** — Tauri + React + Vite app with agent dashboard, task board, messaging, memory explorer, analytics. Dev mode working. (droid, 2026-03-29)

## In Progress

- [ ] **Tauri Desktop App** — Native desktop app with Jarvis-inspired UI, 3D brain visualizer, agent dashboard, task board, messaging, memory explorer. Dev mode working (`npm run tauri dev`). Needs production build. **Owner: droid**

## Up Next (ordered)

1. **Port Unified Recall + Co-occurrence + Budget Recall to Rust Daemon** — Node daemon has smart recall with token budgeting and predictive cache via co-occurrence matrix. Rust daemon needs parity. **Owner: droid**
2. **Port /savings and /dump Endpoints to Rust Daemon** — Match latest Node daemon API surface in the Tauri backend. **Owner: droid**
3. **Co-occurrence Matrix v2** — Upgrade from frequency-based to embedding-based prediction. Use vector similarity to predict needed context, not just access patterns. **Owner: claude**
4. **Progressive Memory Aging** — Fresh memories at full fidelity, week-old compressed to key points, month-old reduced to one-liners. Automatic background process. **Owner: claude**
5. **Session-Type Classification on Boot** — Detect session intent (coding, research, review, debug) on first few tool calls via local Ollama, then bias recall toward relevant memories. **Owner: claude + ollama**
6. **Import History** — Ingest ChatGPT, Claude, and Gemini conversation exports. Parse JSON/ZIP exports, extract decisions and learnings, deduplicate against existing brain. **Owner: TBD**
7. **Production Tauri Build** — `cargo tauri build` → .exe installer. Includes: embedded daemon lifecycle, system tray + minimize-to-tray, airgap-ready packaging. **Owner: droid**
8. **Custom App Icon Design** — Replace default Tauri icon with Cortex-branded icon for taskbar/tray/installer. **Owner: TBD**
9. **Rust Daemon Rewrite** — Port daemon.js to Rust. Eliminates Node.js dependency entirely — single binary, zero external deps. Tauri is already Rust so daemon becomes embedded module, not spawned child process. Target: <5MB binary. **Owner: droid, long-term**
10. **SSE Event Feed** — `GET /events/stream` for real-time push. Dashboard and agents subscribe. **Owner: claude (node.js)**
11. **Ambient Capture** — PostToolUse hook auto-captures decisions to inbox table with confidence gating. Promotion pipeline: inbox → episodic → canonical. **Owner: droid (python workers)**
12. **Ollama Sidecar Workers** — Python workers poll /tasks for completed work, run Qwen/DeepSeek review on changed files, store findings. **Owner: droid (python)**

## Stolen Ideas (from competitive research 2026-03-29, see docs/competitive-intel.md)

- [ ] **cortex_peek** — One-line summaries before full recall. Cost ladder: peek → skim → full. Saves tokens. (from cx)
- [ ] **cortex skill** — Command that emits an optimal agent prompt. Self-teaching pattern. (from cx)
- [ ] **Capsule dedup threshold** — Calibrate at 0.92 cosine similarity. Reference from 724-office.
- [ ] **Async capsule compilation** — LLM compression in background threads during idle time. (from 724-office)
- [ ] **Emit helpers** — `cortex.emit_decision()` for zero-friction stores without explicit API calls. (from Agent Lightning)
- [ ] **Memory pressure eviction** — 70% RAM trigger → evict to 56% for in-memory caches. (from Agent Lightning)
- [ ] **Self-check cron** — Scheduled self-audits with LLM-driven remediation. (from 724-office)
- [ ] **Formal JSON-RPC relay** — Tighten agent API surface with typed contracts. (from Parchi)

## Deferred (good ideas, need foundation first)

- **Token Dedup Router** — cross-agent context elimination via bloom filters. Needs session tracking.
- **Decision Provenance DAG** — trace decision lineage across agents/sessions. Needs event sourcing.
- **Self-Tuning Compiler** — track which boot content gets referenced, shift budget. Needs usage data.
- **Dream Consensus Protocol** — multi-model synthesis agreement. Needs cortex-dream working.
- **Event-Sourced Brain** — events table as source of truth, current tables as views. Big refactor.
- **Structured Dissent Protocol** — store debates as first-class objects, not just conclusions.
- **Transcript Tap / Ghost Protocol** — auto-extract decisions from raw transcripts via local LLM.
- **Recall Learns from Downstream Success** — reinforcement learning for recall relevance.

## Cut (not building)

- Cortex Marketplace Protocol — too early, build product first
- Worker Registration Protocol — over-engineered, workers are just scripts calling HTTP
- Kill the Boot Call — writing compiled context into CLAUDE.md creates sync nightmares
- Multi-Tenant / Cortex-as-a-Service — commercial packaging before product-market fit
- Ollama as Orchestration Brain — local models not reliable enough for orchestration
- Cortex Protocol (Open Standard) — premature standardization
- Dependency-Graph Routing — massive scope, unclear payoff vs file locking
- Adversarial Consistency Checker — nice-to-have, not blocking anything

## Pending Maintenance

- [ ] Fix conductor test #6 (`GET /locks`) — ERR_STREAM_WRITE_AFTER_END from daemon cleanup race
- [ ] Fix MaxListenersExceededWarning in test suite — remove signal handlers in stop()
- [ ] Run cortex-dream --execute to archive 32 duplicates
- [ ] Fix embedding spam — daemon logs 100+ "Ollama error: fetch failed" when Ollama is down
- [ ] Store Archana App requirements to Cortex (meeting was 2026-03-28)

---

## How to Use This File

- **Starting a session?** Read this first, pick the top unassigned item or your assigned item.
- **Starting work?** Move item to "In Progress" with your name.
- **Done?** Move to "Done" with date. Write a spec in `docs/conductor/specs/` only if the feature is complex.
- **New idea?** Add to Deferred with a one-line rationale. Don't expand the Up Next list without agreement.
