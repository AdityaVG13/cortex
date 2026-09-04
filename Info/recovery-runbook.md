# Recovery Runbook

## Symptoms
- `cortex doctor` reports a failed SQLite integrity check.
- `cortex sync import` rejects a JSON changeset. *(CLI status: `cortex sync import` is not implemented in default builds — it exits 1 without reading any file; on default builds an import failure surfaces via the HTTP `POST /import` endpoint instead)*
- `write_buffer.jsonl` contains pending MCP requests after daemon downtime.
- Sync cursor or seen state no longer matches files present in a watch directory.

## Steps
1. Stop app/plugin-managed Cortex clients so no new writes arrive.
2. Run `cortex doctor` against the target `CORTEX_DB`.
3. If the DB is healthy, export a fresh snapshot with `cortex export --format json --out <path>`. *(not implemented in default builds — the command exits 1; use the daemon's HTTP `GET /export` endpoint (JSON format) to page out a snapshot instead)*
4. If the DB is unhealthy, restore the newest valid `backups/cortex-*.db` or a prior JSON export, then rerun `cortex doctor`.
5. For sync directories, validate incoming changesets with `cortex sync import --file <path>` before marking them seen; sync imports reject missing/unsupported version markers, non-changeset modes, invalid cursors, and count mismatches. *(not implemented in default builds — exits 1; this step applies only to feature-enabled builds)*
6. Resume `cortex sync watch --dir <path>` and confirm the cursor advances after a successful export/import pass. *(not implemented in default builds — exits 1; this step applies only to feature-enabled builds)*

## Commands
*(All `cortex export`, `cortex import`, and `cortex sync ...` commands below are NOT implemented in default builds — each exits 1 with an error. On default builds use the daemon's HTTP `GET /export` (JSON format) and `POST /import` endpoints, or the desktop app.)*
- `cortex doctor`
- `cortex export --format json --out cortex-export.json`
- `cortex import --file cortex-export.json`
- `cortex sync export --out changeset.json --cursor-file sync.cursor`
- `cortex sync import --file changeset.json`

## Notes
- SQLite is the source of truth; exported JSON and sync changesets are recovery/interchange artifacts.
- Do not edit `cortex.db` directly. Edit an export or changeset, validate JSON, then import.
- Old artifact files should remain readable after interrupted writes because sync artifacts are atomically replaced.
