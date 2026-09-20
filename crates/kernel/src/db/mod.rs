mod connection;
mod maintenance;
mod migrations;
mod schema;
mod team;

pub use connection::*;
pub use maintenance::*;
pub use migrations::{
    applied_migration_versions, current_schema_user_version, ensure_schema_migrations_table,
    migration_definitions, pending_migration_versions, run_pending_migrations,
    run_pending_migrations_quiet,
};
pub use schema::initialize_schema;

pub const DECISION_OBSERVATION_EVIDENCE_DDL: &str = "CREATE TABLE IF NOT EXISTS decision_observation_evidence (decision_id INTEGER NOT NULL, source_id TEXT NOT NULL, principal TEXT NOT NULL, source_key TEXT NOT NULL, role TEXT NOT NULL, relationship TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY(decision_id, source_id));";
pub mod addresses;
pub mod backup;
pub mod capture_policy;
pub mod cold;
pub mod compiled;
pub mod erasure;
pub mod feedback_ledger;
pub mod outbox;
mod promote_policy;
pub mod promotion;
pub mod query_memory;
pub mod records;
pub mod replication;
pub mod term_bridges;
pub mod threads;
pub mod views;
pub use team::*;

pub(crate) use cortex_logic::protocol::{
    ACTIVE_TEMPORAL_SQL, CREATED_UPDATED_STAMP_SQL, EXPIRED_SQL, LAST_ACCESSED_CREATED_STAMP_SQL,
    LAST_TOUCHED_STAMP_SQL, TEMPORAL_BOUNDS_SQL, UNORPHANED_VERSION_SQL, UPDATED_CREATED_STAMP_SQL,
    VALIDITY_START_SQL, active_temporal_sql, like_contains, like_escape, like_prefix,
    optional_coalesce_like_sql, temporal_bounds_sql_at, unorphaned_version_sql, validity_start_sql,
};
