# Cortex Roadmap

**Goal:** A self-improving, multi-AI brain that compounds intelligence across sessions, agents, and projects. Not a toy. Not a demo. A real system that makes every AI session meaningfully smarter than the last.

---

## Architecture Decision: Node.js Core + Python Workers

The daemon (HTTP + MCP + SQLite) stays in Node.js. Everything else gets built in Python.

**Why Node.js for the daemon:**
- MCP transport (JSON-RPC over stdio) is native to Node
- sql.js (WASM SQLite) is the only dependency — no native compilation issues
- Event loop is naturally suited to "sit on a port, serve requests, stay alive"
- Already stable on Windows 10 without WSL

**Why Python for everything else:**
- ML/embedding pipelines (torch, sentence-transformers, numpy)
- Dashboard/visualization (Streamlit, Gradio, Dash)
- Local LLM orchestration for dreaming/compaction (Ollama Python SDK)
- Graph analysis and clustering (networkx, sklearn)
- Data science on memory quality (pandas, matplotlib)
- Voice/vision/multi-modal integration (future)
- The entire AI/ML ecosystem is Python-first

**The boundary:**
```
┌─────────────────────────────────────────────┐
│  Cortex Daemon (Node.js)                    │
│  HTTP :7437 + MCP stdio                     │
│  SQLite, recall, store, conflict, boot      │
│  Capsule compiler, auth, lifecycle          │
└────────────┬────────────────────────────────┘
             │ HTTP API (the universal interface)
    ┌────────┴──────────────────────┐
    │                               │
┌───▼──────────┐  ┌────────────────▼─────────────┐
│ Hooks (JS)   │  │ Cortex Workers (Python)       │
│ brain-boot   │  │                               │
│ session hooks│  │ - cortex-dream (compaction)   │
│              │  │ - cortex-dash (Streamlit UI)  │
│              │  │ - cortex-embed (batch embed)  │
│              │  │ - cortex-graph (analysis)     │
│              │  │ - cortex-decay (scoring)      │
│              │  │ - cortex-capture (ambient)    │
│              │  │                               │
│              │  │ All talk to daemon via HTTP    │
└──────────────┘  └──────────────────────────────┘
```

Python workers are independent processes. They read from and write to Cortex through the same HTTP API that any AI uses. No shared state, no tight coupling.

---

## Phase 0: Foundation (DONE)
- [x] Node.js daemon with HTTP + MCP
- [x] SQLite via sql.js (zero native deps)
- [x] Store, recall, conflict detection
- [x] Boot prompt compiler with profiles
- [x] Diary / state.md writer
- [x] Ollama embeddings (nomic-embed-text)
- [x] Auth on all mutation routes
- [x] brain-boot.js auto-start + status reporting
- [x] Cross-AI instruction files (GEMINI.md, AGENTS.md)

## Phase 1: Reliability (DONE)
- [x] Fix embedding type coercion in conflict detection
- [x] Fix auth gaps on /diary, /forget, /resolve
- [x] Fix CLI/daemon response contract mismatches
- [x] Fix diary keyDecisions field name
- [x] Surface disputes prominently in boot
- [x] Test suite (Codex Round 2)

## Phase 2: Capsule Compiler (DONE)
- [x] Identity capsule (stable, ~200 tokens)
- [x] Delta capsule (what changed since last boot, ~50-100 tokens)
- [x] Per-agent boot tracking (agent_boot events)
- [x] Legacy section compiler preserved as fallback
- [x] HTTP + MCP routes updated

## Phase 3: Keyword Fallback + Decay Scoring (Codex Round 4)
- [ ] Tokenized OR matching in keyword recall (not phrase LIKE)
- [ ] Recency weighting in search results
- [ ] last_accessed / access_count columns
- [ ] Decay-on-boot scoring pass
- [ ] Pinned flag for user-critical memories

## Phase 4: Python Worker — cortex-dream
- [ ] Nightly compaction via local Ollama (Qwen 2.5 32B)
- [ ] Deduplication: find overlapping decisions, synthesize canonical rules
- [ ] Source lineage: synthesized rules link back to originals
- [ ] "Summarize first, archive second, delete last" policy
- [ ] Conflict auto-flagging (not auto-resolution — human confirms)

## Phase 5: Python Worker — cortex-dash (JARVIS Visualizer)
- [ ] Streamlit dashboard at localhost:3333
- [ ] Memory explorer: browse, search, pin, archive
- [ ] Conflict resolver UI: see both sides, pick winner
- [ ] Knowledge graph visualizer (networkx → Streamlit)
- [ ] Token efficiency metrics (boot cost over time, cache hit rate)
- [ ] Agent activity timeline (who booted when, what they stored)
- [ ] Decay curve visualizer (memory half-life, retrieval frequency)

## Phase 6: Ambient Knowledge Capture
- [ ] PostToolUse hook captures decisions automatically (inbox, not direct store)
- [ ] Confidence-labeled inbox: inferred / observed / user-confirmed
- [ ] Promotion pipeline: inbox → episodic → canonical
- [ ] Research capture: when AIs do deep research, findings land in Cortex

## Phase 7: Advanced Retrieval
- [ ] Scoped recall: by project, by file path, by agent, by recency
- [ ] Session-delta compiler: "what changed since your last session on THIS project"
- [ ] Workspace map artifact: top-level modules, critical files, entry points
- [ ] Structured debate storage: topic, stance, rebuttal, consensus, open questions

## Phase 8: Provider Cache Adapters
- [ ] Anthropic prompt cache adapter (prefix-based, 5min TTL)
- [ ] Gemini context cache adapter
- [ ] OpenAI cache adapter (if useful)
- [ ] Cache as optional output of capsule compilation
- [ ] Cache invalidation on capsule content change

## Phase 9: Public Release Preparation
- [ ] `npx cortex init` — zero-config setup
- [ ] Strip hardcoded paths (configurable knowledge sources)
- [ ] README with clear value proposition
- [ ] Demo video showing multi-AI brain in action
- [ ] Pricing model decision (free core + paid workers? Open source + sponsorware?)
- [ ] Plugin marketplace listing for Claude Code

## Stretch Goals
- [ ] Voice integration (whisper + TTS for verbal brain queries)
- [ ] Mobile companion app (read brain status, approve conflicts)
- [ ] Multi-user Cortex (team brain, shared knowledge with access control)
- [ ] Cross-project intelligence (lessons from project A improve project B)
- [ ] Self-improving compiler (track which capsule content was actually useful)

---

## Design Principles

1. **Compound, don't accumulate.** Every memory should make the next session smarter, not just bigger. If a fact is stored but never retrieved, it should decay. If two facts overlap, they should merge.

2. **Push, don't pull.** The brain should inject context before being asked. The AI should boot already knowing what matters, not spend its first 5 messages asking questions.

3. **Universal interface.** HTTP is the API. Any AI, any language, any platform can connect. MCP is a convenience transport for Claude Code. HTTP is the truth.

4. **Reliability over intelligence.** A brain that crashes is worse than no brain. Every feature must be tested. Every mutation must be authenticated. Every failure must be graceful.

5. **Node for the kernel, Python for the cortex.** The daemon is infrastructure — it should be minimal, fast, and boring. The intelligence layer is research — it should be expressive, iterative, and powerful. Different jobs, different tools.
