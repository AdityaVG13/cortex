//! Wave 10: causal replication, origin-lineage support, fencing tokens;
//! authorized erasure across derived state, restore reconciliation of the
//! erasure floor, revocation fence on cached Views; feedback separation and
//! the deferred adaptive policy's safety envelope. Convergence is not
//! consensus: replicas converge on evidence and keep contradictory heads.

use cortex_kernel::db::backup::{backup_to, restore_from};
use cortex_kernel::db::erasure::{
    erase, erasure_floor, fence_check, is_erased, read_ledger, reconcile_after_restore, LEDGER_FILE,
    MAX_ERASURE_LEDGER_BYTES,
};
use cortex_kernel::db::feedback_ledger::{
    adaptive_policy, envelope_check, family_stats, record, OutcomeFeedback, SafeChoice,
};
use cortex_kernel::db::records::{heads, import_legacy, record_for_legacy, revision_body};
use cortex_kernel::db::replication::{
    acquire_fence, check_fence, independent_support, ingest, pending_count, CausalRef, Ingest,
    ReplicatedCommit, SupportWitness,
};
use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_tests::support::{open_file_db, solo_state};
use serde_json::json;
use std::fs;
use support::unique_temp_dir;

#[path = "../support/mod.rs"]
#[allow(dead_code)]
mod support;

fn commit(
    origin: &str,
    counter: i64,
    parents: Vec<CausalRef>,
    record: &str,
    text: &str,
    parent_revisions: Vec<String>,
) -> ReplicatedCommit {
    ReplicatedCommit {
        origin_id: origin.into(),
        origin_counter: counter,
        parents,
        principal: format!("peer:{origin}"),
        ack_profile: "process_crash".into(),
        record_id: record.into(),
        kind: "decision".into(),
        body: json!({"text": text}),
        parent_revisions,
    }
}

