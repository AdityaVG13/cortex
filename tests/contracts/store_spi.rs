//! Runtime seam + BrainStore SPI core-level contracts against the SQLite
//! reference store. Failure-first: each law names the behavior a wrong
//! backend would violate.

use cortex_daemon::protocol::{LogicalId, PayloadAvailability};
use cortex_daemon::runtime::{CortexRuntime, LensInput};
use cortex_daemon::store_spi::sqlite::SqliteStore;
use cortex_daemon::store_spi::{
    BrainStore, CandidateProfile, Durability, Op, Predicate, ReadSnapshot, ScanLimits,
    StoreSpiError, WriteIntent, WriteTransaction,
};
use cortex_tests::support::{open_file_db, solo_state};

fn temp_db() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("cortex-spi-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("cortex.db");
    (dir, path)
}

fn intent(request_id: &str, key: Option<&str>) -> WriteIntent {
    WriteIntent {
        request_id: request_id.into(),
        idempotency_key: key.map(str::to_string),
        principal: "user:test".into(),
        expected_heads: Vec::new(),
    }
}

fn decision(name: &str, text: &str) -> Op {
    Op::InsertDecision {
        local_name: name.into(),
        text: text.into(),
        context: None,
        agent: "spi-agent".into(),
        owner_id: None,
    }
}

#[test]
fn library_caller_deposits_and_recalls_without_http() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        let runtime = CortexRuntime::from_state(state);
        let outcome = runtime
            .deposit(
                cx,
                "req-lib-1",
                "PAY-12 ledger writes must be idempotent under retry",
                "lib-agent",
                None,
            )
            .await
            .expect("deposit");
        assert!(
            outcome.receipt.is_locally_durable(),
            "receipt must carry a local commit frontier: {:?}",
            outcome.receipt
        );
        assert_eq!(
            outcome.receipt.durability.payload_availability,
            PayloadAvailability::Retained
        );
        assert_eq!(
            outcome.receipt.entries.get("decision"),
            Some(&LogicalId::from_legacy(
                "decision",
                outcome.target_id.expect("id")
            ))
        );
        assert!(
            outcome
                .receipt
                .durability
                .projected_through
                .contains_key("lexical"),
            "each maintained index reports its frontier"
        );
        let view = runtime
            .lens(
                cx,
                LensInput {
                    query: "PAY-12".into(),
                    agent: "lib-agent".into(),
                    ..Default::default()
                },
            )
            .await
            .expect("lens");
        let excerpts: Vec<&str> = view["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["excerpt"].as_str())
            .collect();
        assert!(
            excerpts.iter().any(|e| e.contains("PAY-12")),
            "library lens must see the library deposit: {view}"
        );
    });
}

#[test]
fn snapshot_reads_are_coherent_and_expose_a_frontier() {
    let (_dir, path) = temp_db();
    let mut store = SqliteStore::new(open_file_db(&path)).expect("store");
    let mut tx = store.begin_write(intent("r1", None)).unwrap();
    tx.apply(decision("a", "first decision about caching"))
        .unwrap();
    let receipt = tx.commit(Durability::ProcessCrash).unwrap();
    let committed_frontier = receipt.durability.local_commit.clone().expect("frontier");
    let snapshot = store.read_snapshot().unwrap();
    assert_eq!(
        snapshot.frontier(),
        &committed_frontier,
        "a snapshot opened after commit sits at that commit frontier"
    );
    let page = snapshot
        .scan(
            &Predicate::Kind("decision".into()),
            None,
            ScanLimits::default(),
        )
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert!(page.coverage.exhausted);
    assert_eq!(page.rows[0].body["text"], "first decision about caching");
    let got = snapshot.get(&[receipt.entries["a"].clone()]).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, receipt.entries["a"]);
}

#[test]
fn abort_leaves_no_partial_success() {
    let (_dir, path) = temp_db();
    let mut store = SqliteStore::new(open_file_db(&path)).expect("store");
    let mut tx = store.begin_write(intent("r-abort", None)).unwrap();
    tx.apply(decision("x", "will be rolled back")).unwrap();
    tx.apply(decision("y", "also rolled back")).unwrap();
    tx.abort();
    let snapshot = store.read_snapshot().unwrap();
    let page = snapshot
        .scan(
            &Predicate::Kind("decision".into()),
            None,
            ScanLimits::default(),
        )
        .unwrap();
    assert!(
        page.rows.is_empty(),
        "aborted batch must leave nothing: {:?}",
        page.rows
    );
}

#[test]
fn idempotent_replay_returns_original_receipt_and_conflicts_on_different_payload() {
    let (_dir, path) = temp_db();
    let mut store = SqliteStore::new(open_file_db(&path)).expect("store");
    let ops = vec![decision("d", "retry policy: never after ledger commit")];
    let mut tx = store
        .begin_write(intent("r-idem-1", Some("writer/run-1")))
        .unwrap();
    for op in ops.clone() {
        tx.apply(op).unwrap();
    }
    let original = tx.commit(Durability::ProcessCrash).unwrap();
    let replayed = store
        .replay("user:test", "writer/run-1", &ops)
        .unwrap()
        .expect("known key");
    assert_eq!(
        replayed, original,
        "same key + same payload returns the original receipt"
    );
    let different = vec![decision("d", "retry policy: always retry")];
    match store.replay("user:test", "writer/run-1", &different) {
        Err(StoreSpiError::IdempotencyConflict { key }) => assert_eq!(key, "writer/run-1"),
        other => panic!("same key + different payload must conflict, got {other:?}"),
    }
    assert!(
        store
            .replay("user:test", "writer/run-2", &ops)
            .unwrap()
            .is_none(),
        "new key is not a replay"
    );
    assert!(
        store
            .replay("user:other", "writer/run-1", &ops)
            .unwrap()
            .is_none(),
        "idempotency is principal-scoped"
    );
}

