mod connection;
mod maintenance;
mod migrations;
mod schema;
mod team;
#[cfg(test)]
mod tests;
pub(crate) use connection::*;
pub use connection::{configure, open, sqlite_vec_status, RepairError, RepairResult, SQLITE_BUSY_TIMEOUT_MS};
pub(crate) use maintenance::*;
pub use maintenance::{
    auto_repair, checkpoint_wal_best_effort, delete_expired_entries, migrate_focus_table, quick_check, rebuild_fts, rebuild_fts_if_needed, reindex_fts,
    verify_integrity,
};
pub use migrations::{
    applied_migration_versions, current_schema_user_version, migration_definitions, pending_migration_versions, run_pending_migrations,
    run_pending_migrations_quiet,
};
pub use schema::initialize_schema;
pub(crate) use team::*;
pub use team::{
    create_team_mode_tables, current_mode, ensure_default_team_membership, is_team_mode, migrate_to_team_mode, migration_counts, table_exists,
    upsert_owner_user,
};
