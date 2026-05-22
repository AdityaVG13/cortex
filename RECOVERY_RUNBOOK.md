# Recovery Runbook

## Symptoms
- `cortex doctor` reports a failed SQLite integrity check.
- `cortex sync import` rejects a JSON changeset.
- `write_buffer.jsonl` contains pending MCP requests after daemon downtime.
- Sync cursor or seen state no longer matches files present in a watch directory.

## Steps
1. Stop app/plugin-managed Cortex clients so no new writes arrive.
2. Run `cortex doctor` against the target `CORTEX_DB`.
3. If the DB is healthy, export a fresh snapshot with `cortex export --format json --out <path>`.
4. If the DB is unhealthy, restore the newest valid `backups/cortex-*.db` or a prior JSON export, then rerun `cortex doctor`.
5. For sync directories, validate incoming changesets with `cortex sync import --file <path>` before marking them seen.
6. Resume `cortex sync watch --dir <path>` and confirm the cursor advances after a successful export/import pass.

## Commands
- `cortex doctor`
- `cortex export --format json --out cortex-export.json`
- `cortex import --file cortex-export.json`
- `cortex sync export --out changeset.json --cursor-file sync.cursor`
- `cortex sync import --file changeset.json`

## Notes
- SQLite is the source of truth; exported JSON and sync changesets are recovery/interchange artifacts.
- Do not edit `cortex.db` directly. Edit an export or changeset, validate JSON, then import.
- Old artifact files should remain readable after interrupted writes because sync artifacts are atomically replaced.
