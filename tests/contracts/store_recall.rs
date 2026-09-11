use cortex_kernel::handlers::recall::{execute_unified_recall, RecallContext};
use cortex_kernel::handlers::store::store_decision_with_ttl;
use cortex_tests::support::solo_state;

const MARKER: &str =
    "UNIQUE_STORE_RECALL_MARKER_7f3c2a91 persist FTS5 porter tokens in handlers/recall/engine.rs";

#[test]
fn store_rejects_vague_decision_with_exact_error() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let err = {
            let mut conn = state.db.lock(&cx).await.expect("database lock");
            store_decision_with_ttl(
                &mut conn,
                "ok",
                None,
                Some("decision".into()),
                "oracle-agent".into(),
                Some(0.9),
                None,
                None,
            )
            .expect_err("vague text must fail")
        };
        assert_eq!(err, "Memory too vague (quality 0)");
    });
}

#[test]
fn store_then_recall_returns_exact_decision_text() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let id = {
            let mut conn = state.db.lock(&cx).await.expect("database lock");
            let (entry, id) = store_decision_with_ttl(
                &mut conn,
                MARKER,
                Some("round-trip".into()),
                Some("decision".into()),
                "oracle-agent".into(),
                Some(0.9),
                None,
                None,
            )
            .unwrap_or_else(|err| panic!("store: {err}"));
            assert_eq!(entry["action"], "inserted", "store JSON: {entry}");
            assert_eq!(entry["status"], "active");
            let id = id.expect("stored id");
            assert_eq!(entry["id"], id);
            let persisted: String = conn
                .query_row(
                    "SELECT decision FROM decisions WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .expect("select stored decision");
            assert_eq!(persisted, MARKER);
            id
        };

        let recalled = execute_unified_recall(
            &cx,
            &state,
            "UNIQUE_STORE_RECALL_MARKER_7f3c2a91 FTS5 porter",
            320,
            8,
            "oracle-agent",
            &RecallContext::solo(),
            None,
        )
        .await
        .unwrap_or_else(|err| panic!("recall: {err}"));

        let results = recalled["results"]
            .as_array()
            .unwrap_or_else(|| panic!("results array missing: {recalled}"));
        let hit = results
            .iter()
            .find(|item| item["excerpt"].as_str() == Some(MARKER));
        assert!(
            hit.is_some(),
            "recall must return exact stored decision {MARKER:?}, got {recalled}"
        );
        let source = hit.unwrap()["source"].as_str().expect("source string");
        assert_eq!(
            source, "round-trip",
            "decision source is the stored context when present, got {source} for id {id}"
        );
    });
}
