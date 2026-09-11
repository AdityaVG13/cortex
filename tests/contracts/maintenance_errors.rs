//! HYP-014 (cortex-5tm) fault contracts: destructive maintenance paths
//! (compaction DELETE/VACUUM, aging UPDATE/ARCHIVE/GC) must be honest about
//! failure. A failed purge/maintenance op must be REPORTABLE (structured
//! per-op failure records) and must never be counted as successful work.
//!
//! Faults (all deterministic, no product hooks needed):
//! - RAISE(ABORT) trigger on `decisions` denies every aging UPDATE while the
//!   SELECT still returns rows: discriminates "counts denied mutations as
//!   work with no failure report" (the pre-fix behavior: compressed=2) from
//!   honest counting plus a failure report.
//! - DROP TABLE `decisions` makes every decisions-targeted op fail with
//!   "no such table": discriminates success-shaped output from a structured
//!   failure report (pre-fix the pass returned (0, 0) as if healthy).
//! - RAISE(ABORT) trigger on `co_occurrence` denies the singleton-prune
//!   DELETE: same discrimination for the compaction path.
//!
//! Healthy-DB companions pin the no-false-positive side: zero failures and
//! exact counts when every op succeeds. Note: `AgingReport.failures` /
//! `CompactionResult.failures` did not exist before this contract — the
//! compile failure against pre-fix code was the first red for the reporting
//! channel itself.
//!
//! HYP-014b (cortex-z4x) extends the same contracts to the remaining
//! compaction submodules: events prunes/rollups + wal_checkpoint, archived
//! text strips + expired-row DELETEs, orphan cluster-member DELETEs, and the
//! benchmark purge (whose `BenchmarkPurgeResult.failures` channel is again
//! compile-error red against pre-fix code; failures there are additive
//! visibility and never change what the purge counts).

use cortex_kernel::aging;
use cortex_kernel::compaction;
use cortex_kernel::db::{initialize_schema, run_pending_migrations_quiet};
use rusqlite::Connection;

fn open_fresh_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    initialize_schema(&conn).expect("initialize base schema");
    run_pending_migrations_quiet(&conn);
    conn
}

const OLD_TS: &str = "2010-01-01 00:00:00";

fn seed_memory(conn: &Connection, tier: &str, score: f64) {
    conn.execute(
        "INSERT INTO memories (text, source, status, pinned, age_tier, score, created_at, updated_at, last_accessed) \
         VALUES ('Memory text. must preserve the architecture decision here.', 'maintenance-errors-test', 'active', 0, ?1, ?2, ?3, ?3, ?3)",
        rusqlite::params![tier, score, OLD_TS],
    )
    .expect("seed memory");
}

fn seed_decision(conn: &Connection, tier: &str, score: f64) {
    conn.execute(
        "INSERT INTO decisions (decision, context, status, pinned, age_tier, score, created_at, updated_at, last_accessed) \
         VALUES ('Decision text.', 'ctx', 'active', 0, ?1, ?2, ?3, ?3, ?3)",
        rusqlite::params![tier, score, OLD_TS],
    )
    .expect("seed decision");
}

fn deny_all_decision_updates(conn: &Connection) {
    conn.execute_batch(
        "CREATE TRIGGER deny_decision_updates BEFORE UPDATE ON decisions \
         BEGIN SELECT RAISE(ABORT, 'denied by maintenance-errors test trigger'); END;",
    )
    .expect("create deny trigger");
}

fn deny_table_deletes(conn: &Connection, table: &str) {
    conn.execute_batch(&format!(
        "CREATE TRIGGER deny_{table}_deletes BEFORE DELETE ON {table} \
         BEGIN SELECT RAISE(ABORT, 'denied by maintenance-errors test trigger'); END;",
    ))
    .expect("create deny trigger");
}

fn seed_old_event(conn: &Connection, event_type: &str) {
    conn.execute(
        "INSERT INTO events (type, data, source_agent, created_at) VALUES (?1, '{}', 'maintenance-errors-test', ?2)",
        rusqlite::params![event_type, OLD_TS],
    )
    .expect("seed old event");
}

