---
date: 2026-03-28
topic: conductor-orchestration
focus: Multi-AI Conductor inside Cortex daemon
status: raw — not yet filtered
---

# Ideation: Conductor — Multi-AI Orchestration Layer

## Codebase Context

Cortex is a minimal Node.js daemon (8 modules, 1 dep, ~125KB). HTTP :7437 + MCP stdio. Already has: capsule compiler (97% token reduction at boot), conflict detection, semantic search, SQLite via sql.js, multi-AI support.

**Missing:** orchestration, dashboard, concurrency protection, ambient capture.

**Key constraints from prior debates:**
- Event-driven hooks for coordination (4-1 consensus), NOT per-prompt brain checks
- No write locking exists — concurrent `/store` calls risk data loss
- Dashboard should be separate process (per roadmap), not embedded in daemon
- Ambient capture needs inbox pattern with confidence gating

## Raw Ideas (40 total, 5 frames)

### Frame 1: User Pain & Friction

#### 1.1 Transcript Tap (Passive Conversation Indexing)
PostToolUse hook silently captures user prompts/AI responses, extracts decisions via local LLM, stores as searchable fragments. Eliminates the voluntary-reporting bottleneck where 90%+ of context evaporates because agents forget to `cortex_store`.
- **Impact:** Captures the 90% of decisions that currently vanish
- **Boldness:** 3

#### 1.2 File Lock Registry with Deadlock Detection
`/locks` endpoint family: acquire (file + agent + TTL), release, list. Auto-expire after 5min. Background sweep detects circular waits.
- **Impact:** Eliminates the #1 destructive failure mode of multi-agent setups
- **Boldness:** 2

#### 1.3 Token Dedup Router (Cross-Agent Context Elimination)
Track per-agent "knowledge state" as bloom filter of decision IDs. On boot, compute set difference — strip what agent already knows from other agents' recent stores. Report cumulative savings.
- **Impact:** Eliminates redundant context across concurrent agents
- **Boldness:** 3

#### 1.4 Conductor Task Board
`tasks` table + `/tasks` endpoints. User posts tasks with descriptions, file scopes, priority. Agents claim tasks on boot. Unclaimed tasks appear in delta capsules.
- **Impact:** Turns Cortex from memory system into coordination system
- **Boldness:** 3

#### 1.5 Ollama Sidecar Workers (Test Gen + Review)
Python workers poll `/tasks` for completed implementation tasks, run Qwen/DeepSeek analysis on changed files, store findings as `type: 'review'`. Findings appear in next agent's delta capsule.
- **Impact:** Local models validate cloud model output at zero API cost
- **Boldness:** 3

#### 1.6 Session Replay & Blame
`GET /replay?since=2h` returns time-ordered log of all decisions, conflicts, resolutions across agents. Git blame for the AI brain.
- **Impact:** Makes multi-agent failures auditable
- **Boldness:** 2

#### 1.7 Cortex Marketplace Protocol
Portable "brain export" format — anonymized rules/patterns from project types. Sell premium capsules ($5-20) for specific stacks.
- **Impact:** Revenue model + collective intelligence
- **Boldness:** 5

#### 1.8 Adversarial Consistency Checker
On conflict detection, spin up local LLM debate. Auto-resolve with confidence >0.9, present recommendation for <0.9. Store debate transcripts.
- **Impact:** Resolves 80% of conflicts automatically
- **Boldness:** 4

### Frame 2: Missing Capabilities

#### 2.1 Session Ledger with File Intent Declarations
`/session/register` + `/session/claim` endpoints. Advisory locking with live session tracking. No OS-level locks.
- **Impact:** Foundation for all coordination features
- **Boldness:** 2

#### 2.2 Ollama-as-Gatekeeper (Store/Recall Pre-screening)
Local LLM evaluates stores for novelty and recalls for relevance before committing/returning. Catches semantic contradictions that cosine similarity misses.
- **Impact:** Higher quality brain with zero API cost
- **Boldness:** 3

#### 2.3 Cross-Agent Replay Feed (Event Sourcing)
`GET /feed` SSE endpoint for real-time event stream. Dashboard and agents subscribe. Events already in SQLite — just needs push delivery.
- **Impact:** Makes dashboard and coordination trivial to build
- **Boldness:** 3

