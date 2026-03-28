# Cortex

**A persistent, self-improving brain for AI coding agents.**

Cortex is a memory daemon that gives Claude Code, Gemini CLI, Codex, Cursor, and any other AI a shared, long-term brain. It stores decisions, detects conflicts between agents, compiles token-efficient boot prompts, and compounds intelligence across sessions — so every conversation starts smarter than the last.

```
You: "Build a game with neural network AI opponents"
AI:  [already knows your toolchain, past research, platform quirks, project conventions]
AI:  [pulls relevant decisions from 200+ sessions instantly]
AI:  [builds the plan in one shot because context is pre-loaded]
```

One daemon. One database. Every AI connects. Knowledge compounds.

---

## Quick Start

```bash
# Install
cd ~/cortex
npm install

# Start the daemon
node src/daemon.js serve

# Register with Claude Code
claude mcp add cortex -s user -- node C:\Users\aditya\cortex\src\daemon.js mcp

# Verify
curl http://localhost:7437/health
```

The daemon runs on `localhost:7437`. Any AI that can make HTTP requests can use it.

---

## How It Works

### The Capsule Compiler

When an AI boots, Cortex compiles a minimal prompt from two capsules:

**Identity capsule** (~200 tokens, stable): Who you are, platform rules, hard constraints, known sharp edges. Doesn't change between sessions.

**Delta capsule** (~50-100 tokens, fresh): What changed since this specific agent last connected. New decisions, new conflicts, state changes. Only the diff.

Result: ~300 tokens to fully orient any AI, versus 4,000+ tokens of raw file reads.

### Conflict Detection

When Claude stores "Use Python 3.12" and Gemini stores "Use Python 3.10," Cortex detects the semantic conflict via embedding similarity, marks both as disputed, and surfaces the disagreement in every subsequent boot prompt until a human resolves it.

### Multi-AI Architecture

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  Claude Code │  │  Gemini CLI  │  │  Codex CLI   │
│  (MCP+HTTP)  │  │  (HTTP)      │  │  (HTTP)      │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       │                 │                 │
       └─────────────────┼─────────────────┘
                         │
              ┌──────────▼──────────┐
              │   Cortex Daemon     │
              │   localhost:7437    │
              │                     │
              │  ┌───────────────┐  │
              │  │  SQLite DB    │  │
              │  │  (sql.js)     │  │
              │  └───────────────┘  │
              │                     │
              │  ┌───────────────┐  │
              │  │  Ollama       │  │
              │  │  Embeddings   │  │
              │  └───────────────┘  │
              └─────────────────────┘
```

---

## API

### HTTP Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/boot?agent=<id>` | No | Compiled boot prompt (capsule system) |
| `GET` | `/recall?q=<query>` | No | Hybrid semantic + keyword search |
| `GET` | `/health` | No | Daemon status, memory counts, Ollama status |
| `POST` | `/store` | Yes | Store a decision with conflict detection |
| `POST` | `/diary` | Yes | Write session state to state.md |
| `POST` | `/forget` | Yes | Decay matching memories by keyword |
| `POST` | `/resolve` | Yes | Resolve a disputed decision pair |
| `POST` | `/shutdown` | Yes | Graceful daemon shutdown |

Auth: `Authorization: Bearer <token>` (token at `~/.cortex/cortex.token`).

### MCP Tools

| Tool | Description |
|------|-------------|
| `cortex_boot` | Compiled boot prompt with capsule metadata |
| `cortex_recall` | Hybrid search across all memories and decisions |
| `cortex_store` | Store decision with conflict detection and dedup |
| `cortex_diary` | Write session handoff to state.md |
| `cortex_health` | System health check |
| `cortex_forget` | Decay memories matching a keyword |
| `cortex_resolve` | Resolve a dispute between decisions |

### CLI

```bash
cortex boot                    # Print compiled boot prompt
cortex recall "auth tokens"    # Search memories
cortex store "Use uv only"     # Store a decision
cortex health                  # System health
cortex status                  # PID, uptime, counts
cortex forget "deprecated"     # Decay matching entries
cortex resolve 42 --keep 37    # Resolve a conflict
cortex stop                    # Shutdown daemon
```

---

## Architecture

```
src/
  daemon.js      # HTTP + MCP server, auth, lifecycle
  brain.js       # Core: indexAll, recall, store, forget, diary
  compiler.js    # Capsule compiler (identity + delta) + legacy profiles
  embeddings.js  # Ollama nomic-embed-text vectors, cosine similarity
  conflict.js    # Semantic conflict detection, dispute management
  profiles.js    # Profile loader (full/operational/subagent/index)
  db.js          # SQLite via sql.js, schema, CRUD helpers
  cli.js         # CLI wrapper, auto-start daemon, HTTP client
```