#[test]
fn aging_pass_denied_updates_are_reported_and_not_counted_as_work() {
    let conn = open_fresh_db();
    // memories succeed: 1 fresh->recent compress, 2 archives (old tier).
    seed_memory(&conn, "fresh", 0.9);
    seed_memory(&conn, "old", 0.9);
    seed_memory(&conn, "old", 0.01);
    // decisions all denied by trigger: 1 compress, 2 archives would-be.
    seed_decision(&conn, "fresh", 0.9);
    seed_decision(&conn, "old", 0.9);
    seed_decision(&conn, "old", 0.01);
    deny_all_decision_updates(&conn);

    let report = aging::run_aging_pass(&conn);

    // Honesty contract: only mutations that actually committed count as work.
    assert_eq!(
        report.compressed, 1,
        "aging pass counted denied decisions UPDATEs as compressed work"
    );
    assert_eq!(
        report.archived, 2,
        "aging pass counted denied decisions UPDATEs as archived work"
    );
    // And the denials must be reported, naming the op and the driver error.
    assert_eq!(
        report.failures.len(),
        3,
        "expected one failure per denied decisions UPDATE (compress, archive, gc), got {:?}",
        report.failures
    );
    for failure in &report.failures {
        assert!(
            failure.op.contains("decisions"),
            "failure must name its decisions op: {:?}",
            failure
        );
        assert!(
            failure
                .error
                .contains("denied by maintenance-errors test trigger"),
            "failure must carry the driver error: {:?}",
            failure
        );
    }
}

#[test]
fn aging_pass_missing_table_is_reported_not_success_shaped() {
    let conn = open_fresh_db();
    seed_memory(&conn, "fresh", 0.9);
    seed_decision(&conn, "fresh", 0.9);
    conn.execute_batch("DROP TABLE decisions;")
        .expect("drop decisions table");

    let report = aging::run_aging_pass(&conn);

    // Pre-fix this pass returned (0, 0) — indistinguishable from a healthy
    // no-op. Every decisions-targeted op (2 tier SELECTs, archive, gc) must
    // now be reported.
    assert!(
        report.failures.len() >= 4,
        "success-shaped output despite destroyed decisions table; got {:?}",
        report.failures
    );
    for failure in &report.failures {
        assert!(
            failure.op.contains("decisions"),
            "failure must name its decisions op: {:?}",
            failure
        );
        assert!(
            failure.error.contains("no such table"),
            "failure must carry the driver error: {:?}",
            failure
        );
    }
    // The surviving memories path must still do and report its real work.
    assert_eq!(
        report.compressed, 1,
        "memories compression must still be counted"
    );
    assert_eq!(report.archived, 0, "no archive candidates seeded");
}

#[test]
fn aging_pass_healthy_db_has_zero_failures_and_exact_counts() {
    let conn = open_fresh_db();
    seed_memory(&conn, "fresh", 0.9);
    seed_memory(&conn, "old", 0.9);
    seed_decision(&conn, "fresh", 0.9);
    seed_decision(&conn, "old", 0.9);

    let report = aging::run_aging_pass(&conn);

    assert!(
        report.failures.is_empty(),
        "healthy db must produce zero failures: {:?}",
        report.failures
    );
    assert_eq!(report.compressed, 2, "one memory + one decision compress");
    assert_eq!(report.archived, 2, "one memory + one decision archive");
}

#[test]
fn compaction_failed_singleton_delete_is_reported_and_not_counted() {
    let conn = open_fresh_db();
    conn.execute_batch(
        "INSERT INTO co_occurrence (source_a, source_b, count) VALUES ('a','b',1), ('c','d',1), ('e','f',5);",
    )
    .expect("seed co_occurrence");
    conn.execute_batch(
        "CREATE TRIGGER deny_cooccur_deletes BEFORE DELETE ON co_occurrence \
         BEGIN SELECT RAISE(ABORT, 'denied by maintenance-errors test trigger'); END;",
    )
    .expect("create deny trigger");

    let result = compaction::run_compaction(&conn);

    // The DELETE references "count", which no longer exists: it must fail and
    // be reported, never counted as pruned rows.
    assert_eq!(
        result.co_occurrence_pruned, 0,
        "a failed DELETE must never be reported as pruned rows"
    );
    let failure = result
        .failures
        .iter()
        .find(|f| f.op.contains("co_occurrence"))
        .expect("run_compaction must report the failed co_occurrence prune in result.failures");
    assert!(
        failure
            .error
            .contains("denied by maintenance-errors test trigger"),
        "failure must carry the driver error, got: {:?}",
        failure
    );
}