#[test]
fn peer_commits_wait_for_parents_and_contradictory_heads_stay_concurrent() {
    let home = unique_temp_dir("repl");
    fs::create_dir_all(&home).unwrap();
    let conn = open_file_db(&home.join("cortex.db"));
    cortex_kernel::db::records::ensure_authoritative_schema(&conn).unwrap();
    // Child arrives before its parent: stored pending, nothing applied.
    let child = commit(
        "peer-a",
        1,
        vec![CausalRef {
            origin_id: "peer-a".into(),
            origin_counter: 0,
        }],
        "rec:x",
        "child",
        vec!["rec:x@1".into()],
    );
    let out = ingest(&conn, &child).unwrap();
    assert!(
        matches!(out, Ingest::Pending { ref missing } if missing.iter().any(|m| m.origin_counter == 0)),
        "{out:?}"
    );
    assert_eq!(pending_count(&conn), 1);
    assert!(
        heads(&conn, "rec:x").unwrap().is_empty(),
        "presentation never precedes causality"
    );
    // Parent arrives: applied, and the pending child drains behind it.
    let parent = commit("peer-a", 0, vec![], "rec:x", "parent", vec![]);
    let out = ingest(&conn, &parent).unwrap();
    assert!(matches!(out, Ingest::Applied { drained: 1, .. }), "{out:?}");
    assert_eq!(pending_count(&conn), 0);
    let h = heads(&conn, "rec:x").unwrap();
    assert_eq!(
        h,
        vec!["rec:x@2".to_string()],
        "child superseded its declared parent: {h:?}"
    );
    assert!(matches!(ingest(&conn, &parent).unwrap(), Ingest::Duplicate));
    // A second origin asserting a contradicting revision from the same base
    // creates a concurrent head; convergence keeps both.
    let other = commit(
        "peer-b",
        0,
        vec![CausalRef {
            origin_id: "peer-a".into(),
            origin_counter: 0,
        }],
        "rec:x",
        "contradiction",
        vec!["rec:x@1".into()],
    );
    assert!(matches!(
        ingest(&conn, &other).unwrap(),
        Ingest::Applied { .. }
    ));
    let h = heads(&conn, "rec:x").unwrap();
    assert_eq!(
        h.len(),
        2,
        "two contradictory heads survive replication: {h:?}"
    );
    assert_eq!(
        revision_body(&conn, "rec:x@3").unwrap().unwrap()["text"],
        "contradiction"
    );
    // Out-of-order per-origin counter is pending until the gap fills.
    let gap = commit("peer-b", 2, vec![], "rec:y", "gap", vec![]);
    assert!(matches!(
        ingest(&conn, &gap).unwrap(),
        Ingest::Pending { .. }
    ));
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn copies_collapse_to_origin_lineage_and_unknown_lineage_earns_nothing() {
    let w = |origin: Option<&str>, agent: &str, copied: Option<&str>| SupportWitness {
        origin_id: origin.map(str::to_string),
        agent: agent.into(),
        copied_from: copied.map(str::to_string),
    };
    // Three agents restating one origin's claim: one lineage.
    assert_eq!(
        independent_support(&[
            w(Some("o1"), "alice", None),
            w(Some("o2"), "bob", Some("o1")),
            w(Some("o3"), "carol", Some("o1"))
        ]),
        1
    );
    // Same agent, two origins: agent identity is not lineage.
    assert_eq!(
        independent_support(&[w(Some("o1"), "alice", None), w(Some("o2"), "alice", None)]),
        2
    );
    // Unknown lineage never adds support.
    assert_eq!(
        independent_support(&[w(None, "alice", None), w(None, "bob", None)]),
        0
    );
    assert_eq!(
        independent_support(&[w(Some("o1"), "alice", None), w(None, "bob", None)]),
        1
    );
}

#[test]
fn leases_are_advisory_and_exclusive_effects_need_a_live_fence_token() {
    let home = unique_temp_dir("fence");
    fs::create_dir_all(&home).unwrap();
    let conn = open_file_db(&home.join("cortex.db"));
    // No fence: an effect that claims exclusivity is refused.
    assert_eq!(
        check_fence(&conn, "deploy:prod", 1).unwrap_err()["status"],
        "denied"
    );
    let first = acquire_fence(&conn, "deploy:prod", "agent-a", 60).unwrap();
    assert!(check_fence(&conn, "deploy:prod", first.token).is_ok());
    // Another holder cannot take a live fence.
    assert!(acquire_fence(&conn, "deploy:prod", "agent-b", 60)
        .unwrap_err()
        .contains("fenced by agent-a"));
    // The holder re-acquires: the token advances and the old one is stale.
    let second = acquire_fence(&conn, "deploy:prod", "agent-a", 60).unwrap();
    assert_eq!(second.token, first.token + 1);
    let stale = check_fence(&conn, "deploy:prod", first.token).unwrap_err();
    assert_eq!(stale["error"], "stale fencing token");
    // Expired fences can be taken over; the previous token is then stale.
    acquire_fence(&conn, "deploy:stage", "agent-a", 1).unwrap();
    conn.execute(
        "UPDATE fences SET expires_at = '2000-01-01T00:00:00Z' WHERE resource = 'deploy:stage'",
        [],
    )
    .unwrap();
    let taken = acquire_fence(&conn, "deploy:stage", "agent-b", 60).unwrap();
    assert_eq!(taken.holder, "agent-b");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn erasure_reaches_derived_state_revokes_views_and_survives_restore() {
    cortex_tests::support::run_with_cx(|_cx| async move {
        let home = unique_temp_dir("erase");
        fs::create_dir_all(&home).unwrap();
        let db = home.join("cortex.db");
        {
            let conn = open_file_db(&db);
            for text in [
                "ERS-1 secret rotation key lives in vault path ops/keys/prod",
                "ERS-2 unrelated cache note",
            ] {
                conn.execute("INSERT INTO decisions (decision, type, source_agent, status, retention_class) VALUES (?1, 'decision', 'seed', 'active', 'durable')", [text]).unwrap();
            }
            import_legacy(&conn).unwrap();
            let _ = cortex_logic::clockwork::rebuild_clock_projections(&conn, 64);
        }
        // Backup BEFORE the erasure: the classic resurrection vector.
        let backup = home.join("pre-erasure.db");
        backup_to(&db, &backup).unwrap();
        let conn = open_file_db(&db);
        let record = record_for_legacy(&conn, "decision", 1).unwrap().unwrap();
        // A View alias bound to the record before erasure.
        conn.execute("INSERT INTO view_receipts (receipt_id, principal_id, brain_epoch, through_sequence, receipt_json) VALUES ('rcpt-e', 'solo', '0', 1, ?1)", [json!({"records": [record.clone()]}).to_string()]).unwrap();
        conn.execute("INSERT INTO view_aliases (receipt_id, alias, record_id, revision_id, representation_version) VALUES ('rcpt-e', 'm1', ?1, ?1 || '@1', 'v1')", [&record]).unwrap();
        assert!(
            erase(&conn, &home, &record, "", "gdpr").is_err(),
            "erasure requires an authority"
        );
        let report = erase(&conn, &home, &record, "owner:aditya", "gdpr request").unwrap();
        assert_eq!(report.revisions_tombstoned, 1);
        assert_eq!(report.legacy_rows_erased, 1);
        assert_eq!(report.aliases_revoked, 1);
        assert_eq!(
            report.views_revoked, 1,
            "orphan view receipts must drop even when receipt_json omits the record id"
        );
        assert!(report.projections_dropped >= 1, "{report:?}");
        assert!(
            !report.not_retractable.is_empty(),
            "external limits are disclosed"
        );
        assert_eq!(erasure_floor(&conn), report.erasure.sequence);
        assert!(is_erased(&conn, &record));
        assert!(
            !is_erased(&conn, "authority")
                && !is_erased(&conn, "record_id")
                && !is_erased(&conn, "decision"),
            "erasure identity is the record_id field, not a JSON substring"
        );
        let body = revision_body(&conn, &format!("{record}@1"))
            .unwrap()
            .unwrap();
        assert_eq!(
            body["erased"], true,
            "tombstone keeps minimum metadata: {body}"
        );
        let text: String = conn
            .query_row("SELECT decision FROM decisions WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(text, "[erased]");
        let anchors: i64 = conn.query_row("SELECT COUNT(*) FROM clock_anchor_evidence WHERE target_type = 'decision' AND target_id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(anchors, 0, "projections dropped");
        let ledger = read_ledger(&home);
        assert_eq!(ledger.len(), 1);
        // Revocation fence: a View minted before the erasure is not deliverable.
        assert_eq!(
            fence_check(&conn, 1).unwrap_err()["status"],
            "resnapshot_required"
        );
        assert!(fence_check(&conn, report.erasure.sequence).is_ok());
        // Recall no longer surfaces the erased content.
        drop(conn);
        // Restore the pre-erasure backup: the ledger re-applies the erasure first.
        let restore = restore_from(&backup, &db, &home).unwrap();
        assert_eq!(restore.erasures_reapplied, 1, "{}", restore.to_json());
        assert!(!restore.quarantined);
        let conn = open_file_db(&db);
        let text: String = conn
            .query_row("SELECT decision FROM decisions WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            text, "[erased]",
            "a restored old backup cannot resurrect an erased record"
        );
        assert!(is_erased(&conn, &record));
        assert!(erasure_floor(&conn) >= 1);
        let survivor: String = conn
            .query_row("SELECT decision FROM decisions WHERE id = 2", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(survivor.contains("ERS-2"), "unrelated records untouched");
        // A restore into a home with a ledger but a backup lacking any erasure
        // rows AND no matching records is quarantined rather than trusted.
        let other_home = unique_temp_dir("erase-quarantine");
        fs::create_dir_all(&other_home).unwrap();
        fs::copy(
            cortex_kernel::db::erasure::ledger_path(&home),
            cortex_kernel::db::erasure::ledger_path(&other_home),
        )
        .unwrap();
        let other_db = other_home.join("cortex.db");
        let empty = unique_temp_dir("erase-empty");
        fs::create_dir_all(&empty).unwrap();
        let empty_db = empty.join("cortex.db");
        {
            let c = open_file_db(&empty_db);
            c.execute("INSERT INTO decisions (decision, type, source_agent, status) VALUES ('fresh brain', 'decision', 'seed', 'active')", []).unwrap();
            import_legacy(&c).unwrap();
        }
        let empty_backup = empty.join("b.db");
        backup_to(&empty_db, &empty_backup).unwrap();
        let _ = open_file_db(&other_db);
        let r = restore_from(&empty_backup, &other_db, &other_home).unwrap();
        assert!(
            r.quarantined,
            "ledger present at home, none in backup, nothing reconcilable ⇒ quarantine: {}",
            r.to_json()
        );
        assert_eq!(r.to_json()["verified"], false);
        for d in [&home, &other_home, &empty] {
            let _ = fs::remove_dir_all(d);
        }
    });
}

#[test]
fn feedback_separates_exposure_use_success_and_credit_and_the_bandit_stays_off() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        let caller = || Caller {
            owner_id: None,
            agent: "fb",
            principal: "solo".into(),
        };
        dispatch(
            cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "FB-1 retries after ledger commit are forbidden"}),
        )
        .await
        .unwrap();
        dispatch(
            cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "FB-2 cache warmers run at 03:00"}),
        )
        .await
        .unwrap();
        let view = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "FB-1 ledger commit retries", "profile": "answer"}),
        )
        .await
        .unwrap();
        let receipt = view["receipt"]["receipt_id"]
            .as_str()
            .or_else(|| view["receipt"].as_str())
            .map(str::to_string)
            .or_else(|| {
                view["receipt"]["receipt_id"]["value"]
                    .as_str()
                    .map(|v| format!("receipt:{v}"))
            });
        let exposed_alias_record = view["cards"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["record"].as_str())
            .map(str::to_string);
        // Feedback that used only one of the exposed sources.
        let out = dispatch(cx, &state, caller(), Operation::Feedback, &json!({"outcome": "success", "taskClass": "retry-policy", "receipt": receipt, "memorySources": ["decision::1"], "scope": "repo-a"})).await.unwrap();
        assert!(out["ledger"].is_object(), "{out}");
        let conn = state.db.lock(cx).await.unwrap();
        let stats = family_stats(&conn, "repo-a", "retry-policy").unwrap();
        assert_eq!(stats.outcomes, 1);
        assert_eq!(stats.successes, 1);
        let used = stats
            .sources
            .get("decision::1")
            .expect("used source counted");
        assert_eq!((used.used, used.credit), (1, 1));
        if let Some(record) = exposed_alias_record {
            if let Some(exposed) = stats.sources.get(&record) {
                assert_eq!(
                    exposed.used, 0,
                    "injection ≠ use: exposed-but-unused sources earn no credit: {exposed:?}"
                );
                assert_eq!(exposed.exposure, 1);
            }
        }
        // Isolation: another scope sees none of it.
        assert_eq!(
            family_stats(&conn, "repo-b", "retry-policy")
                .unwrap()
                .outcomes,
            0
        );
        // Liabilities are counted separately from success.
        record(
            &conn,
            &OutcomeFeedback {
                scope: "repo-a".into(),
                task_family: "retry-policy".into(),
                outcome: "failure".into(),
                used: vec!["decision::2".into()],
                harmful_reuse: true,
                wrong_scope: true,
                agent: "fb".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let stats = family_stats(&conn, "repo-a", "retry-policy").unwrap();
        assert_eq!(
            (
                stats.liabilities.harmful_reuse,
                stats.liabilities.wrong_scope
            ),
            (1, 1)
        );
        assert_eq!(stats.sources["decision::2"].credit, 0);
        // Deferred adaptive policy: off without held-out benefit, envelope enforced.
        assert!(
            !adaptive_policy((20, 25), (10, 25)).enabled,
            "too few held-out samples"
        );
        assert!(!adaptive_policy((20, 40), (25, 40)).enabled, "no benefit");
        assert!(adaptive_policy((30, 40), (20, 40)).enabled);
        assert_eq!(
            envelope_check(&json!({"choice": "view_size"})).unwrap(),
            SafeChoice::ViewSize
        );
        for forbidden in [
            "lower_authorization",
            "erase_source",
            "relabel_as_verified",
            "withhold_constraint",
        ] {
            assert_eq!(
                envelope_check(&json!({"choice": "view_size", "forbidden": forbidden}))
                    .unwrap_err()["status"],
                "denied"
            );
        }
        assert_eq!(
            envelope_check(&json!({"choice": "raise_trust"})).unwrap_err()["status"],
            "denied"
        );
    });
}

