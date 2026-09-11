//! Daemonless liveness: deposits enqueue durable jobs in the same commit;
//! no caller means no progress; a crashed claimant leaves its job; completion
//! is idempotent and generation-checked; debt is visible and hard debt
//! refuses intake; telemetry is pruned; brain health is semantic.

use cortex_kernel::db::outbox::{
    claim_next, complete, debt, enqueue_for_commit, maintain_slice, prune_telemetry,
    DEBT_HARD_LIMIT_JOBS, FEED_MAX_ROWS,
};
use cortex_kernel::db::records::append_commit;
use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_kernel::store_spi::sqlite::SqliteStore;
use cortex_kernel::store_spi::BrainStore;
use cortex_tests::support::{open_file_db, solo_state, test_conn};
use serde_json::json;

fn caller() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "outbox",
        principal: "solo".into(),
    }
}

#[test]
fn deposits_leave_durable_jobs_and_only_callers_drain_them() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        dispatch(
            cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "OUT-1 first"}),
        )
        .await
        .unwrap();
        let conn = state.db.lock(cx).await.unwrap();
        let d = debt(&conn);
        assert!(
            d.pending_jobs >= 2,
            "deposit enqueued projection jobs in its commit: {:?}",
            d
        );
        assert_eq!(d.earliest_unprojected_sequence, Some(1));
        // Nothing moves on its own.
        drop(conn);
        std::thread::sleep(std::time::Duration::from_millis(30));
        let conn = state.db.lock(cx).await.unwrap();
        assert_eq!(debt(&conn).pending_jobs, d.pending_jobs, "idle means idle");
        let slice = maintain_slice(&conn, "test", 10).unwrap();
        assert!(slice["jobs"].as_array().unwrap().len() >= 2, "{slice}");
        assert_eq!(debt(&conn).pending_jobs, 0);
        assert_eq!(debt(&conn).pressure(), "none");
    });
}

#[test]
fn crashed_claimant_leaves_the_job_and_completion_is_generation_checked() {
    let conn = test_conn();
    let seq = append_commit(&conn, "solo", None, "process_crash").unwrap();
    assert_eq!(
        enqueue_for_commit(&conn, seq, &["checkpoint_wal"], json!({})).unwrap(),
        1
    );
    assert_eq!(
        enqueue_for_commit(&conn, seq, &["checkpoint_wal"], json!({})).unwrap(),
        0,
        "idempotent per (sequence, kind)"
    );
    let first = claim_next(&conn, "worker-a").unwrap().expect("claim");
    assert_eq!(first.generation, 1);
    // Worker A crashes: lease still live, nobody else can claim.
    assert!(
        claim_next(&conn, "worker-b").unwrap().is_none(),
        "a live lease is exclusive"
    );
    // Simulate lease expiry.
    conn.execute(
        "UPDATE outbox SET lease_until = '2000-01-01T00:00:00Z' WHERE job_id = ?1",
        [&first.job_id],
    )
    .unwrap();
    let second = claim_next(&conn, "worker-b")
        .unwrap()
        .expect("reclaim after expiry");
    assert_eq!(second.generation, 2, "reclaim bumps the generation");
    assert!(
        !complete(&conn, &first).unwrap(),
        "the crashed claimant's stale generation cannot complete"
    );
    assert!(complete(&conn, &second).unwrap());
    assert!(
        complete(&conn, &second).unwrap(),
        "completion is idempotent"
    );
    assert_eq!(debt(&conn).pending_jobs + debt(&conn).claimed_jobs, 0);
}

#[test]
fn hard_debt_refuses_intake_with_an_actionable_error() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        {
            let conn = state.db.lock(cx).await.unwrap();
            cortex_kernel::db::records::ensure_authoritative_schema(&conn).unwrap();
            let seq = append_commit(&conn, "solo", None, "process_crash").unwrap();
            let mut stmt = conn.prepare("INSERT INTO outbox (job_id, commit_sequence, job_kind, state, generation, payload_json, attempts) VALUES (?1, ?2, ?3, 'pending', 0, '{}', 0)").unwrap();
            for i in 0..DEBT_HARD_LIMIT_JOBS {
                stmt.execute(rusqlite::params![
                    format!("job:flood:{i}"),
                    seq,
                    format!("flood_{i}")
                ])
                .unwrap();
            }
            assert_eq!(debt(&conn).pressure(), "hard");
        }
        let refused = dispatch(
            cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "OUT-2 under pressure"}),
        )
        .await
        .unwrap();
        assert_eq!(
            refused["status"], "unavailable",
            "backpressure is a retryable unavailable, not a client error: {refused}"
        );
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .contains("maintenance_debt"),
            "{refused}"
        );
        let conn = state.db.lock(cx).await.unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE decision LIKE 'OUT-2%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "refused intake writes nothing");
    });
}

#[test]
fn telemetry_is_pruned_as_its_own_retention_class() {
    let conn = test_conn();
    cortex_kernel::db::records::ensure_authoritative_schema(&conn).unwrap();
    for i in 0..(FEED_MAX_ROWS + 50) {
        conn.execute("INSERT INTO feed (id, agent, kind, summary, timestamp) VALUES (?1, 'a', 'note', 'x', ?2)", rusqlite::params![format!("f{i}"), format!("2026-09-05T00:00:{:02}.{:03}Z", (i / 1000) % 60, i % 1000)]).unwrap();
    }
    conn.execute("INSERT INTO feed (id, agent, kind, summary, timestamp) VALUES ('old', 'a', 'note', 'x', '2020-01-01T00:00:00Z')", []).unwrap();
    let pruned = prune_telemetry(&conn).unwrap();
    assert!(pruned >= 51, "{pruned}");
    let left: i64 = conn
        .query_row("SELECT COUNT(*) FROM feed", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, FEED_MAX_ROWS);
    let durable: i64 = conn
        .query_row("SELECT COUNT(*) FROM decisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        durable, 0,
        "durable tables are untouched by telemetry pruning"
    );
}

#[test]
fn health_reports_semantic_brain_state_and_spi_maintain_drains() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        dispatch(
            cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "OUT-3 health probe"}),
        )
        .await
        .unwrap();
        let payload = cortex_daemon::handlers::health::build_health_payload(cx, &state, false)
            .await
            .unwrap();
        let brain = &payload["brain"];
        for key in [
            "commit_durability",
            "recoverable_sources",
            "projection_lag",
            "maintenance_debt",
            "unresolved_heads",
            "open_contradictions",
            "last_verified_restore",
            "restore_epoch",
        ] {
            assert!(
                brain.get(key).is_some(),
                "brain health lacks {key}: {brain}"
            );
        }
        assert!(
            brain["projection_lag"].as_i64().unwrap() >= 1,
            "lag visible before a slice: {brain}"
        );
        let mut store = SqliteStore::new(open_file_db(&state.db_path)).unwrap();
        let ran = store.maintain_slice(10).unwrap();
        assert!(ran >= 1);
        let after = cortex_daemon::handlers::health::build_health_payload(cx, &state, false)
            .await
            .unwrap();
        assert_eq!(after["brain"]["projection_lag"], 0, "{}", after["brain"]);
    });
}