#[test]
fn compaction_healthy_pass_has_zero_failures_and_exact_prune_count() {
    let conn = open_fresh_db();
    conn.execute_batch(
        "INSERT INTO co_occurrence (source_a, source_b, count) VALUES ('a','b',1), ('c','d',1), ('e','f',5);",
    )
    .expect("seed co_occurrence");

    let result = compaction::run_compaction(&conn);

    assert!(
        result.failures.is_empty(),
        "healthy db must produce zero failures: {:?}",
        result.failures
    );
    assert_eq!(
        result.co_occurrence_pruned, 2,
        "both singleton pairs must be pruned"
    );
}

#[test]
fn compaction_events_prune_failures_are_reported_and_not_counted() {
    let conn = open_fresh_db();
    // Old non-protected events give the retention prune real candidates; old
    // savings events route through the savings-rollup DELETE as well.
    for _ in 0..3 {
        seed_old_event(&conn, "diary_write");
    }
    for _ in 0..3 {
        seed_old_event(&conn, "recall_query");
    }
    deny_table_deletes(&conn, "events");

    let result = compaction::run_compaction(&conn);

    // Honesty contract: every denied events DELETE must be neither performed
    // nor claimed. (events_pruned itself also legitimately counts
    // event_savings_rollups-table deletes, so the row count is the precise
    // "not counted" check here, not the aggregate.)
    let surviving: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type IN ('diary_write', 'recall_query')",
            [],
            |row| row.get(0),
        )
        .expect("count surviving events");
    assert_eq!(
        surviving, 6,
        "denied events DELETEs must leave all seeded rows in place"
    );
    let denied: Vec<_> = result
        .failures
        .iter()
        .filter(|f| {
            f.error
                .contains("denied by maintenance-errors test trigger")
        })
        .collect();
    assert!(
        denied.len() >= 2,
        "every failed events DELETE (rollup + retention prune) must be reported, got {:?}",
        result.failures
    );
    for failure in &denied {
        assert!(
            failure.op.contains("DELETE events"),
            "failure must name its events DELETE op: {:?}",
            failure
        );
    }
    assert!(
        denied
            .iter()
            .any(|f| f.op.contains("rollup_old_savings_events")),
        "the savings-rollup events DELETE failure must be reported, got {:?}",
        result.failures
    );
    assert!(
        denied.iter().any(|f| f.op.contains("prune_old_events")),
        "the retention events DELETE failure must be reported, got {:?}",
        result.failures
    );
}

#[test]
fn compaction_archived_and_expired_failures_are_reported_and_not_counted() {
    let conn = open_fresh_db();
    // An expired memory whose DELETE will be denied...
    conn.execute(
        "INSERT INTO memories (text, source, status, pinned, age_tier, score, created_at, updated_at, last_accessed, expires_at) \
         VALUES ('Expired text.', 'maintenance-errors-test', 'active', 0, 'fresh', 0.5, ?1, ?1, ?1, '2011-01-01 00:00:00')",
        rusqlite::params![OLD_TS],
    )
    .expect("seed expired memory");
    // ...and an ancient archived memory whose text-strip UPDATE will be denied.
    conn.execute(
        "INSERT INTO memories (text, source, status, pinned, age_tier, score, created_at, updated_at, last_accessed) \
         VALUES ('Ancient archived text.', 'maintenance-errors-test', 'archived', 0, 'ancient', 0.5, ?1, ?1, ?1)",
        rusqlite::params![OLD_TS],
    )
    .expect("seed archived memory");
    deny_table_deletes(&conn, "memories");
    conn.execute_batch(
        "CREATE TRIGGER deny_memory_updates BEFORE UPDATE ON memories \
         BEGIN SELECT RAISE(ABORT, 'denied by maintenance-errors test trigger'); END;",
    )
    .expect("create deny trigger");

    let result = compaction::run_compaction(&conn);

    assert_eq!(
        result.expired_pruned, 0,
        "a denied expired-row DELETE must never be counted as pruned"
    );
    assert_eq!(
        result.archived_text_stripped, 0,
        "a denied strip UPDATE must never be counted as stripped"
    );
    let strip = result
        .failures
        .iter()
        .find(|f| f.op.contains("strip_archived_text") && f.op.contains("memories"))
        .expect("denied strip_archived_text UPDATE must be reported in result.failures");
    assert!(
        strip
            .error
            .contains("denied by maintenance-errors test trigger"),
        "failure must carry the driver error, got: {:?}",
        strip
    );
    let expired = result
        .failures
        .iter()
        .find(|f| f.op.contains("prune_expired_entries") && f.op.contains("memories"))
        .expect("denied expired-memory DELETE must be reported in result.failures");
    assert!(
        expired
            .error
            .contains("denied by maintenance-errors test trigger"),
        "failure must carry the driver error, got: {:?}",
        expired
    );
}

