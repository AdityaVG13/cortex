//! F20 retains failed-write atomicity and recovery at the kernel deposit seam.
//! Retired with the listener: F13 port conflicts, F14 partial TLS configuration,
//! F15 handshake survival, F18 body limits, and F19 partial-request survival.
//! HTTP status/envelope assertions are not kernel contracts.
use cortex_daemon::CortexRuntime;
use cortex_tests::support::{run_with_cx, solo_state};

#[test]
fn f20_locked_store_is_atomic_and_recovers() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        runtime
            .deposit(
                &cx,
                "baseline",
                "The archive rotator preserves seven nightly snapshots on the vault volume.",
                "fault-matrix",
                None,
            )
            .await
            .expect("baseline deposit");
        let conn = rusqlite::Connection::open(&runtime.state().db_path).unwrap();
        let before: (i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM decisions), (SELECT COUNT(*) FROM events)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        conn.execute_batch("BEGIN EXCLUSIVE")
            .expect("inject write lock");
        let blocked = runtime
            .deposit(
                &cx,
                "blocked",
                "The checksum auditor rejects tiles when crc32 differs from the manifest.",
                "fault-matrix",
                None,
            )
            .await
            .expect_err("locked deposit must not acknowledge");
        assert!(
            blocked.to_string().contains("database is locked"),
            "{blocked}"
        );
        conn.execute_batch("ROLLBACK").unwrap();
        let after: (i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM decisions), (SELECT COUNT(*) FROM events)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            after, before,
            "failed deposit must leave neither row nor event"
        );
        let recovered = runtime
            .deposit(
                &cx,
                "blocked",
                "The checksum auditor rejects tiles when crc32 differs from the manifest.",
                "fault-matrix",
                None,
            )
            .await
            .expect("retry after lock release");
        assert_eq!(recovered.entry["action"], "inserted");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, before.0 + 1);
    });
}