**Design constraints:**
- One npm dependency: `sql.js` (SQLite compiled to WASM)
- No native compilation, no build step
- Works on Windows 10 without WSL
- Daemon stays alive across sessions, auto-starts on boot

---

## Current Status

### What Works Today
- Persistent memory across sessions (145+ memories, 4+ decisions)
- Hybrid recall: semantic (Ollama embeddings) + keyword fallback
- Cross-agent conflict detection via cosine similarity
- Capsule-based boot compilation (~300 tokens, agent-aware deltas)
- Auto-start daemon via SessionStart hook
- Multi-AI connectivity (Claude Code, Gemini CLI, Codex, Cursor)
- Auth on all mutation routes
- Test suite covering critical path

### What's In Progress
- Keyword fallback improvement (tokenized OR matching)
- Memory decay scoring (age-based, access-weighted)
- Pinned memories (never decay)

---

## Future Additions

### Phase 3: Retrieval & Decay
- [ ] Tokenized OR matching in keyword recall (not phrase LIKE)
- [ ] Recency weighting in search results
- [ ] `last_accessed` / `access_count` tracking on all entries
- [ ] Age-based decay scoring (score * 0.95 per day since access)
- [ ] Pinned flag for user-critical memories that never decay
- [ ] Score floor (0.1) to prevent complete erasure
- [ ] Spaced repetition boost (frequently retrieved = permanently strong)

### Phase 4: Dreaming & Compaction (Python Worker)
- [ ] `cortex-dream` Python worker process
- [ ] Nightly compaction via local Ollama (Qwen 2.5 32B)
- [ ] Deduplication: find overlapping decisions, synthesize canonical rules
- [ ] Source lineage: synthesized rules link back to originals
- [ ] "Summarize first, archive second, delete last" policy
- [ ] Conflict auto-flagging (not auto-resolution — human confirms)
- [ ] Temporal pattern extraction (what does the user do on Mondays?)
- [ ] Cross-session lesson synthesis (3 sessions hit same bug → master rule)
- [ ] Stale workaround detection (fix landed but workaround still active)
- [ ] Dream report: morning summary of what was compacted/synthesized

### Phase 5: JARVIS Dashboard (Python/Streamlit)
- [ ] `cortex-dash` Streamlit app at localhost:3333
- [ ] Memory explorer: browse, search, pin, archive, delete
- [ ] Knowledge graph visualizer (networkx + Streamlit)
- [ ] Conflict resolver UI: see both sides, pick winner, add context
- [ ] Agent activity timeline: who booted when, what they stored
- [ ] Token efficiency tracker: boot cost over time, per-agent
- [ ] Decay curve visualizer: memory half-life, retrieval frequency
- [ ] Memory quality heatmap: confidence x recency x access count
- [ ] Real-time memory feed: live stream of stores/recalls/conflicts
- [ ] Search playground: test recall queries, see relevance scores
- [ ] Capsule inspector: see exactly what each agent gets at boot
- [ ] Export/import: backup brain to JSON, restore from backup

### Phase 6: Ambient Knowledge Capture
- [ ] PostToolUse hook captures decisions automatically
- [ ] Confidence-labeled inbox (inferred / observed / user-confirmed)
- [ ] Promotion pipeline: inbox → episodic → canonical
- [ ] Research capture: web search results → structured knowledge entries
- [ ] Git commit capture: extract "why" from commit messages
- [ ] Code review capture: decisions from PR discussions
- [ ] Error pattern capture: recurring errors → prevention rules
- [ ] Solution pattern capture: successful fixes → reusable recipes
- [ ] Conversation summarization: end-of-session auto-extract
- [ ] File change capture: track which files changed together (co-change patterns)

### Phase 7: Advanced Retrieval
- [ ] Scoped recall: by project path
- [ ] Scoped recall: by file path
- [ ] Scoped recall: by agent ID
- [ ] Scoped recall: by recency window (last 24h, last week)
- [ ] Scoped recall: by memory type (decision, lesson, feedback, goal)
- [ ] Scoped recall: by confidence threshold
- [ ] Scoped recall: by status (active, disputed, archived)
- [ ] Session-delta compiler: "what changed since your last session on THIS project"
- [ ] Workspace map artifact: top-level modules, critical files, entry points
- [ ] Causal retrieval: "what caused X" → traverse dependency chains
- [ ] Predictive pre-loading: classify first 2-3 tool calls, inject relevant context
- [ ] Cross-project recall: lessons from project A surfaced in project B
- [ ] Structured debate storage: topic, stance, rebuttal, consensus, open questions