#### 2.4 Task Router with Agent Capability Profiles
`/tasks/create` + `/tasks/next` with routing based on capability profiles in `cortex-profiles.json`. Matches tasks to agents by difficulty tier and required capabilities.
- **Impact:** Automated task assignment — user stops being the orchestrator
- **Boldness:** 4

#### 2.5 Cortex-Native Test/Review Pipeline via Ollama
`POST /review` accepts git diffs, routes to local model for automated review. Results stored as decisions, surfaced in next boot.
- **Impact:** Continuous quality gate at zero cost
- **Boldness:** 4

#### 2.6 Context Budget Optimizer (Predictive Boot)
Track which recalled memories lead to follow-up stores (signal of usefulness). Per-agent relevance model — tailor boot content by historical usefulness, not just recency.
- **Impact:** Another 30-50% token reduction on boot prompts
- **Boldness:** 3

#### 2.7 Workspace Awareness via FileSystem Watcher
`fs.watch` on project directory. Auto-update session ledger, pre-warm recalls, detect conflicts when files change while another agent has them claimed.
- **Impact:** Cortex sees what's happening on disk, not just what agents report
- **Boldness:** 4

#### 2.8 Worker Registration Protocol (Plugin Marketplace)
`POST /workers/register` — any script registers with Cortex, declares capabilities, schedule, health-check. Cortex manages lifecycle. Third parties publish workers.
- **Impact:** Turns Cortex from monolith into platform
- **Boldness:** 5

### Frame 3: Inversion & Automation

#### 3.1 Session Ledger (Locks via Daemon)
In-memory lock map (file path → agent + timestamp), auto-expire 5min. Lock table injected into delta capsules at boot.
- **Impact:** ~20 line change, prevents file collision class
- **Boldness:** 2

#### 3.2 Conductor EventBus (SSE Push)
Replace pull-based `/digest` with SSE on `GET /events/stream`. Dashboard becomes reactive. Future conductor responds to events in real-time.
- **Impact:** Inverts the entire data flow from pull to push
- **Boldness:** 3

#### 3.3 Ollama Triage Router
`POST /triage` accepts task description, classifies via GLM-4.7-Flash, routes work to Qwen 32B or DeepSeek R1. Cortex becomes local compute broker.
- **Impact:** Turns idle VRAM into cost savings
- **Boldness:** 4

#### 3.4 Ghost Protocol (Automatic Decision Capture)
`POST /ingest` accepts raw transcripts. Local LLM extracts decisions to inbox table with `confidence: 'inferred'`. Dashboard shows for human promotion.
- **Impact:** Brain learns whether you meant it to or not
- **Boldness:** 4

#### 3.5 Kill the Boot Call (Pre-computed Context Injection)
Cortex writes compiled context directly into CLAUDE.md/AGENTS.md/GEMINI.md files. Agents boot warm automatically, zero API calls.
- **Impact:** Eliminates boot call entirely — zero-latency, zero-token boot
- **Boldness:** 5

#### 3.6 Decision Replay Debugger
`GET /replay?agent=X&since=DATE` returns markdown narrative timeline. "What did Claude and Codex disagree about last Tuesday?"
- **Impact:** Makes the brain auditable and demo-able
- **Boldness:** 2

#### 3.7 Cortex-as-Referee (Automated Conflict Resolution)
Local LLM evaluates both sides of a dispute, writes recommendation with confidence. Human still confirms but sees reasoned analysis.
- **Impact:** Auto-resolves the easy 80% of conflicts
- **Boldness:** 3

#### 3.8 Tenant Isolation Layer
Add `tenant_id` to all tables. Auth token becomes tenant-scoped. Single daemon serves multiple users. Prerequisite for commercial hosting.
- **Impact:** Unlocks every commercial path (team, hosted, enterprise)
- **Boldness:** 5

### Frame 4: Leverage & Compounding

#### 4.1 Session Bus (Agent Presence Protocol)
`/session/start` + `/session/end` with heartbeat. Real-time registry of active agents, their project, and open files. Foundation for everything else.
- **Impact:** You cannot orchestrate what you cannot see
- **Boldness:** 2

#### 4.2 File Lock Ledger with Semantic Conflict Prediction
Advisory locks + semantic search predicts LIKELY conflicts from file coupling. "auth.js and middleware.js are tightly coupled — warning."
- **Impact:** Catches problems before code is written
- **Boldness:** 3

