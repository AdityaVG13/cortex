//! cortex-kernel embed contract: a Rust host opens the brain in-process,
//! deposits one decision and lenses it back. Failure-first: the test binds
//! the daemon's default port itself before opening the runtime and asserts
//! no listener appeared on any new port, so an `open()` that started a
//! server (or needed one) would fail here. The crate must not link axum:
//! `cargo tree -p cortex-kernel -i axum` reports no such package.

use cortex_kernel::{BootInput, CortexError, CortexRuntime, LensInput};
use std::net::TcpListener;

fn listening_ports() -> Vec<u16> {
    // Ports this process is listening on, via `lsof` when available; the
    // assertion below tolerates an empty answer only on platforms without it.
    let pid = std::process::id().to_string();
    let out = std::process::Command::new("lsof").args(["-nP", "-a", "-iTCP", "-sTCP:LISTEN", "-p", &pid]).output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.rsplit(':').next().and_then(|tail| tail.split_whitespace().next()).and_then(|p| p.parse::<u16>().ok()))
        .collect()
}

#[test]
fn kernel_opens_deposits_and_lenses_without_a_server_or_a_port() {
    cortex_tests::support::run_with_cx(|cx| async move {
    // Hold the daemon port so any attempt to serve on it fails loudly.
    let _guard = TcpListener::bind(("127.0.0.1", cortex_kernel::DEFAULT_CORTEX_PORT)).ok();
    let before = listening_ports();
    let dir = tempfile::Builder::new().prefix("cortex-kernel-embed-").tempdir().unwrap();
    let db = dir.path().join("cortex.db");
    let rt = CortexRuntime::open_db(&db).expect("open brain in-process");
    let out = rt.deposit(&cx, "req-embed-1", "EMB-1 ledger writes must be idempotent under retry", "outfit", None).await.expect("deposit");
    assert!(out.receipt.is_locally_durable(), "{:?}", out.receipt);
    let view = rt.lens(&cx, LensInput { query: "EMB-1".into(), agent: "outfit".into(), ..Default::default() }).await.expect("lens");
    let excerpts: Vec<&str> = view["results"].as_array().unwrap().iter().filter_map(|r| r["excerpt"].as_str()).collect();
    assert!(excerpts.iter().any(|e| e.contains("EMB-1")), "{view}");
    let boot = rt.boot(&cx, BootInput { agent: "outfit".into(), ..Default::default() }).await.expect("boot");
    assert!(boot.boot_prompt.contains("EMB-1") || boot.token_estimate > 0, "boot compiles in-process: {}", boot.boot_prompt);
    // Typed errors at the library boundary, not strings.
    let err = rt.deposit(&cx, "req-embed-2", "x", "outfit", None).await.unwrap_err();
    assert!(matches!(err, CortexError::Rejected(_)), "{err:?}");
    let after = listening_ports();
    assert_eq!(before, after, "open()/deposit/lens/boot must not open a listener: before={before:?} after={after:?}");
    // A second handle over the same file sees the same durable state.
    let other = CortexRuntime::open_db(&db).unwrap();
    let again = other.lens(&cx, LensInput { query: "EMB-1".into(), agent: "other".into(), ..Default::default() }).await.unwrap();
    assert!(again.to_string().contains("EMB-1"));
    // The kernel's dependency graph carries no HTTP server.
    let tree = std::process::Command::new("cargo").args(["tree", "-p", "cortex-kernel", "-e", "normal", "--prefix", "none"]).current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).output();
    if let Ok(tree) = tree {
        let text = String::from_utf8_lossy(&tree.stdout);
        for banned in ["axum", "hyper", "tower-http", "tokio-rustls", "reqwest"] {
            assert!(!text.lines().any(|l| l.starts_with(&format!("{banned} "))), "cortex-kernel must not depend on {banned}");
        }
    }
    });
}

#[test]
fn cancelled_deposit_does_not_write_or_poison_the_next_request() {
    let state = cortex_tests::support::run_with_cx(|cx| async move {
        let state = cortex_tests::support::solo_state();
        let runtime = CortexRuntime::from_state(state.clone());
        let guard = state.db.lock(&cx).await.unwrap();
        let cancelled = cx.clone();
        let mut deposit = Box::pin(runtime.deposit(
            &cancelled, "cancelled-1", "CANCEL-1 must not cross the write boundary", "test", None,
        ));
        std::future::poll_fn(|task| {
            assert!(std::future::Future::poll(deposit.as_mut(), task).is_pending());
            std::task::Poll::Ready(())
        }).await;
        cancelled.set_cancel_requested(true);
        assert!(matches!(deposit.await, Err(CortexError::Lock(asupersync::sync::LockError::Cancelled))));
        drop(guard);
        state
    });
    cortex_tests::support::run_with_cx(|cx| async move {
        let conn = state.db.lock(&cx).await.unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM decisions WHERE decision LIKE '%CANCEL-1%'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0, "cancelled waiter must not write");
        drop(conn);
        let runtime = CortexRuntime::from_state(state);
        let out = runtime.deposit(&cx, "retry-1", "CANCEL-1 fresh request can commit", "test", None).await.unwrap();
        assert!(out.receipt.is_locally_durable());
    });
}

