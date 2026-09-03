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

use cortex_daemon::aging;
use cortex_daemon::compaction;
use cortex_daemon::db::{initialize_schema, run_pending_migrations_quiet};
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
        assert!(failure.op.contains("decisions"), "failure must name its decisions op: {:?}", failure);
        assert!(failure.error.contains("denied by maintenance-errors test trigger"), "failure must carry the driver error: {:?}", failure);
    }
}

#[test]
fn aging_pass_missing_table_is_reported_not_success_shaped() {
    let conn = open_fresh_db();
    seed_memory(&conn, "fresh", 0.9);
    seed_decision(&conn, "fresh", 0.9);
    conn.execute_batch("DROP TABLE decisions;").expect("drop decisions table");

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
        assert!(failure.op.contains("decisions"), "failure must name its decisions op: {:?}", failure);
        assert!(failure.error.contains("no such table"), "failure must carry the driver error: {:?}", failure);
    }
    // The surviving memories path must still do and report its real work.
    assert_eq!(report.compressed, 1, "memories compression must still be counted");
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

    assert!(report.failures.is_empty(), "healthy db must produce zero failures: {:?}", report.failures);
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
        failure.error.contains("denied by maintenance-errors test trigger"),
        "failure must carry the driver error, got: {:?}", failure
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

    assert!(result.failures.is_empty(), "healthy db must produce zero failures: {:?}", result.failures);
    assert_eq!(result.co_occurrence_pruned, 2, "both singleton pairs must be pruned");
}