### Phase 8: Knowledge Graph
- [ ] Entity extraction: files, agents, errors, concepts, tools
- [ ] Relationship types: `caused_by`, `depends_on`, `resolved_by`, `supersedes`
- [ ] Graph queries: "why did deploy.sh fail?" → traversal
- [ ] Cluster detection: find groups of related memories
- [ ] Orphan detection: memories with no connections (candidates for decay)
- [ ] Graph visualization in dashboard (force-directed layout)
- [ ] Subgraph export: extract relevant neighborhood for a topic
- [ ] Graph-informed recall: boost results that are connected to the query topic

### Phase 9: Provider Cache Adapters
- [ ] Anthropic prompt cache adapter (prefix-based, 5min/1hr TTL)
- [ ] Gemini context cache adapter
- [ ] OpenAI cache adapter
- [ ] Cache as optional output of capsule compilation (text + cacheable block)
- [ ] Cache invalidation on capsule content change
- [ ] Cache hit rate tracking
- [ ] Cost savings calculator (cached vs uncached tokens)
- [ ] Auto-warm: periodically refresh caches before they expire

### Phase 10: Multi-Agent Coordination
- [ ] Consensus layer: when agents disagree, flag immediately
- [ ] Agent trust scoring: track which agents produce reliable decisions
- [ ] Agent specialization profiles: Claude is good at X, Gemini at Y
- [ ] Cross-agent learning: if Claude learns a fix, Gemini sees it next boot
- [ ] Debate protocol: structured multi-agent disagreement with resolution
- [ ] Agent handoff: one agent can leave context for another on the same task
- [ ] Collaborative research: dispatch parallel agents, merge findings
- [ ] Agent activity audit: what did each agent do, when, and was it correct?

### Phase 11: Self-Improvement Loop
- [ ] Track which capsule sections were actually referenced by AIs
- [ ] A/B test compilation strategies (more rules vs more decisions)
- [ ] Compiler self-tuning: adjust token budgets based on usage patterns
- [ ] Memory quality scoring: automatically identify high-value vs noise entries
- [ ] Retrieval quality metrics: was the recalled context actually used?
- [ ] Feedback loop: AIs report whether recalled context was helpful
- [ ] Auto-index new knowledge sources when detected
- [ ] Self-healing: detect and repair corrupted entries, stale embeddings
- [ ] Performance profiling: identify slow queries, optimize hot paths

### Phase 12: Integrations
- [ ] VS Code extension: file-aware context injection
- [ ] Browser extension: capture research from web browsing
- [ ] Git hooks: pre-commit captures decision context
- [ ] Slack/Discord bot: query brain from chat
- [ ] Notion import: pull existing knowledge bases
- [ ] Obsidian import: convert vault to Cortex memories
- [ ] Webhook API: notify external systems on conflicts/decisions
- [ ] REST SDK (Python): `pip install cortex-client`
- [ ] REST SDK (Node): `npm install cortex-client`
- [ ] REST SDK (Go): for Codex and other Go-based tools

### Phase 13: Infrastructure & Scale
- [ ] `npx cortex init` — zero-config setup for new users
- [ ] Configurable knowledge sources (not hardcoded to SIE paths)
- [ ] Multi-database support (one DB per project, shared global DB)
- [ ] Incremental re-indexing (watch files, re-embed only changes)
- [ ] Backup/restore: automated daily snapshots, point-in-time recovery
- [ ] Import/export: JSON format for brain portability
- [ ] Migration tools: OMEGA → Cortex, claude-mem → Cortex
- [ ] Health monitoring: alerting on daemon crashes, stale memories
- [ ] Performance profiling: query timing, embedding latency
- [ ] Rate limiting: protect against runaway ambient capture

### Phase 14: Advanced AI Features
- [ ] Voice interface: ask questions verbally, hear brain status
- [ ] Vision integration: screenshot → code context mapping
- [ ] Multi-modal memory: store images, diagrams, screenshots with context
- [ ] Autonomous research agent: "research X" → agent explores, stores findings
- [ ] Task planning from memory: "build Y" → brain generates plan from prior knowledge
- [ ] Natural language conflict resolution: describe the resolution in words
- [ ] Memory-powered code generation: generate boilerplate from past patterns
- [ ] Predictive bug detection: "this pattern caused issues in project A"