#[test]
fn restore_reconciliation_refuses_oversize_erasure_ledger() {
    let home = unique_temp_dir("erasure-ledger-limit");
    fs::create_dir_all(&home).unwrap();
    let db = home.join("cortex.db");
    let conn = open_file_db(&db);
    fs::write(
        home.join(LEDGER_FILE),
        vec![b'x'; MAX_ERASURE_LEDGER_BYTES as usize + 1],
    )
    .unwrap();
    let err = reconcile_after_restore(&conn, &home).unwrap_err();
    assert_eq!(err, "erasure_ledger_byte_limit");
}

#[test]
fn erasure_of_a_memory_does_not_rewrite_a_decision_row() {
    let home = unique_temp_dir("erase-memory");
    fs::create_dir_all(&home).unwrap();
    let conn = open_file_db(&home.join("cortex.db"));
    conn.execute(
        "INSERT INTO memories (text, type, source_agent, status, retention_class) VALUES ('MEM-secret', 'note', 'seed', 'active', 'operational')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO decisions (decision, type, source_agent, status, retention_class) VALUES ('DEC-keep', 'decision', 'seed', 'active', 'durable')",
        [],
    )
    .unwrap();
    import_legacy(&conn).unwrap();
    let record = record_for_legacy(&conn, "memory", 1).unwrap().unwrap();
    let report = erase(&conn, &home, &record, "owner:aditya", "gdpr request").unwrap();
    assert_eq!(report.legacy_rows_erased, 1);
    let mem: String = conn
        .query_row("SELECT text, status FROM memories WHERE id = 1", [], |r| {
            Ok(format!("{}:{}", r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .unwrap();
    assert_eq!(mem, "[erased]:erased");
    let decision: String = conn
        .query_row("SELECT decision, status FROM decisions WHERE id = 1", [], |r| {
            Ok(format!("{}:{}", r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .unwrap();
    assert_eq!(decision, "DEC-keep:active");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn is_erased_matches_plain_string_target_descriptor() {
    let home = unique_temp_dir("erase-plain-descriptor");
    fs::create_dir_all(&home).unwrap();
    let conn = open_file_db(&home.join("cortex.db"));
    conn.execute(
        "INSERT INTO decisions (decision, type, source_agent, status, retention_class) VALUES ('DEC-plain', 'decision', 'seed', 'active', 'durable')",
        [],
    )
    .unwrap();
    import_legacy(&conn).unwrap();
    let record = record_for_legacy(&conn, "decision", 1).unwrap().unwrap();
    erase(&conn, &home, &record, "owner:aditya", "gdpr request").unwrap();
    conn.execute(
        "UPDATE erasures SET target_descriptor = ?1",
        [&record],
    )
    .unwrap();
    assert!(
        is_erased(&conn, &record),
        "pre-JSON target_descriptor rows still identify the erased record"
    );
    assert!(
        !is_erased(&conn, "authority") && !is_erased(&conn, "record_id"),
        "plain-string identity is exact, not a JSON-key substring"
    );
    let _ = fs::remove_dir_all(&home);
}
