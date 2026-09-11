//! Durable Deposit laws: the acknowledgement profile follows the real
//! `synchronous` pragma; a deposit is one atomic batch; idempotency is
//! principal-scoped with canonical-payload comparison through the local runtime.

use cortex_kernel::db::{configure_with_profile, DurabilityProfile};
use cortex_logic::protocol::AckProfile;
use cortex_kernel::runtime::CortexRuntime;
use cortex_kernel::store_spi::sqlite::ack_profile;
use cortex_tests::support::{run_with_cx, solo_state, test_conn};

#[test]
fn durability_profile_selects_synchronous_and_receipt_reports_it() {
    assert_eq!(
        DurabilityProfile::parse(None),
        DurabilityProfile::Durable,
        "durable is the default"
    );
    assert_eq!(
        DurabilityProfile::parse(Some("fast")),
        DurabilityProfile::Fast
    );
    assert_eq!(
        DurabilityProfile::parse(Some("nonsense")),
        DurabilityProfile::Durable,
        "unknown never silently weakens"
    );
    let conn = test_conn();
    configure_with_profile(&conn, DurabilityProfile::Durable).unwrap();
    let sync: i64 = conn
        .query_row("PRAGMA synchronous", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sync, 2, "durable profile is synchronous=FULL");
    assert!(matches!(
        ack_profile(&conn),
        AckProfile::PowerLossAssumed { .. }
    ));
    configure_with_profile(&conn, DurabilityProfile::Fast).unwrap();
    let sync: i64 = conn
        .query_row("PRAGMA synchronous", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sync, 1, "fast profile is synchronous=NORMAL");
    assert_eq!(
        ack_profile(&conn),
        AckProfile::ProcessCrash,
        "the receipt must not claim more than the pragma provides"
    );
}

#[test]
fn library_idempotency_is_principal_scoped_and_payload_checked() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let first = runtime
            .deposit_with_key(
                &cx,
                "r1",
                Some("run/1"),
                "PAY-40 idempotent deposit",
                "agent-a",
                None,
            )
            .await
            .unwrap();
        let replay = runtime
            .deposit_with_key(
                &cx,
                "r2",
                Some("run/1"),
                "PAY-40 idempotent deposit",
                "agent-a",
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            replay.receipt, first.receipt,
            "same key + same payload returns the original receipt"
        );
        assert_eq!(replay.target_id, first.target_id);
        let conflict = runtime
            .deposit_with_key(
                &cx,
                "r3",
                Some("run/1"),
                "PAY-40 a different text",
                "agent-a",
                None,
            )
            .await;
        assert!(
            matches!(conflict, Err(cortex_kernel::CortexError::Conflict(ref m)) if m.starts_with("idempotency_conflict")),
            "different payload must conflict"
        );
        let count: i64 = runtime
            .state()
            .db
            .lock(&cx)
            .await
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE decision LIKE 'PAY-40%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "no duplicate row was written");
        let other_principal = runtime
            .deposit_with_key(
                &cx,
                "r4",
                Some("run/1"),
                "PAY-40 idempotent deposit",
                "agent-a",
                Some(7),
            )
            .await;
        assert!(
            other_principal.is_ok(),
            "idempotency keys are scoped per principal"
        );
    });
}

#[test]
fn deposit_is_one_atomic_batch() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let outcome = runtime
            .deposit(
                &cx,
                "r-atomic",
                "ATOM-1 atomic deposit projects anchors and traces",
                "agent-a",
                None,
            )
            .await
            .unwrap();
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        let id = outcome.target_id.unwrap();
        let versions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM versions WHERE target_type='decision' AND target_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        let anchors: i64 = conn.query_row("SELECT COUNT(*) FROM clock_anchor_evidence WHERE target_type='decision' AND target_id=?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(versions, 1, "trace/version written in the same batch");
        assert!(anchors > 0, "clock projection written in the same batch");
        // Nothing may be left open after the deposit: an outer BEGIN must succeed.
        conn.execute_batch("BEGIN; ROLLBACK;")
            .expect("deposit released its savepoint");
        let in_tx = conn.is_autocommit();
        assert!(in_tx, "connection is back in autocommit after a deposit");
    });
}

#[test]
fn local_store_replays_by_idempotency_key_and_rejects_payload_conflict() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let first = runtime
            .deposit_with_key(
                &cx,
                "first",
                Some("local/run-1"),
                "PAY-41 local idempotent store",
                "idem",
                None,
            )
            .await
            .unwrap();
        let receipt = serde_json::to_value(&first.receipt).unwrap();
        assert!(
            receipt["durability"]["local_commit"].is_object(),
            "receipt carries the durability vector: {receipt}"
        );
        assert_eq!(
            receipt["durability"]["ack_profile"]["kind"], "power_loss_assumed",
            "default local profile is durable: {receipt}"
        );
        let second = runtime
            .deposit_with_key(
                &cx,
                "second",
                Some("local/run-1"),
                "PAY-41 local idempotent store",
                "idem",
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            second.receipt, first.receipt,
            "replay returns the original receipt"
        );
        let conflict = runtime
            .deposit_with_key(
                &cx,
                "conflict",
                Some("local/run-1"),
                "PAY-41 other text",
                "idem",
                None,
            )
            .await;
        // HTTP status/envelope claims are retired; the public local error is typed.
        assert!(
            matches!(conflict, Err(cortex_kernel::CortexError::Conflict(ref message)) if message.starts_with("idempotency_conflict")),
            "{conflict:?}"
        );
    });
}

/// Concurrency law: a read (recall/peek/budget) must not stall behind a
/// long-held write connection; its retrieval bump is deferred and applied by
/// the next writer under its own lock.
#[test]
fn recall_does_not_block_behind_a_held_write_lock() {
    run_with_cx(|cx| async move {
        use cortex_kernel::handlers::recall::{execute_unified_recall, RecallContext};
        use cortex_kernel::state::SIDE_EFFECT_LOCK_WAIT_MS;
        let runtime = CortexRuntime::from_state(solo_state());
        let state = runtime.state().clone();
        runtime
            .deposit(&cx, "r", "LOCK-1 recall under contention", "agent", None)
            .await
            .unwrap();
        // Simulate a background pass holding the writer for much longer than
        // any read should wait.
        let held = state.db.lock(&cx).await.unwrap();
        let started = std::time::Instant::now();
        let payload = execute_unified_recall(
            &cx,
            &state,
            "LOCK-1",
            320,
            8,
            "agent",
            &RecallContext::solo(),
            None,
        )
        .await
        .unwrap();
        let elapsed = started.elapsed();
        assert!(
            payload["results"]
                .as_array()
                .map(|r| !r.is_empty())
                .unwrap_or(false),
            "{payload}"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(SIDE_EFFECT_LOCK_WAIT_MS * 8),
            "recall waited {elapsed:?} behind the write lock"
        );
        let deferred = state.deferred_side_effects.lock().unwrap().len();
        assert!(deferred >= 1, "side effects were deferred, not dropped");
        drop(held);
        // The next writer drains the queue under its own lock.
        runtime
            .deposit(&cx, "r2", "LOCK-2 drains deferred effects", "agent", None)
            .await
            .unwrap();
        let conn = state.db.lock(&cx).await.unwrap();
        state.drain_deferred(&conn);
        assert_eq!(state.deferred_side_effects.lock().unwrap().len(), 0);
        let retrievals: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(retrievals),0) FROM decisions WHERE decision LIKE 'LOCK-1%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(retrievals >= 1, "deferred retrieval bump was applied");
    });
}
