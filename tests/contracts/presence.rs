//! Presence and change-cursor laws. Delivery is not presence: suppression
//! only on exact attested representation under matching epochs (exhaustive
//! truth table), self-contained delivery after compaction / for opaque
//! hosts, `resnapshot_required` on epoch or scope gaps, boot base always
//! carries durable constraints, feed unread boundary is canonical.

use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_logic::presence::{
    decide, ChangeCursor, CurrentEpochs, CursorError, PresenceDecision, CHANGE_RULE_VERSION,
};
use cortex_logic::protocol::envelope::PresentRepresentation;
use cortex_logic::protocol::{ContextPresence, LogicalId};
use cortex_tests::support::solo_state;
use serde_json::json;

fn caller() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "presence-agent",
        principal: "solo".into(),
    }
}

#[test]
fn presence_truth_table_suppresses_only_exact_attested_representation() {
    let current = CurrentEpochs {
        brain_epoch: "b1".into(),
        policy_epoch: "p1".into(),
    };
    let rev = LogicalId::new("revision", "r1");
    let mut suppressed = 0;
    let mut total = 0;
    for has_presence in [false, true] {
        for same_context in [false, true] {
            for same_brain in [false, true] {
                for same_policy in [false, true] {
                    for same_revision in [false, true] {
                        for same_repr in [false, true] {
                            for attested_present in [false, true] {
                                total += 1;
                                let presence = has_presence.then(|| ContextPresence {
                                    context_epoch: if same_context {
                                        "c1".into()
                                    } else {
                                        "c0".into()
                                    },
                                    invocation: "i1".into(),
                                    present: if attested_present {
                                        vec![PresentRepresentation {
                                            revision: if same_revision {
                                                rev.clone()
                                            } else {
                                                LogicalId::new("revision", "r0")
                                            },
                                            representation: if same_repr {
                                                "brief/1".into()
                                            } else {
                                                "exact/1".into()
                                            },
                                        }]
                                    } else {
                                        vec![]
                                    },
                                });
                                let brain =
                                    has_presence.then(|| if same_brain { "b1" } else { "b0" });
                                let policy =
                                    has_presence.then(|| if same_policy { "p1" } else { "p0" });
                                let decision = decide(
                                    presence.as_ref(),
                                    brain,
                                    policy,
                                    &current,
                                    "c1",
                                    &rev,
                                    "brief/1",
                                );
                                let expect_suppress = has_presence
                                    && same_context
                                    && same_brain
                                    && same_policy
                                    && same_revision
                                    && same_repr
                                    && attested_present;
                                assert_eq!(decision.suppresses(), expect_suppress, "{has_presence} {same_context} {same_brain} {same_policy} {same_revision} {same_repr} {attested_present} -> {decision:?}");
                                if !has_presence {
                                    assert_eq!(decision, PresenceDecision::DeliverUnknownPresence);
                                } else if !same_context {
                                    assert_eq!(
                                        decision,
                                        PresenceDecision::DeliverContextEpochChanged
                                    );
                                }
                                if expect_suppress {
                                    suppressed += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(total, 128);
    assert_eq!(suppressed, 1, "exactly one cell of the table may suppress");
}

#[test]
fn unchanged_card_is_redelivered_after_compaction_and_suppressed_only_with_exact_presence() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        dispatch(
            cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "PRES-1 keep the ledger idempotency contract"}),
        )
        .await
        .unwrap();
        let first = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "PRES-1", "profile": "map", "context_epoch": "ctx-1"}),
        )
        .await
        .unwrap();
        assert_eq!(first["cards"].as_array().unwrap().len(), 1, "{first}");
        let caps = dispatch(cx, &state, caller(), Operation::Capabilities, &json!({}))
            .await
            .unwrap();
        let brain_epoch = caps["brain"]["restore_epoch"].as_str().unwrap();
        let policy_epoch = caps["brain"]["policy_epoch"].as_str().unwrap();
        let revision = {
            let conn = state.db.lock(cx).await.unwrap();
            cortex_kernel::db::records::heads(&conn, "decision:1")
                .unwrap()
                .remove(0)
        };
        let exact = json!({"context_epoch": "ctx-1", "invocation": "i2", "brain_epoch": brain_epoch, "policy_epoch": policy_epoch, "present": [{"revision": {"namespace": "revision", "value": revision}, "representation": "brief/1"}]});
        let suppressed = dispatch(cx, &state, caller(), Operation::Query, &json!({"need": "PRES-1", "profile": "map", "context_epoch": "ctx-1", "context_presence": exact})).await.unwrap();
        assert!(
            suppressed["cards"].as_array().unwrap().is_empty(),
            "exact attested presence suppresses the payload: {suppressed}"
        );
        assert_eq!(suppressed["present"][0]["decision"], "suppress");
        assert_eq!(suppressed["present"][0]["revision"], revision);
        // Compaction: the host reports a new current context epoch while the
        // stale attestation still names ctx-1.
        let after = dispatch(cx, &state, caller(), Operation::Query, &json!({"need": "PRES-1", "profile": "map", "context_epoch": "ctx-2", "context_presence": exact})).await.unwrap();
        assert_eq!(
            after["cards"].as_array().unwrap().len(),
            1,
            "after compaction the unchanged card is delivered again: {after}"
        );
        // Opaque host: no attestation at all; or an attestation without the
        // current epoch (presence unknown).
        let opaque = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "PRES-1", "profile": "map"}),
        )
        .await
        .unwrap();
        assert_eq!(
            opaque["cards"].as_array().unwrap().len(),
            1,
            "unknown presence delivers self-contained: {opaque}"
        );
        let no_epoch = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "PRES-1", "profile": "map", "context_presence": exact}),
        )
        .await
        .unwrap();
        assert_eq!(
            no_epoch["cards"].as_array().unwrap().len(),
            1,
            "an attestation without the current context epoch is not presence: {no_epoch}"
        );
        // Different representation requested: brief presence never satisfies exact.
        let mut exact_repr = exact.clone();
        exact_repr["present"][0]["representation"] = json!("exact/1");
        let repr = dispatch(cx, &state, caller(), Operation::Query, &json!({"need": "PRES-1", "profile": "map", "context_epoch": "ctx-1", "context_presence": exact_repr})).await.unwrap();
        assert_eq!(repr["cards"].as_array().unwrap().len(), 1, "{repr}");
    });
}

