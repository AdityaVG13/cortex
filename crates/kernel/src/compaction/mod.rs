mod archived;
mod crystals;
mod events;
mod feedback;
mod governor;
mod helpers;

mod types;
pub use archived::*;
pub use crystals::*;
pub use events::*;
pub use feedback::*;
pub use governor::*;
pub use governor::{
    purge_benchmark_artifacts, run_compaction, run_compaction_governor,
    run_compaction_governor_startup, BenchmarkPurgeResult, MaintenanceFailure,
};
pub use helpers::storage_breakdown;
/// Cold-move pass with an explicit retention window (contract seam).
pub fn strip_archived_text_with_retention_for_test(
    conn: &rusqlite::Connection,
    failures: &mut Vec<MaintenanceFailure>,
    retention_days: i64,
) -> usize {
    archived::strip_archived_text_with_retention(conn, failures, retention_days)
}
pub use helpers::*;
pub use types::*;