#[test]
fn expected_head_mismatch_is_a_conflict_not_an_overwrite() {
    let (_dir, path) = temp_db();
    let mut store = SqliteStore::new(open_file_db(&path)).expect("store");
    let mut tx = store.begin_write(intent("r-head-1", None)).unwrap();
    tx.apply(decision("d", "head test decision")).unwrap();
    let receipt = tx.commit(Durability::ProcessCrash).unwrap();
    let record = receipt.entries["d"].clone();
    let stale = WriteIntent {
        expected_heads: vec![(record.clone(), None)],
        ..intent("r-head-2", None)
    };
    match store.begin_write(stale) {
        Err(StoreSpiError::HeadConflict {
            record: r,
            expected: None,
            actual: Some(_),
        }) => assert_eq!(r, record),
        other => panic!(
            "stale expected head must be rejected, got {:?}",
            other.err()
        ),
    }
    let snapshot = store.read_snapshot().unwrap();
    let rows = snapshot.get(&[record.clone()]).unwrap();
    assert_eq!(
        rows[0].body["status"], "active",
        "rejected write must not touch the record"
    );
}

#[test]
fn exact_candidate_profile_never_claims_exhaustion_it_did_not_have() {
    let (_dir, path) = temp_db();
    let mut store = SqliteStore::new(open_file_db(&path)).expect("store");
    let mut tx = store.begin_write(intent("r-cand", None)).unwrap();
    for i in 0..5 {
        tx.apply(decision(
            &format!("d{i}"),
            &format!("gateway timeout case {i}"),
        ))
        .unwrap();
    }
    tx.commit(Durability::ProcessCrash).unwrap();
    let snapshot = store.read_snapshot().unwrap();
    let limited = snapshot
        .candidates(
            &CandidateProfile::ExactLexical,
            &["gateway".into()],
            ScanLimits {
                rows: 2,
                bytes: 1 << 20,
            },
        )
        .unwrap();
    assert_eq!(limited.rows.len(), 2);
    assert!(
        !limited.coverage.exhausted,
        "a capped candidate pool must say so"
    );
    let full = snapshot
        .candidates(
            &CandidateProfile::ExactLexical,
            &["gateway".into()],
            ScanLimits::default(),
        )
        .unwrap();
    assert_eq!(full.rows.len(), 5);
    assert!(full.coverage.exhausted);
    let none = snapshot
        .candidates(
            &CandidateProfile::ExactLexical,
            &["nonexistent-needle".into()],
            ScanLimits::default(),
        )
        .unwrap();
    assert!(
        none.rows.is_empty() && none.coverage.exhausted,
        "an empty exhausted answer is a real no_match"
    );
}

#[test]
fn read_changes_follows_the_frontier_and_rejects_foreign_epochs() {
    let (_dir, path) = temp_db();
    let mut store = SqliteStore::new(open_file_db(&path)).expect("store");
    let start = store.diagnose().unwrap().frontier;
    let mut tx = store.begin_write(intent("r-ch", None)).unwrap();
    tx.apply(decision("a", "change one")).unwrap();
    tx.apply(decision("b", "change two")).unwrap();
    tx.commit(Durability::ProcessCrash).unwrap();
    let (changes, frontier) = store.read_changes(&start, 10).unwrap();
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_eq!(changes[0].action, "stored");
    let (later, _) = store.read_changes(&frontier, 10).unwrap();
    assert!(later.is_empty(), "nothing after the returned frontier");
    let mut foreign = start.clone();
    foreign.restore_epoch = "restore-99".into();
    assert!(
        matches!(store.read_changes(&foreign, 10), Err(StoreSpiError::Unavailable(msg)) if msg.contains("resnapshot_required"))
    );
}

#[test]
fn export_snapshot_uses_the_online_backup_api_and_round_trips() {
    let (_dir, path) = temp_db();
    let mut store = SqliteStore::new(open_file_db(&path)).expect("store");
    let mut tx = store.begin_write(intent("r-exp", None)).unwrap();
    tx.apply(decision("a", "exported decision")).unwrap();
    tx.commit(Durability::ProcessCrash).unwrap();
    let backup_path = path.with_file_name("backup.db");
    store.export_snapshot(&backup_path).unwrap();
    let restored = SqliteStore::new(open_file_db(&backup_path)).unwrap();
    let snapshot = restored.read_snapshot().unwrap();
    let page = snapshot
        .scan(
            &Predicate::Kind("decision".into()),
            None,
            ScanLimits::default(),
        )
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].body["text"], "exported decision");
    let diag = restored.diagnose().unwrap();
    assert!(diag.integrity_ok);
    assert!(diag.sqlite_version.starts_with("3."));
}
