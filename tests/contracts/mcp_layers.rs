use cortex_daemon::handlers::recall::{execute_unified_recall, RecallContext};
use cortex_daemon::handlers::store::store_decision_with_ttl;
use cortex_tests::support::team_state;

const SECRET: &str =
    "OWNER_ONE_SECRET_MARKER_c0ffee persist wal in cortex-daemon/src/db/maintenance.rs";

#[tokio::test]
async fn team_recall_hides_other_owner_decisions() {
    let state = team_state(1);
    {
        let mut conn = state.db.lock().await;
        let (entry, id) = store_decision_with_ttl(
            &mut conn,
            SECRET,
            Some("isolation".into()),
            Some("decision".into()),
            "owner-one".into(),
            Some(0.9),
            None,
            Some(1),
        )
        .unwrap_or_else(|err| panic!("store: {err}"));
        assert_eq!(entry["action"], "inserted", "store JSON: {entry}");
        assert!(id.is_some());
        let owner: i64 = conn
            .query_row(
                "SELECT owner_id FROM decisions WHERE id = ?1",
                [id.unwrap()],
                |row| row.get(0),
            )
            .expect("owner_id persisted");
        assert_eq!(owner, 1);
    }

    let hidden = execute_unified_recall(
        &state,
        "OWNER_ONE_SECRET_MARKER_c0ffee wal",
        320,
        8,
        "owner-two",
        &RecallContext {
            caller_id: Some(2),
            team_mode: true,
        },
        None,
    )
    .await
    .unwrap_or_else(|err| panic!("recall as owner 2: {err}"));
    let hidden_results = hidden["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results: {hidden}"));
    assert!(
        hidden_results
            .iter()
            .all(|item| item["excerpt"].as_str() != Some(SECRET)),
        "owner 2 must not see owner 1's decision, got {hidden}"
    );

    let visible = execute_unified_recall(
        &state,
        "OWNER_ONE_SECRET_MARKER_c0ffee wal",
        320,
        8,
        "owner-one",
        &RecallContext {
            caller_id: Some(1),
            team_mode: true,
        },
        None,
    )
    .await
    .unwrap_or_else(|err| panic!("recall as owner 1: {err}"));
    let visible_results = visible["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results: {visible}"));
    assert!(
        visible_results
            .iter()
            .any(|item| item["excerpt"].as_str() == Some(SECRET)),
        "owner 1 must recall their own decision, got {visible}"
    );
}