### Phase 15: Public Release
- [ ] Strip all hardcoded paths (configurable everything)
- [ ] Cross-platform testing (macOS, Linux, Windows)
- [ ] README with clear value proposition and demo GIF
- [ ] Documentation site (architecture, API reference, tutorials)
- [ ] Demo video: multi-AI brain in action
- [ ] Claude Code plugin marketplace listing
- [ ] Gemini CLI extension listing
- [ ] npm package: `npx create-cortex`
- [ ] Pricing model decision (open core? sponsorware? freemium?)
- [ ] Community Discord/GitHub Discussions
- [ ] Contributing guide
- [ ] Security audit
- [ ] License review (MIT vs Apache 2.0)

### Stretch / Experimental
- [ ] Team brain: shared Cortex instance with access control
- [ ] Cross-machine sync: Cortex on laptop + desktop, merged state
- [ ] Mobile companion app: read brain status, approve conflicts from phone
- [ ] Edge deployment: run Cortex on Raspberry Pi for always-on brain
- [ ] Federated learning: multiple users' Cortexes share anonymized patterns
- [ ] Time-travel queries: "what did the brain know on March 15th?"
- [ ] Memory archaeology: trace the evolution of a decision over time
- [ ] Dream journal: human-readable log of what compaction changed
- [ ] Brain transplant: fork a brain for a new project, inherit relevant knowledge
- [ ] Competitive mode: pit two compilation strategies against each other, measure

---

## Design Principles

1. **Compound, don't accumulate.** Every memory should make the next session smarter, not just bigger. Unused facts decay. Overlapping facts merge. The brain gets denser, not larger.

2. **Push, don't pull.** The brain injects context before being asked. AIs boot already knowing what matters. No "let me check my memory" — it's already there.

3. **Universal interface.** HTTP is the API. Any AI, any language, any platform. MCP is a convenience transport for Claude Code. HTTP is the truth.

4. **Reliability over intelligence.** A brain that crashes is worse than no brain. Every feature is tested. Every mutation is authenticated. Every failure is graceful.

5. **Node for the kernel, Python for the cortex.** The daemon is infrastructure — minimal, fast, boring. The intelligence layer is research — expressive, iterative, powerful. Different jobs, different tools.

6. **Evidence before assertions.** Never claim a feature works without a test. Never claim a bug is fixed without verification output. The brain holds itself to the same standard it holds its users.

---

## Project Structure

```
cortex/
  src/
    daemon.js         # HTTP + MCP server
    brain.js          # Core memory operations
    compiler.js       # Capsule + legacy compilation
    embeddings.js     # Ollama vector operations
    conflict.js       # Semantic conflict detection
    profiles.js       # Compilation profile loader
    db.js             # SQLite via sql.js
    cli.js            # CLI interface
  test/
    cortex-critical.test.js   # Critical path tests
    run-tests.js              # Test runner
  scripts/
    migrate-v1.js     # Migration from v1
  docs/
    ROADMAP.md                        # Detailed phase roadmap
    JARVIS_ARCHITECTURE_PROPOSAL.md   # Gemini's vision document
    CODEX_CORTEX_JARVIS_REVIEW.md     # Codex's code audit
    CLAUDE_OPUS_CORTEX_REVIEW.md      # Claude's synthesis + roadmap
    debates/                          # Multi-model architecture debates
  cortex-profiles.json  # Compilation profile config
  cortex.db             # SQLite database (gitignored)
  package.json
```

---

## Multi-AI Connection

### Claude Code (MCP + HTTP)
```bash
claude mcp add cortex -s user -- node C:\path\to\cortex\src\daemon.js mcp
```

### Gemini CLI (HTTP)
Add to `~/GEMINI.md`:
```markdown
## Brain Boot Protocol
At session start, read ~/.claude/brain-status.json and print the oneliner.
Use http://localhost:7437 for memory operations.
```

### Codex CLI (HTTP)
Add to `~/AGENTS.md`:
```markdown
## Brain Boot Protocol
At session start, read ~/.claude/brain-status.json and print the oneliner.
Use http://localhost:7437 for memory operations.
```

### Any Other AI (HTTP)
```bash
# Boot
curl http://localhost:7437/boot?agent=my-agent

# Recall
curl "http://localhost:7437/recall?q=authentication+patterns"

# Store (with auth)
curl -X POST http://localhost:7437/store \
  -H "Authorization: Bearer $(cat ~/.cortex/cortex.token)" \
  -H "Content-Type: application/json" \
  -d '{"decision": "Use JWT for API auth", "context": "api-design"}'
```

---

## License

MIT