#[test]
fn compaction_orphan_cluster_member_failures_are_reported_and_not_counted() {
    let conn = open_fresh_db();
    // Two orphan rows: one pointing at a missing memory, one of an unknown
    // target_type; both also hang off a missing cluster. The orphan-memory,
    // unknown-type, and orphan-cluster DELETEs each match rows and get denied.
    conn.execute_batch(
        "INSERT INTO cluster_members (cluster_id, source, target_type, target_id) VALUES \
         (999999, 'maintenance-errors-test', 'memory', 424242), \
         (999999, 'maintenance-errors-test', 'widget', 424243);",
    )
    .expect("seed orphan cluster members");
    deny_table_deletes(&conn, "cluster_members");

    let result = compaction::run_compaction(&conn);

    assert_eq!(
        result.cluster_members_pruned, 0,
        "denied orphan DELETEs must never be counted as pruned"
    );
    let denied: Vec<_> = result
        .failures
        .iter()
        .filter(|f| {
            f.error
                .contains("denied by maintenance-errors test trigger")
        })
        .collect();
    assert!(
        denied.len() >= 2,
        "each failing orphan-cluster_members DELETE must be reported, got {:?}",
        result.failures
    );
    for failure in &denied {
        assert!(
            failure.op.contains("prune_orphan_cluster_members")
                && failure.op.contains("cluster_members"),
            "failure must name the orphan cluster_members prune op: {:?}",
            failure
        );
    }
}

fn seed_benchmark_decision(conn: &Connection) {
    conn.execute(
        "INSERT INTO decisions (decision, context, type, source_agent, status, score, created_at, updated_at, last_accessed) \
         VALUES ('Benchmark decision.', 'ctx', 'benchmark', 'amb-cortex-maintenance-errors', 'active', 1.0, ?1, ?1, ?1)",
        rusqlite::params![OLD_TS],
    )
    .expect("seed benchmark decision");
}

#[test]
fn benchmark_purge_denied_decision_deletes_are_reported_additively() {
    let conn = open_fresh_db();
    for _ in 0..2 {
        seed_benchmark_decision(&conn);
    }
    deny_table_deletes(&conn, "decisions");

    let result = compaction::purge_benchmark_artifacts(&conn);

    // Honesty nuance: `failures` is additive visibility only. The purge still
    // counts exactly the deletions that committed, so its pass/fail semantics
    // are unchanged — a denied DELETE is simply no longer indistinguishable
    // from "nothing matched".
    assert_eq!(
        result.decisions_deleted, 0,
        "a denied DELETE must never be counted as deleted"
    );
    assert_eq!(
        result.total_deleted(),
        0,
        "denied purge ops must not inflate total_deleted"
    );
    assert_eq!(
        result.failures.len(),
        1,
        "exactly the denied benchmark decisions DELETE must be reported, got {:?}",
        result.failures
    );
    let failure = &result.failures[0];
    assert!(
        failure.op.contains("benchmark_purge") && failure.op.contains("DELETE decisions"),
        "failure must name the benchmark decisions DELETE op: {:?}",
        failure
    );
    assert!(
        failure
            .error
            .contains("denied by maintenance-errors test trigger"),
        "failure must carry the driver error, got: {:?}",
        failure
    );
}

#[test]
fn benchmark_purge_healthy_has_zero_failures_and_exact_counts() {
    let conn = open_fresh_db();
    for _ in 0..2 {
        seed_benchmark_decision(&conn);
    }

    let result = compaction::purge_benchmark_artifacts(&conn);

    assert!(
        result.failures.is_empty(),
        "healthy purge must produce zero failures: {:?}",
        result.failures
    );
    assert_eq!(
        result.decisions_deleted, 2,
        "both benchmark decisions must be purged"
    );
    assert_eq!(
        result.total_deleted(),
        2,
        "total must count exactly the committed deletions"
    );
}