#[test]
fn change_cursor_carries_epoch_and_scope_and_requires_resnapshot_on_gaps() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        dispatch(
            cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "CUR-1 first"}),
        )
        .await
        .unwrap();
        let base = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "CUR-1", "profile": "map"}),
        )
        .await
        .unwrap();
        let cursor = base["change_cursor"].as_str().unwrap().to_string();
        let decoded = ChangeCursor::decode(&cursor).unwrap();
        assert_eq!(decoded.rule_version, CHANGE_RULE_VERSION);
        assert!(decoded.scope_filter.ends_with(":map"));
        dispatch(
            cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "CUR-1 second"}),
        )
        .await
        .unwrap();
        let next = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "CUR-1", "profile": "map", "change_cursor": cursor}),
        )
        .await
        .unwrap();
        assert_eq!(next["cursor_status"], "ok", "{next}");
        assert_eq!(
            next["changes"].as_array().unwrap().len(),
            1,
            "one change since the cursor: {next}"
        );
        assert_eq!(
            next["cards"].as_array().unwrap().len(),
            2,
            "the View stays self-contained; the cursor only adds a change list: {next}"
        );
        // Wider/different scope: same cursor, different profile → new baseline.
        let wider = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "CUR-1", "profile": "answer", "change_cursor": next["change_cursor"]}),
        )
        .await
        .unwrap();
        assert_eq!(wider["cursor_status"], "resnapshot_required", "{wider}");
        assert!(
            !wider["cards"].as_array().unwrap().is_empty(),
            "resnapshot never withholds content"
        );
        // Restore epoch moved.
        {
            let conn = state.db.lock(cx).await.unwrap();
            conn.execute(
                "UPDATE brain_meta SET restore_epoch = 'restore-2' WHERE singleton = 1",
                [],
            )
            .unwrap();
        }
        let epoch = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "CUR-1", "profile": "map", "change_cursor": next["change_cursor"]}),
        )
        .await
        .unwrap();
        assert_eq!(epoch["cursor_status"], "resnapshot_required", "{epoch}");
        assert!(matches!(
            ChangeCursor::decode("garbage"),
            Err(CursorError::Malformed)
        ));
    });
}

#[test]
fn feed_unread_boundary_is_decided_by_insertion_order_not_random_ids() {
    use cortex_tests::support::test_conn;
    let conn = test_conn();
    let ts = "2026-09-05T10:00:00.000Z";
    for id in ["zzzz-uuid", "aaaa-uuid", "mmmm-uuid"] {
        conn.execute("INSERT INTO feed (id, agent, kind, summary, timestamp) VALUES (?1, 'other', 'note', ?1, ?2)", [id, ts]).unwrap();
    }
    conn.execute(
        "INSERT INTO feed_acks (agent, last_seen_id, updated_at) VALUES ('me', 'aaaa-uuid', ?1)",
        [ts],
    )
    .unwrap();
    let unread: Vec<String> = conn
        .prepare("SELECT id FROM feed WHERE agent != 'me' AND (timestamp > ?1 OR (timestamp = ?1 AND rowid > (SELECT rowid FROM feed WHERE id = 'aaaa-uuid'))) ORDER BY timestamp ASC, rowid ASC")
        .unwrap()
        .query_map([ts], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        unread,
        vec!["mmmm-uuid".to_string()],
        "only rows inserted after the acked row are unread; uuid order is irrelevant"
    );
}
