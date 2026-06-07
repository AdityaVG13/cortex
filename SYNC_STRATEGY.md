# Sync Strategy

## Source of Truth
- Primary: SQLite (`~/.cortex/cortex.db`, override with `CORTEX_DB`)
- Rationale: Cortex serves reads and writes from SQLite for local ACID behavior, indexing, FTS, retention, and migration control. File exports are backup/interchange artifacts, not an independent writable source.

## Sync Triggers
- On command: `cortex export --format json|sql`, `cortex import --file <path>`, `cortex sync export --out <path>`, `cortex sync import --file <path>`.
- On watch: `cortex sync watch --dir <path>` imports remote changesets, exports local changesets, and advances a cursor file.
- On exit: daemon shutdown checkpoints WAL; it does not export a file snapshot automatically.
- Timer/throttle: `sync watch` uses `--interval-seconds` and cursor-based changesets instead of per-record file writes.

## Versioning
- DB marker: schema state is tracked with `schema_migrations` and `PRAGMA user_version`; sync changesets use `cursor` timestamps from the DB snapshot.
- File marker: sync export files contain `version`, `mode`, `exported_at`, `since`, and `cursor`; watch state uses a local `site_id`, cursor file, and seen-file set.
- Import checks: `cortex sync import` and `sync watch` require `version = 1`, `mode = "changeset"`, a valid RFC3339 `cursor`, and matching `memories_count`/`decisions_count` markers before any DB write. Plain `cortex import` still accepts legacy unversioned full JSON, but rejects unsupported versioned exports and paged fragments.

## Concurrency
- Lock file path: daemon process ownership uses `~/.cortex/cortex.lock`; `cortex sync ...` commands serialize through `~/.cortex/sync.lock`.
- Busy timeout: `CORTEX_DB` connections use `SQLITE_BUSY_TIMEOUT_MS` (5000 ms).
- Snapshot policy: exports run inside a SQLite read transaction so memory and decision rows come from one consistent snapshot.
- Artifact writes: sync export, cursor, site-id, seen-state, and MCP `write_buffer.jsonl` writes use temp-file, fsync, and atomic replacement.

## Failure Handling
- DB locked: retry through SQLite busy timeout; if still locked, report and exit non-zero.
- JSON/JSONL parse or metadata error: skip or reject the file without advancing cursor/seen state.
- Export interrupted: the old export/cursor/state file remains in place because replacement happens only after temp-file fsync succeeds.
- DB corruption: run `cortex doctor`; if integrity fails, use the recovery runbook.