#### 4.3 Recall-Driven Task Router (Token Arbitrage Engine)
Route tasks based on how much Cortex context exists. Rich context → expensive model (fast with recall). Zero context → cheap local model (research first, store for next time).
- **Impact:** Maximizes ROI per token — THE core flywheel made explicit
- **Boldness:** 4

#### 4.4 Decision Provenance DAG
Directed acyclic graph tracing every decision through its lineage. Which recall triggered it, which agent stored it, which dream synthesized it. Confidence decay by lineage.
- **Impact:** Makes the brain auditable at scale + structurally-informed quality
- **Boldness:** 3

#### 4.5 Ambient Inbox with Promotion Pipeline
PostToolUse hook captures observations to inbox with auto-confidence. Repeating patterns promote to memory (0.7) then decision (0.9). Brain learns from behavior.
- **Impact:** Captures the 90% of knowledge in what agents DO vs what they SAY
- **Boldness:** 3

#### 4.6 Self-Tuning Compiler
Track which capsule content gets referenced after boot. Shift budget toward high-retrieval sections. Compiler improves itself over 100 boots.
- **Impact:** Another 30-40% boot cost reduction, fully automatic
- **Boldness:** 4

#### 4.7 Multi-Tenant Cortex (Cortex-as-a-Service)
Multiple isolated SQLite databases per user, tenant-scoped auth, deployed as hosted service. Free <500 memories, $9/mo pro, $29/mo team.
- **Impact:** The commercial product path
- **Boldness:** 5

#### 4.8 Dream Consensus Protocol
Run synthesis through 2-3 local models simultaneously, require consensus. 2-of-3 agreement = high confidence. Catches single-model hallucination.
- **Impact:** Quality flywheel — better dreams → better boots → better behavior
- **Boldness:** 4

### Frame 5: Assumption-Breaking

#### 5.1 Conductor-as-Compiler (Not a Dashboard)
Make orchestration another compilation phase. Each agent boots knowing "Gemini is on tests, Codex is on workers — avoid those files." Zero new infrastructure. Just a wider capsule.
- **Impact:** Reframes the entire problem — coordination IS compilation
- **Boldness:** 4

#### 5.2 Annotated Files, Not Locked Files
Instead of locking, store semantic annotations when agents modify files. Next agent gets context ("Claude assumed JSON return type 12min ago"), not a block. Conflict detection handles the rest.
- **Impact:** Optimistic concurrency > pessimistic locking
- **Boldness:** 4

#### 5.3 Ollama as the Orchestration Brain
Local model watches events table in real-time, produces micro-directives. Local model = project manager (free). Cloud models = developers (paid).
- **Impact:** $0/hr intelligence for orchestration, $20/hr for execution
- **Boldness:** 5

#### 5.4 Cortex Protocol (Open Standard) + Cortex Cloud
Publish the boot-compile-recall-store-conflict protocol as open standard. Sell hosted Cortex Cloud for teams. Redis/Upstash model. The moat is the protocol, not the daemon.
- **Impact:** Adoption via open standard, revenue via hosted service
- **Boldness:** 5

#### 5.5 Event-Sourced Brain
Make events table THE source of truth. Current tables become materialized views. Eliminates race conditions, enables time-travel queries, simplifies dreaming.
- **Impact:** Eliminates fundamental concurrency bugs
- **Boldness:** 3

#### 5.6 Recall Learns from Downstream Success
Track which recalled memories lead to successful outcomes. Memories that help solve problems float up, noise sinks. Reinforcement learning for recall.
- **Impact:** Recall evolves from relevance matching to outcome optimization
- **Boldness:** 3

#### 5.7 Structured Dissent Protocol
Instead of resolving conflicts (destroying information), store structured debates as first-class objects. Preserve the decision SURFACE, not just the decision POINT.
- **Impact:** Institutional memory that captures reasoning, not just conclusions
- **Boldness:** 4

#### 5.8 Dependency-Graph Routing
Analyze codebase import graph, partition into weakly-connected subgraphs. Assign each subgraph to one agent. Routing by code structure, not human judgment.
- **Impact:** Eliminates manual routing — safe parallelism computed from code
- **Boldness:** 5

## Session Log
- 2026-03-28: Initial ideation — 40 ideas generated across 5 frames, filtering pending
- 2026-03-28: Droid onboarding in progress — will participate in adversarial filtering
