mod connection;
mod maintenance;
mod migrations;
mod schema;
mod team;

pub use connection::*;
pub use connection::{
    configure, configure_with_profile, open, sqlite_vec_status, sqlite_version,
    sqlite_wal_reset_fixed, DurabilityProfile, RepairError, RepairResult, SQLITE_BUSY_TIMEOUT_MS,
};
pub use maintenance::*;
pub use maintenance::{
    auto_repair, checkpoint_wal_best_effort, delete_expired_entries, migrate_focus_table,
    quick_check, rebuild_fts, rebuild_fts_if_needed, reindex_fts, verify_integrity,
};
pub use migrations::{
    applied_migration_versions, current_schema_user_version, migration_definitions,
    pending_migration_versions, run_pending_migrations, run_pending_migrations_quiet,
};
pub use schema::initialize_schema;
pub mod addresses;
pub mod backup;
pub mod capture_policy;
pub mod cold;
pub mod compiled;
pub mod erasure;
pub mod feedback_ledger;
pub mod outbox;
pub mod promotion;
pub mod records;
pub mod replication;
pub mod threads;
pub mod views;
pub use team::*;
pub use team::{
    create_team_mode_tables, current_mode, ensure_default_team_membership, is_team_mode,
    migrate_to_team_mode, migration_counts, table_exists, upsert_owner_user,
};
