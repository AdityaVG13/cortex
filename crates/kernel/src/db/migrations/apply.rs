use super::{add_twin_columns, ensure_quality_columns, require_columns, require_tables};
use crate::db::{migrate_aging_columns_with_logging, migrate_focus_table};
use rusqlite::Connection;

pub fn apply_migration_with_logging(
    conn: &Connection,
    version: &str,
    log_success: bool,
) -> rusqlite::Result<()> {
    match version {
        "001_initial_schema" => Ok(()),
        "002_aging_columns" => {
            migrate_aging_columns_with_logging(conn, log_success);
            require_columns(
                conn,
                &[
                    ("memories", "compressed_text"),
                    ("memories", "age_tier"),
                    ("decisions", "compressed_text"),
                    ("decisions", "age_tier"),
                ],
                "aging migration did not create expected columns",
            )
        }
        "003_focus_table" => {
            migrate_focus_table(conn);
            require_tables(
                conn,
                &["focus_sessions"],
                "focus table migration did not create focus_sessions",
            )
        }
        "004_crystal_tables" => {
            crate::crystallize::migrate_crystal_tables(conn);
            require_tables(
                conn,
                &["memory_clusters", "cluster_members"],
                "crystal migration did not create memory_clusters/cluster_members",
            )
        }
        "005_quality_dedup_columns" => ensure_quality_columns(conn),
        "006" => add_twin_columns(conn, &[("expires_at", "TEXT")]),
        "007" => ensure_quality_columns(conn),
        later => apply_later(conn, later, log_success),
    }
}
mod later;
use later::apply_later;
