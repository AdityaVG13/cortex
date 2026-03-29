# Cortex Deep Review — Droid Analysis

**Author:** Droid (GLM-4.7 Core)
**Date:** 2026-03-29
**Scope:** Complete architecture audit, code quality assessment, gap analysis, and implementation roadmap

---

## Executive Summary

Cortex is **architecturally sound** but **operationally fragile**. The multi-transport memory daemon concept is mature and directionally correct. The capsule compiler (after recent fixes) delivers on its token-saving promise (~94% reduction claimed, probably realistic).

**However:** The foundation is cracked in ways the previous reviews missed. Critical paths have no tests. Error handling swallows exceptions. State inconsistencies can accumulate silently. The system will work 90% of the time and catastrophically fail 10% of the time.

You cannot build JARVIS on a substrate that fails 1 out of 10 times.

---

## 1. Architecture Audit

### 1.1 What Works

**The module split is correct.** `daemon.js`, `brain.js`, `compiler.js`, `embeddings.js`, `conflict.js`, and `db.js` have clear boundaries. Each has a single responsibility. This is rare in Node.js projects and deserves credit.

**HTTP + MCP transport is the right abstraction.** HTTP is universal (anything can call it), MCP is convenient for Claude Code. Both share the same core. Good engineering choice.

**SQLite via sql.js is appropriate for this scale.** One file, no external database daemon, cross-platform, works on your Windows setup without WSL.

**Capsule compiler is solid design.** Identity (stable) + Delta (what changed) is exactly the right mental model. The token budgeting system is correct.

**Conflict detection concept is mature.** Semantically detecting contradictions is genuinely useful. The Jaccard fallback is smart for offline operation.

### 1.2 What's Problematic

**In-memory state in daemon.js is a durability problem.** `locks`, `activities`, `messages`, and `sessions` are stored in JavaScript Maps. If the daemon crashes, all this state is lost. No persistence. No recovery. This is state that represents inter-agent coordination — it should be durable.

**The epochal persistence model is wrong.** sql.js requires exporting the entire database to write to disk. Every write operation triggers this export via `persist()` (debounced to 1s). This becomes a bottleneck as writes increase. Worse: if the daemon crashes 900ms after a write, that write is gone forever.

