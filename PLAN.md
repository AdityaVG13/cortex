# Cortex v2 -- Opus Tasks Implementation Plan

## Phase Ordering

| Phase | Stream | Tasks | Depends On | Files |
|-------|--------|-------|------------|-------|
| 1 | Over-fetch recall | #66 | None | 3 |
| 2 | Admin endpoints | #71 | Phase 1 (resolve_caller) | 4 |
| 3 | CLI commands | #72 | Phase 2 (admin API) | 2 |
| 4 | Migration enhancement | #74 | Phase 2 (team tables) | 3 |
| 5 | Graceful degradation | #4 | Phases 1-4 stable | 5 |
| 6 | Chrome extension | #9, #40 | Phases 1-5 (separate crew) | N/A |

## Critical Discovery

`ensure_auth` in `handlers/mod.rs:66` validates tokens but **discards which user matched**. `token_matches_state` checks hashes but never returns the user_id. No handler knows WHO is calling -- only that someone valid is calling. Phase 1 must fix this first.

---

## Phase 1: Over-fetch recall with visibility filtering (#66)

**Files (3):** `handlers/recall.rs`, `handlers/mod.rs`, `crystallize.rs`
**Commit:** `feat(recall): add over-fetch-then-filter with visibility`

### Changes

1. **`handlers/mod.rs`** -- New `resolve_caller_id(headers, state) -> Option<i64>`
   - Solo mode: returns None (no filtering)
   - Team mode: finds which (user_id, hash) matches the Bearer token
   - This is the missing link -- currently auth succeeds but caller identity is lost

2. **`handlers/recall.rs`** -- Modify `run_recall_with_engine` (line 315)
   - New struct: `RecallContext { caller_id: Option<i64>, team_mode: bool }`
   - Semantic scan (lines 360-423): SELECT adds `owner_id, visibility`. Visibility filter: owner match, shared, or team membership
   - Over-fetch: `raw_k = max(k*5, 50)` instead of `k`. If < k results survive visibility filter, retry with `raw_k * 2` (max 2 retries)
   - FTS5 search: same visibility filtering via WHERE clause
   - Update all callers: handle_recall, handle_peek, handle_budget_recall, execute_unified_recall

3. **`crystallize.rs`** -- Add visibility to crystal search (line 517)

### Acceptance Criteria
- Solo mode: identical behavior (no regression)
- Team mode: private memories invisible to other users
- Over-fetch retry triggers when top-k is dominated by unauthorized results
- Latency < 15ms worst case (3 scans at 200 vectors)

### Tests
- Unit: in-memory DB, 2 users, mixed visibility, verify filtering
- Unit: over-fetch retry when k=10 but only 3 of top-50 are visible
- Regression: `cargo test` passes unchanged

---

## Phase 2: Admin endpoints (#71)

**Files (4):** `handlers/admin.rs` (NEW), `handlers/mod.rs`, `server.rs`, `state.rs`
**Commit:** `feat(admin): add user/team management endpoints`

### Changes

1. **`handlers/mod.rs`** -- New `ensure_admin(headers, state) -> Result<i64, Response>`
   - Calls ensure_auth + resolve_caller_id
   - Checks role is owner/admin via DB query
   - Returns 403 in solo mode or for non-admin users

2. **`handlers/admin.rs`** (NEW, ~400 lines) -- 13 endpoints:
   - User: add, rotate-key, remove, list
   - Team: create, add-member, remove-member, list
   - Data: unowned, assign-owner, set-visibility, archive, stats

3. **`server.rs`** -- Register 13 admin routes after line ~108

4. **`state.rs`** -- Change `team_api_key_hashes` to `Arc<RwLock<Vec<(i64, String)>>>` for runtime mutation

### Acceptance Criteria
- All 13 endpoints return 403 in solo mode
- All 13 endpoints return 403 for member-role users
- `user add` returns working API key
- `user rotate-key` invalidates old key
- Existing routes unaffected

---

## Phase 3: CLI commands (#72)

**Files (2):** `main.rs`, `handlers/admin.rs` (minor response types)
**Commit:** `feat(cli): add user/team/admin management commands`

### Changes

1. **`main.rs`** -- New match arms after line 153:
   - `cortex user add|rotate-key|remove|list`
   - `cortex team create|add|remove|list`
   - `cortex admin list-unowned|assign-owner|set-visibility|archive|stats`
   - Each calls daemon admin API via HTTP with confirmation prompts for destructive ops

### Acceptance Criteria
- `cortex user list` prints table of users
- `cortex user add testuser` prints ctx_ API key
- Destructive ops prompt for confirmation
- Clear errors when daemon not running or in solo mode

---

## Phase 4: Migration enhancement (#74)

**Files (3):** `setup.rs`, `db.rs`, `main.rs`
**Commit:** `feat(migration): enhance solo-to-team with backup, counts, dry-run`

### Changes

1. **`setup.rs`** -- Enhance `run_setup_team`:
   - Pre-migration backup: `cp cortex.db cortex.db.bak`
   - Interactive owner prompt
   - Print per-table assignment counts
   - Summary block

2. **`db.rs`** -- New `migration_counts(conn) -> Vec<(String, i64)>`

3. **`main.rs`** -- Add `cortex migrate` alias with `--dry-run`

### Acceptance Criteria
- Creates cortex.db.bak before changes
- Prints per-table row counts
- Idempotent on already-team database
- `--dry-run` shows counts without committing

---

## Phase 5: Graceful degradation (#4)

**Files (5):** `state.rs`, `server.rs`, `handlers/recall.rs`, `mcp_proxy.rs`, `tls.rs`
**Commit:** `fix(resilience): systematic graceful degradation across all layers`

### 7 Failure Scenarios

| # | Scenario | Status | Change |
|---|----------|--------|--------|
| 1 | ONNX failure | Partial | Add log + degraded flag in /health |
| 2 | Daemon crash (solo) | Works | Add CORTEX_STANDALONE_FALLBACK env var |
| 3 | Daemon crash (team) | WRONG -- falls back | Fail closed, refuse standalone |
| 4 | SQLite corruption | Complete | No changes needed |
| 5 | Network drop (team) | Fails hard | Write-ahead buffer + stale cache |
| 6 | Embedding download fail | Complete | Add /health field |
| 7 | TLS expired | Refuses always | Team: refuse. Solo: allow plain HTTP |

---

## Phase 6 (DEFERRED): Chrome extension (#9, #40)

Separate crew run. JS/TS project, different toolchain. Depends on Phases 1-5.
Only daemon prereq: CORS for `chrome-extension://` origins (added in Phase 2).

---

## Risks

1. **RwLock on team_api_key_hashes** -- adds locking to every auth'd request. Mitigated: std::sync::RwLock, <1us critical section, read-heavy.
2. **Over-fetch at scale** -- full cosine scan works at 200 vectors, not 10K+. Future: HNSW index.
3. **Write buffer durability** -- JSONL risks data loss on crash. Mitigated: O_APPEND + fsync.
4. **Admin security** -- single `ensure_admin` chokepoint + unit tests for 403 on non-admin.
