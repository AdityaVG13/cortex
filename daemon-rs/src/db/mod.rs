// SPDX-License-Identifier: MIT
mod connection;
mod migrations;
mod schema;
mod team;
mod maintenance;

#[cfg(test)]
mod tests;

pub(crate) use connection::*;
pub(crate) use migrations::*;
pub(crate) use schema::*;
pub(crate) use team::*;
pub(crate) use maintenance::*;

pub use connection::{open, configure, sqlite_vec_status, SQLITE_BUSY_TIMEOUT_MS, SQLITE_WAL_AUTOCHECKPOINT_PAGES, RepairResult, RepairError};
pub use migrations::{
    migration_definitions, latest_schema_user_version, current_schema_user_version,
    set_schema_user_version, ensure_schema_migrations_table, applied_migration_versions,
    pending_migration_versions, run_pending_migrations, run_pending_migrations_quiet,
};
pub use schema::initialize_schema;
pub use team::{
    current_mode, is_team_mode, migration_counts, create_team_mode_tables, upsert_owner_user,
    migrate_to_team_mode, ensure_default_team_membership, table_exists,
};
pub use maintenance::{
    checkpoint_wal_best_effort, delete_expired_entries, ExpiredCleanupCounts, rebuild_fts,
    reindex_fts, rebuild_fts_if_needed, verify_integrity, quick_check, auto_repair,
    archive_entries_scoped, archive_entries, migrate_focus_table,
};