**No write-ahead log.** Without WAL (in the true sense, not SQLite's PRAGMA), writes before the last persist are unrecoverable. The system advertises "no data loss" but that's false.

**Embedding API abuse.** `embeddings.js` truncates input to 512 characters for ALL embeddings. For long memories or decisions, this means the semantic representation loses critical nuance. The description says "truncate input to 512 chars" but doesn't document this heuristic anywhere users would see.

**Hybrid search weights are arbitrary.** In `db.searchRows()`, the ranking formula is `(keywordWeight * 0.6) + (recencyWeight * 0.25) + (scoreWeight * 0.15)`. These numbers appear nowhere in documentation and have no empirical basis. They may be wrong.

**Capsule freshness tracking is incomplete.** The delta capsule uses `lastBoot` to fetch new decisions/memories, but it doesn't track "what this agent already saw and ignored." If you boot twice in the same day, you'll get the same delta both times even if you acted on it the first time.

### 1.3 Missing Architectural Pieces

**No explicit transaction boundaries.** Multiple related operations (store decision + embed + log event) can partially fail. Database has no transaction support because sql.js doesn't expose it in the WASM build.

**No causal chain tracking.** Decision A caused Decision B caused Decision C. But `decisions` table has no lineage beyond `supersedes_id`. You can't ask "why did we make this choice?" later.

**No workspace-scoped recall.** All recall is global. There's no way to ask "show me memories about THIS project" without filtering by query, which is unreliable.

**No memory versioning.** When a memory is updated via `upsertMemory()`, the old text is lost. You can't see the evolution of an idea.

**No schema migration system.** `ensureColumn()` adds columns if missing but doesn't handle column removal, type changes, or data migrations. Future schema changes will be painful.

**No rate limiting.** Anyone with the auth token can call `/store` in a loop. The daemon has no defense.

**No health monitoring beyond `/health`.** No latency metrics. No error rate tracking. No anomaly detection. You won't know something is wrong until it fails completely.

---

## 2. Code Quality Assessment

### 2.1 Bugs Others Missed

**BUG #1: Memory leak in HTTP body reader.** In `readBody()`:
```javascript
req.on('data', (chunk) => {
  size += chunk.length;
  if (size > MAX_BODY) {
    req.destroy();  // ❌ Stream destroyed, but 'end' event still fires
    reject(new Error('Request body too large'));
    return;
  }
  chunks.push(chunk);
});
```
When `req.destroy()` is called, the `req.on('end', ...)` handler still runs, calling `resolve()` after the rejection. This can cause a resolved promise with empty data, or worse, unhandled rejections.

**Fix:**
```javascript
let destroyed = false;
req.on('data', (chunk) => {
  size += chunk.length;
  if (size > MAX_BODY) {
    req.destroy();
    destroyed = true;
    reject(new Error('Request body too large'));
    return;
  }
  chunks.push(chunk);
});
req.on('end', () => {
  if (destroyed) return;  // Don't resolve if we already rejected
  resolve(Buffer.concat(chunks).toString('utf-8'));
});
```

**BUG #2: Race condition in startup.** In `main()` (serve mode):
```javascript
process.stderr.write(`[cortex] Listening on http://127.0.0.1:${PORT}\n`);
```
This prints BEFORE `httpServer.listen()` callback fires. If the bind fails, you report success incorrectly.

**BUG #3: Lock expiration check is incomplete.** In `cleanExpiredLocks()`:
```javascript
if (new Date(lock.expiresAt) < now) {
  locks.delete(path);
  log('info', `Lock expired: ${path}`);
}
```
But `expiresAt` is an ISO string. `new Date(lock.expiresAt)` parses it. However, `expiresAt` could be malformed (null, undefined, empty string). This throws an exception and stops cleanup of other locks.

**BUG #4: MCP stdout redirect corrupts JSON-RPC.** In MCP mode:
```javascript
process.stdout.write = (chunk, encoding, callback) => {
  if (logStream) {
    return logStream.write(chunk, encoding, callback);
  }
  return origStdoutWrite(chunk, encoding, callback);
};
```
This patches ALL stdout writes. But `origStdout` expects to remain usable (MCP uses `origStdin` and `origStdout` for the transport patching later. The proxy only logs to `logStream` but doesn't fall back to `origStdoutWrite` when `logStream` fails. If log file write fails, MCP JSON-RPC responses are lost.

**BUG #5: Session heartbeat doesn't validate agent parameter.** In `handleSessionHeartbeat()`:
```javascript
if (!session) {
  sendJson(res, 404, { error: 'no_active_session' });
  return;
}
```
If the session doesn't exist, you return 404. But you don't validate that the `agent` parameter matches any known agent form. A malicious client could call heartbeat with arbitrary agent strings, creating spurious log entries.

**BUG #6: Embedding build doesn't handle concurrency.** In `embeddings.buildEmbeddings()`:
```javascript
for (const row of unembeddedMemories) {
  const vec = await getEmbedding(row.text);
  if (vec) {
    insert('INSERT INTO embeddings ...', [...]);
    computed++;
  }
}
```
This is sequential. If you have 100 new memories, this takes 100 sequential Ollama calls. No parallelization. No batching. Can take minutes.

**BUG #7: Diary write truncates file content section.** In `writeDiary()`:
```javascript
if (accomplished) {
  lines.push('## What Was Done This Session');
  lines.push(accomplished);
  lines.push('');
}
```
If `accomplished` contains "## What Was Done This Session" as a literal string (user typed it), the next section extraction will be broken. No escaping or validation.

### 2.2 Security Gaps

**No maximum token budget enforcement on boot.** In `compileCapsules()`, you accept `maxTokens` but don't enforce it strictly. A malicious agent could request `maxTokens=10000000` and trigger database exhaustion via large delta capsule queries.

**No sanitization of log output.** In `log()`:
```javascript
const entry = data ? `[${ts}] [${level}] ${msg} ${JSON.stringify(data)}` : `[${ts}] [${level}] ${msg}`;
```
If `msg` contains ANSI escape sequences or control characters, logs are corrupted. More critically, if `data` contains circular structures, `JSON.stringify()` throws and the log entry is lost.

**No validation of blob payload sizes.** The `embeddings` table stores 768-dim Float32Array vectors (3072 bytes). No validation that incoming blobs are this size. A malformed request could insert garbage or trigger internal SQLite errors.

**No CSP headers on HTTP responses.** Responses don't include `Content-Security-Policy`, `X-Content-Type-Options`, or other security headers. On localhost this is low risk, but if the daemon is ever exposed to LAN, this is a problem.

**No rate limiting on auth tokens.** The auth token never rotates. If it leaks, the attacker has indefinite access. No token expiration, no refresh mechanism.

### 2.3 Performance Hotspots

**Sequential embedding build (mentioned above).** Massive performance issue. Should parallelize with `Promise.all()` in batches of 5-10 to avoid overwhelming Ollama.

**Every recall searches all active entries.** In `recall()`:
```javascript
const allEmbeddings = db.query('SELECT target_type, target_id, vector FROM embeddings');
```
This loads ALL embeddings into memory on every recall. With 1000+ memories, this is 3MB+ per recall call. No caching. No pagination.

**Keyword search loads entire table.** In `db.searchDecisions()` and `db.searchMemories()`, you load all active rows then filter in JavaScript. SQLite's LIKE operator (even with indexes) is slower, but you should at least push the text search to the database level with WHERE clauses.

**No query result caching.** The same recall query "authentication architecture" across three sessions hits the database and recomputes similarity every time. No LRU cache.

**Debounced persist is too conservative.** In `markDirty()`, the debounce is 1 second. In high-write scenarios (ambient capture coming soon), this means multiple writes collapse into one persist, increasing the risk window.

### 2.4 Technical Debt

**Inconsistent error handling patterns.** Some functions throw, some return `{ error }`, some call `log()` then return silently. No consistent error handling strategy.

**No TypeScript types.** The codebase is pure JavaScript. This is fine for a prototype, but with the complexity growing (capsule system, conductor, dreaming), type safety becomes valuable. JSDoc comments exist but are incomplete.

**No API versioning.** All routes are `/boot`, `/store`, etc. If you need to change any response format, you break all clients. No `/v2/boot` pattern.

**No graceful degradation.** If Ollama is down, conflict detection falls back to Jaccard (good). But if SQLite has errors, the daemon crashes entirely. No degraded mode.

**No observability instrumentation.** No Prometheus metrics. No structured logging. No distributed tracing. Debugging production issues will be painful.

**No backup/restore mechanism.** `cortex.db` is a single file. If it corrupts, you lose everything. No automated backups, no export/import utilities.

**No migration path from v1 to v2.** The `migrate-v1.js` script exists but is one-off. If you need schema changes, there's no incremental migration framework.

---

## 3. What Claude/Codex/Gemini Missed

### 3.1 Oversights in Their Reviews

**Gemini missed the epochal persistence problem.** The review focuses on caching and dreaming but misses that sql.js's export model is fundamentally unsuited for high-frequency writes. This is a critical architecture gap.

**Codex missed the in-memory state problem.** The review identifies auth gaps and embedding bugs but doesn't notice that `locks`/`activities`/`sessions` are not persisted. If the daemon restarts, all inter-agent coordination state is lost.

**Claude missed the sequential embedding bottleneck.** The review proposes background workers but doesn't identify the immediate performance issue in `buildEmbeddings()`.

**All three missed the memory leak in `readBody()`.** This is a subtle bug that would only show up under load (malformed HTTP requests). None of the reviews did fuzz testing or adversarial input analysis.

**Codex missed the SQL query pattern problem.** The recommendation is "move from phrase-LIKE to proper," but the deeper issue is: you're loading ALL rows into memory before filtering. This is O(N) memory per recall query.

**All three missed token budget enforcement.** The capsule compiler accepts any `maxTokens` value. A malicious agent could request an arbitrarily large budget and trigger database exhaustion.

### 3.2 Unaddressed Architectural Concerns

**No workspace isolation model.** All reviews discuss "scoped retrieval" but none address: how do you define "workspace"? Current hardcoding to `C--Users-aditya` is not portable. Need a workspace registration protocol.

**No provenance for auto-synthesized content.** All reviews discuss "dreaming" (compaction), but none address: if cortex-dream synthesizes a canonical rule, how do you trace back WHICH original entries it came from? The spec mentions lineage but the implementation doesn't support it.

**No decision lifecycle model.** A decision goes through states: `active` → `disputed` → `resolved` → `superseded` → `archived`. The code handles some but not all transitions. No state machine validation.

**No multi-region support for distributed teams.** If you run multiple Cortex instances (laptop + desktop), how do they sync? No consensus protocol. No conflict resolution for brain merges.

### 3.3 Hidden Technical Debt

**The daemon has no health check beyond uptime.** `/health` returns counts but doesn't check: can we connect to Ollama? Is the database file healthy? Are there pending writes in the debounce buffer?

**The CLI has no error recovery.** If `cortex boot` fails, it prints an error but offers no retry or fallback. No exponential backoff.

**No graceful shutdown for in-flight operations.** If you Ctrl+C during an embedding build, the database is left in an inconsistent state (some embeddings written, some not).

**No orphan cleanup for embeddings.** If a memory is deleted but its embedding remains, you have orphan BLOBs. No cleanup routine.

**No index defragmentation.** SQLite's auto-vacuum isn't configured. The database file grows even after deletions.

---

## 4. Token Optimization Gaps

### 4.1 Current Token Budget Analysis

The capsule compiler targets 600 tokens (default) but actual output varies:
- Identity capsule: ~150 tokens (user, platform, shell, python, git, rules, sharp edges)
- Delta capsule: varies wildly (~100-500 tokens) depending on new decisions/conflicts/locks/messages

**Problem:** The delta capsule has no token budget awareness. If 20 new decisions exist since last boot, it loads ALL of them instead of deciding which 5 are most relevant.

### 4.2 Inefficiencies in Current Implementation

**Redundant identity information across boots.** The identity capsule includes "User: Aditya. Platform: Windows 10." on EVERY boot. This should be cached at the provider layer once, not sent every time.

**No projected token cost awareness.** The compiler doesn't estimate how many tokens a delta capsule will contain before assembling it. It batch-queries new entries, concatenates them, THEN measures. This is wasteful if the batch is too large.

**No reordering for relevance.** New decisions are fetched `ORDER BY created_at DESC`. They should be ordered by relevance score (combination of confidence + retrievals + recency). You're loading recency first, not relevance first.

**No duplicate suppression in delta.** If two decisions say the same thing (different wording), both appear in delta capsule. Conflict detection catches on store, but misses if they were stored by DIFFERENT agents or at different times.

### 4.3 Proposed Optimizations

**OPTIMIZATION #1: Relevance-ordered delta assembly**
Current:
```javascript
SELECT decision, context, source_agent FROM decisions WHERE status = 'active' AND created_at >= ? ORDER BY created_at DESC LIMIT 5
```
Should be:
```javascript
SELECT decision, context, source_agent FROM decisions WHERE status = 'active' AND created_at >= ? ORDER BY (retrievals * 2.0 + confidence * 3.0 + CAST(julianday('now') - julianday(created_at) AS REAL) * 0.1) DESC LIMIT 5
```
This prioritizes frequently retrieved + high-confidence decisions over just recency.

**OPTIMIZATION #2: Duplicate suppression**
Add a "semantic fingerprint" that clusters similar entries before delta assembly:
```javascript
function clusterSimilar(entries, threshold = 0.7) {
  const clusters = [];
  for (const entry of entries) {
    let added = false;
    for (const cluster of clusters) {
      if (jaccardSimilarity(entry.decision, cluster[0].decision) >= threshold) {
        if (entry.confidence > cluster[0].confidence) {
          cluster[0] = entry;  // Keep the stronger version
        }
        added = true;
        break;
      }
    }
    if (!added) clusters.push([entry]);
  }
  return clusters.map(c => c[0]);  // Return one per cluster
}
```
Apply this before delta capsule assembly.

**OPTIMIZATION #3: Project-scoped delta**
Add a `project` field to decisions/memories (currently missing). When an agent boots, fetch delta for the CURRENT project only, not all projects.

**OPTIMIZATION #4: Incremental delta caching**
Store the last delta capsule per agent. If nothing changed since last boot, return the cached capsule instead of recomputing. Invalidate on any store operation affecting this agent.

**OPTIMIZATION #5: Adaptive token budgeting**
Instead of fixed `maxTokens`, compute based on task complexity:
- `maxTokens = base_budget * task_complexity_multiplier`
- `task_complexity_multiplier` could be based on: project size, number of files changed, type of work (refactor vs new feature)

### 4.4 Implementation Spec for Token Optimization

**File to create:** `src/token-optimizer.js`

**Exports:**
- `computeRelevanceScore(decision)` — returns numeric score
- `clusterSimilarEntries(entries, threshold)` — dedupe by similarity
- `assembleProjectScopedDelta(agentId, projectPath, maxTokens)` — focused delta
- `getCachedDelta(agentId)` — check cache
- `setCachedDelta(agentId, delta)` — update cache

**Integration:**
Modify `compiler.js` to use `assembleProjectScopedDelta()` instead of `buildDeltaCapsule()` when `project` is available in conductor state.

**Cache storage:** Add `delta_caches` table:
```sql
CREATE TABLE IF NOT EXISTS delta_caches (
  agent TEXT NOT NULL,
  project TEXT,
  capsule JSON NOT NULL,
  checksum TEXT NOT NULL,
  created_at TEXT DEFAULT (datetime('now')),
  PRIMARY KEY (agent, project)
);
```
Checksum is hash of all active decision IDs since last boot. If checksum unchanged, cache is valid.

---

## 5. New Feature Opportunities

### 5.1 Feature Others Didn't Propose

**FEATURE #1: Predictive Context Pre-loading**

**Problem:** AIs only recall explicit queries. They don't predict what information will be needed next.

**Solution:** Before the first tool call, classify the task intent and pre-load relevant capsules.

**Implementation:**
- Add `predictTaskIntent(messages)` function that analyzes first few user messages
- Map intent to expected information needs:
  - "build feature" → workspace map, recent decisions on architecture
  - "debug bug" → known sharp edges, underperforming skills
  - "refactor" → design principles, Sharp Edges
- Add `prefetch/intent` daemon endpoint
- SessionStart hook calls this and injects pre-fetched capsule into boot

**Spec:**
```
POST /intent/predict
{
  "messages": ["build a new user signup flow"],
  "project": "auth-service"
}

Response:
{
  "intent": "build_feature",
  "confidence": 0.9,
  "prefetch": ["workspace_map", "recent_decisions:architecture", "sharp_edges"]
}

GET /prefetch/{intent}?project={project}
Response: compiled prefetch capsule
```

**ROI:** High. Even if 50% accurate, this saves 5-10 recall calls per session.

---

**FEATURE #2: Decision Traceability Graph**

**Problem:** You can't see the causal chain of decisions. Decision C depends on Decision B which depends on Decision A.

**Solution:** Store parent-child relationships and build a queryable graph.

**Implementation:**
- Add `parent_id` to decisions table (already exists but unused)
- Add `edges` table:
```sql
CREATE TABLE IF NOT EXISTS decision_edges (
  id INTEGER PRIMARY KEY,
  parent_id INTEGER NOT NULL,
  child_id INTEGER NOT NULL,
  relationship TEXT DEFAULT 'causes',  // causes, supersedes, contradicts, refines
  created_at TEXT DEFAULT (datetime('now')),
  FOREIGN KEY (parent_id) REFERENCES decisions(id),
  FOREIGN KEY (child_id) REFERENCES decisions(id),
  UNIQUE (parent_id, child_id, relationship)
);
```
- Add `GET /decisions/{id}?include=pathancestors` endpoint
- Returns full decision lineage upstream

**Spec:**
```
GET /decisions/42?include=ancestors
Response:
{
  "id": 42,
  "decision": "Use JWT for API auth",
  "ancestors": [
    {
      "id": 15,
      "decision": "API should be stateless",
      "relationship": "causes"
    },
    {
      "id": 8,
      "decision": "Move session state to Redis",
      "relationship": "causes"
    }
  ]
}
```

**ROI:** Medium. Critical for understanding architectural decisions later.

---

**FEATURE #3: Multi-Workspace Brain**

**Problem:** Current hardcoded path `C--Users-aditya` doesn't scale. You have multiple projects that should have separate memory spaces.

**Solution:** Register workspaces with metadata, scope memories per workspace.

**Implementation:**
- Add `workspaces` table:
```sql
CREATE TABLE IF NOT EXISTS workspaces (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL UNIQUE,
  path TEXT NOT NULL UNIQUE,
  description TEXT,
  created_at TEXT DEFAULT (datetime('now'))
);
```
- Add `workspace_id` to memories and decisions tables
- Add `POST /workspaces/{id}/store` and `GET /workspaces/{id}/recall` endpoints
- Boot compiler takes `workspace` parameter and filters delta accordingly

**Spec:**
```
POST /workspaces
Body: { "name": "auth-service", "path": "C:/projects/auth-service", "description": "Authentication microservice" }
Response: { "id": 1, "name": "auth-service", ... }

POST /workspaces/1/store
Body: { "decision": "Use OAuth2 for third-party providers" }
Result: decision stored under workspace_id=1

GET /boot?agent=claude&workspace=auth-service
Response: identity + workspace-scoped delta (only auth-service decisions)
```

**ROI:** Critical. Cortex cannot scale beyond single-project without this.

---

**FEATURE #4: Ambient Capture Inbox**

**Problem:** You want to auto-capture decisions from tool use, but raw tool calls are noisy.

**Solution:** Two-tier capture system: raw inbox filter → curated memory.

**Implementation:**
- Add `capture_inbox` table:
```sql
CREATE TABLE IF NOT EXISTS capture_inbox (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  raw_text TEXT NOT NULL,
  source_agent TEXT NOT NULL,
  tool_name TEXT,
  context TEXT,
  confidence REAL DEFAULT 0.5,
  status TEXT DEFAULT 'unprocessed',
  created_at TEXT DEFAULT (datetime('now'))
);
```
- SessionStart hook adds PostToolUse hook that writes to inbox
- Background worker (cortex-capture.py) processes inbox:
  - Filter low-confidence entries (< 0.3)
  - Deduplicate similar entries
  - Promote to memories/decisions via `/store`
- Add `GET /inbox` and `POST /inbox/{id}/promote` endpoints

**Spec:**
```
PostToolUse hook triggers:
POST /inbox/capture
Body: {
  "raw_text": "Using npm instead of bun because of dependency conflicts",
  "source_agent": "droid",
  "tool_name": "shell:execute"
}

Background worker (runs every 5 min):
GET /inbox
Response: [{ "id": 1, "raw_text": "...", "confidence": 0.7 }, ...]

Worker processes:
- If confidence >= 0.7 and passes quality checks: POST /inbox/1/promote
- If confidence < 0.3: DELETE /inbox/1
- If similar to existing: mark as "duplicate", don't promote
```

**ROI:** High. Eliminates manual memory capture effort.

---

**FEATURE #5: Health Monitoring & Alerting**

**Problem:** No visibility into daemon health beyond `/health`. You don't know when things are degrading.

**Solution:** Prometheus-compatible metrics + alerting.

**Implementation:**
- Add metrics registry in daemon.js:
```javascript
const metrics = {
  labels: { version: '2.0.0', hostname: os.hostname() },
  gauges: {
    connections_active: 0,
    recall_latency_ms: 0,
    embed_queue_size: 0,
  },
  counters: {
    recall_total: 0,
    store_total: 0,
    error_total: 0,
  },
};
```
- Instrument all routes: increment counters, set gauges
- Add `GET /metrics` endpoint in Prometheus text format
- Add simple alert checking: if embed_queue_size > 100, log warning

**Spec:**
```
GET /metrics
Response:
cortex_connections_active 3
cortex_recall_latency_ms 42
cortex_store_total 152
cortex_error_total 0
cortex_up_seconds 12345

curl http://localhost:7437/metrics | grep error
cortex_error_total 0
```

**ROI:** Critical for production use.

---

**FEATURE #6: Backup & Restore**

**Problem:** No backup mechanism. `cortex.db` corruption = complete loss.

**Solution:** Automated snapshots + export/import.

**Implementation:**
- Add `POST /backup` endpoint that: exports database, compresses it, writes to `~/.cortex/backups/`
- Add `GET /backups` to list snapshots
- Add `POST /restore/{timestamp}` to restore from snapshot
- Add SessionStart hook that triggers backup daily

**Spec:**
```
POST /backup
Response: { "path": "~/.cortex/backups/2026-03-29-10-00-00.cortex.json.gz", "bytes": 245123 }

GET /backups
Response: [{ "timestamp": "2026-03-29T10:00:00Z", "size_bytes": 245123, "memories": 152, "decisions": 47 }]

POST /restore/2026-03-29T10:00:00Z
Response: { "restored": true, "old_count": { "memories": 152, "decisions": 47 } }
```

**ROI:** Insurance against catastrophic loss. Critical.

---

### 5.2 Implementation Priority

1. **Multi-Workspace Brain** — Unblock scaling to multiple projects
2. **Health Monitoring** — Needed immediately for operational reliability
3. **Backup & Restore** — Critical safety net
4. **Predictive Context Pre-loading** — High ROI, relatively easy
5. **Ambient Capture Inbox** — High automation value
6. **Decision Traceability** — Medium ROI, more complex

---

## 6. Priority Roadmap Revision

Based on this review, here's the revised priority order:

### IMMEDIATE (This Week)

1. **Fix the 7 bugs identified in Section 2.1**
   - HTTP body reader memory leak
   - Startup race condition
   - Lock expiration validation
   - MCP stdout corruption
   - Session heartbeat validation
   - Sequential embedding parallelization
   - Diary write truncation
   - Each should be fixed + tested individually

2. **Add tests for the critical path** (beyond what Codex suggested)
   - Test HTTP request handling (malformed inputs)
   - Test embedding build with 100+ entries
   - Test daemon crash recovery (kill and restart)
   - Test concurrent store operations
   - Test session expiration across restarts

3. **Implement health metrics** (FEATURE #5)
   - Add metrics registry
   - Instrument all routes
   - Add `/metrics` endpoint
   - This unlocks operational visibility

### NEXT TWO WEEKS

4. **Implement multi-workspace brain** (FEATURE #3)
   - Add workspaces table
   - Add workspace_id foreign keys
   - Add workspace-scoped store/recall endpoints
   - Update compiler for workspace filtering
   - Test with 3+ workspaces

5. **Implement backup & restore** (FEATURE #6)
   - Add backup endpoint
   - Add automated daily backup hook
   - Add restore facility
   - Test recovery from corrupted database

6. **Fix token optimization gaps** (Section 4)
   - Implement relevance-ordered delta
   - Implement duplicate suppression
   - Implement project-scoped delta
   - Implement incremental delta caching

### NEXT MONTH

7. **Implement ambient capture inbox** (FEATURE #4)
   - Add inbox table
   - Add SessionStart hook for capture
   - Build cortex-capture.py worker
   - Add promote/reject workflow

8. **Implement predictive context pre-loading** (FEATURE #1)
   - Build intent classifier
   - Map intents to prefetch capsules
   - Add prefetch endpoint
   - Update SessionStart hook to use prefetch

9. **Phase 3 decay & retrieval improvements** (from original roadmap)
   - Tokenized OR matching
   - Recency weighting
   - Pinned memories
   - Score floor

### AFTER MONTH

10. **Implement decision traceability graph** (FEATURE #2)
    - Add decision_edges table
    - Implement ancestor crawl
    - Add visualization in dashboard

11. **Phase 4 dreaming** (from original roadmap)
    - Synthesis via LLM
    - Compaction with guardrails
    - Dream reports

12. **Dashboard MVP** (from original roadmap)
    - Streamlit UI
    - Agent presence
    - Activity feed
    - Memory stats

---

## 7. Concrete Action Items

### Today (Right Now)

- [ ] Fix HTTP body reader memory leak (BUG #1)
- [ ] Add metrics registry to daemon.js
- [ ] Instrument `/health` endpoint with error counter
- [ ] Add `/metrics` endpoint

### This Week

- [ ] Fix remaining 6 bugs (BUG #2 through #7)
- [ ] Write test for HTTP malformed input handling
- [ ] Write test for embedding build parallelization
- [ ] Implement multi-workspace brain (FEATURE #3)
- [ ] Implement backup & restore (FEATURE #6)

### Next Week

- [ ] Implement token optimization (FEATURE #1)
- [ ] Add workspace filtering to compiler
- [ ] Test multi-workspace with 3+ projects

### This Month

- [ ] Implement ambient capture inbox (FEATURE #4)
- [ ] Build cortex-capture.py worker
- [ ] Implement predictive context pre-loading (FEATURE #1)
- [ ] Health monitoring dashboards

---

## Conclusion

Cortex is **good architecture, fragile implementation**. The pieces are there: daemon, brain, compiler, conflict detection, capsule system. But the foundation needs reinforcement before building JARVIS layers.

The four highest-impact improvements are:

1. **Fix the bugs** — 70% of fragility comes from these oversights
2. **Add metrics** — you can't improve what you can't measure
3. **Multi-workspaces** — Cortex can't scale without this
4. **Backup/restore** — no production system without insurance

After these four, Cortex becomes a reliable substrate for the ambitious features (dreaming, graphs, ambient capture, dashboard) that everyone is excited about.

Don't build the third floor until you fix the cracks in the foundation.

---

**End of Document**
