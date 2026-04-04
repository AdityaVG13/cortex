### Features
- Over-fetch-then-filter recall with visibility filtering (`raw_k=max(k*5,50)`, max 2 retries)
- 13 admin endpoints for user/team/data management (`/admin/user/*`, `/admin/team/*`, `/admin/stats`, etc.)
- CLI commands: `cortex user add/rotate-key/remove/list`, `cortex team create/add/remove/list`, `cortex admin stats/list-unowned/assign-owner`
- Solo-to-team migration enhancement: pre-migration backup, per-table row counts, interactive owner prompt, `cortex migrate --dry-run`
- Graceful degradation across 7 failure scenarios: ONNX fallback with `degraded_mode` flag, team-mode MCP fail-closed, write-ahead buffer for offline stores, TLS solo/team mode distinction

### Security
- `ensure_auth_with_caller` combines auth + identity resolution in single argon2 pass
- `RecallContext` threads visibility through entire recall pipeline (semantic, keyword, crystal, budget)
- Role-based admin auth via `ensure_admin` (owner/admin required)
- Table name allowlists prevent SQL injection in dynamic admin queries

### Fixes
- Decision search in retry loop used hardcoded limit instead of `fts_limit`
- Fallback recall paths returned NULL `owner_id` causing visibility issues
- MCP handlers bypassed visibility by hardcoding solo context
- `handle_user_add` used re-query instead of `last_insert_rowid()`
- Removed `tasks` from archive/visibility allowlists (uses `task_id` TEXT, not `id`)

### Known Issues
- `/unfold` endpoint has no visibility filtering (root cause fix pending)
- MCP JSON-RPC lacks per-caller identity (uses default owner for all callers)
- `is_visible` treats NULL `owner_id` as visible in team mode (should fail closed after migration)
- Team-mode test environment needed to validate end-to-end
