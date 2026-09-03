use cortex_daemon::handlers::store::store_decision_with_ttl;
use cortex_tests::support::test_conn;
use serde_json::json;
use std::fs;
use std::time::Duration;

#[path = "../support/mod.rs"]
mod support;
use support::{
    daemon_spawn_test_guard, read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon,
    unique_temp_dir, wait_for_exit, wait_for_health,
};

fn db_path_for_home(home: &std::path::Path) -> std::path::PathBuf {
    home.join("cortex.db")
}

#[test]
fn store_redacts_sk_and_ghp_to_redacted_exact() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("redaction_store_secrets");
    fs::create_dir_all(&home_dir).expect("create home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let raw_sk = "sk-test1234567890abcdefFAKE";
    let raw_ghp = "ghp_abcdefghijklmnopqrstuvFAKE";
    let raw_decision = format!(
        "Deployment pipeline uses {raw_sk} and {raw_ghp} for service auth in production handler"
    );
    let expected_decision =
        "Deployment pipeline uses [redacted] and [redacted] for service auth in production handler";

    let store = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({
            "decision": raw_decision,
            "context": "redaction secrets context check",
            "type": "decision",
            "source_agent": "redaction-test",
            "confidence": 0.92
        })),
    )
    .expect("store");
    assert_eq!(
        store.body["stored"], true,
        "store must succeed, got {}",
        store.body
    );
    assert_eq!(store.body["entry"]["action"], "inserted");
    assert_eq!(store.body["entry"]["status"], "active");

    let recall = request_json(
        port,
        "GET",
        "/recall?q=Deployment%20pipeline%20service%20auth%20production%20handler&budget=300&k=10&agent=redaction-test",
        Some(&token),
        None,
    )
    .expect("recall");
    let results = recall.body["results"].as_array().expect("results array");
    let hit = results
        .iter()
        .find(|item| item["excerpt"].as_str() == Some(expected_decision))
        .unwrap_or_else(|| {
            panic!(
                "recall must return exact redacted excerpt {expected_decision:?}, got {}",
                recall.body
            )
        });
    assert_eq!(
        hit["excerpt"].as_str().unwrap(),
        expected_decision,
        "excerpt must be exact redacted string"
    );
    let excerpt = hit["excerpt"].as_str().unwrap();
    assert!(
        !excerpt.contains(raw_sk),
        "excerpt must not contain raw sk secret, got {excerpt:?}"
    );
    assert!(
        !excerpt.contains(raw_ghp),
        "excerpt must not contain raw ghp secret, got {excerpt:?}"
    );
    assert!(
        excerpt.contains("[redacted]"),
        "excerpt must contain [redacted] placeholder, got {excerpt:?}"
    );

    let db_path = db_path_for_home(&home_dir);
    // The DB location is contract-pinned (health reports db_path = home/cortex.db)
    // and the 200 store above already committed a row, so these DB-layer
    // assertions must never be silently skipped behind an existence check.
    assert!(
        db_path.is_file(),
        "daemon db must exist at {} after a successful store; DB-layer redaction assertions must not be skippable",
        db_path.display()
    );
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        let row_id = hit["id"]
            .as_i64()
            .or_else(|| store.body["entry"]["id"].as_i64())
            .expect("id");
        let persisted: String = conn
            .query_row(
                "SELECT decision FROM decisions WHERE id = ?1",
                [row_id],
                |row| row.get(0),
            )
            .expect("select decision");
        assert_eq!(
            persisted, expected_decision,
            "DB row must be stored redacted, got {persisted:?}"
        );
        assert!(
            !persisted.contains(raw_sk),
            "DB row must not contain raw sk"
        );
        assert!(
            !persisted.contains(raw_ghp),
            "DB row must not contain raw ghp"
        );
        let raw_recall = request_json(
            port,
            "GET",
            &format!("/recall?q={}&budget=300&k=10&agent=redaction-test", raw_sk),
            Some(&token),
            None,
        )
        .expect("raw recall");
        let arr = raw_recall.body["results"]
            .as_array()
            .expect("raw recall results array must be present for the FTS leak check");
        {
            let still_contains_raw = arr.iter().any(|item| {
                item["excerpt"]
                    .as_str()
                    .map(|s| s.contains(raw_sk) || s.contains(raw_ghp))
                    .unwrap_or(false)
            });
            assert!(
                !still_contains_raw,
                "FTS must not index raw secret, got raw recall {raw_recall:?}"
            );
        }
    }

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn store_preserves_benign_text_no_false_positive() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("redaction_store_benign");
    fs::create_dir_all(&home_dir).expect("create home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let benign = "Service handles risk-free tokens and mask-1234 in cache layer for user session store handler";
    let store = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({
            "decision": benign,
            "context": "benign redaction check",
            "type": "decision",
            "source_agent": "redaction-test",
            "confidence": 0.9
        })),
    )
    .expect("store benign");
    assert_eq!(store.body["stored"], true);
    assert_eq!(store.body["entry"]["action"], "inserted");

    let recall = request_json(
        port,
        "GET",
        "/recall?q=Service%20handles%20risk-free%20tokens%20mask-1234%20cache%20layer&budget=300&k=10&agent=redaction-test",
        Some(&token),
        None,
    )
    .expect("recall benign");
    let results = recall.body["results"].as_array().expect("results");
    let hit = results
        .iter()
        .find(|item| item["excerpt"].as_str() == Some(benign))
        .unwrap_or_else(|| {
            panic!(
                "benign recall must return exact original text {benign:?}, got {}",
                recall.body
            )
        });
    assert_eq!(
        hit["excerpt"].as_str().unwrap(),
        benign,
        "benign excerpt must be byte-identical, no false-positive redaction"
    );
    assert!(
        !hit["excerpt"].as_str().unwrap().contains("[redacted]"),
        "benign must not contain [redacted], got {:?}",
        hit["excerpt"].as_str().unwrap()
    );

    let db_path = db_path_for_home(&home_dir);
    // The DB location is contract-pinned (health reports db_path = home/cortex.db)
    // and the 200 store above already committed a row, so these DB-layer
    // assertions must never be silently skipped behind an existence check.
    assert!(
        db_path.is_file(),
        "daemon db must exist at {} after a successful store; DB-layer redaction assertions must not be skippable",
        db_path.display()
    );
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        let row_id = hit["id"]
            .as_i64()
            .or_else(|| store.body["entry"]["id"].as_i64())
            .expect("id");
        let persisted: String = conn
            .query_row(
                "SELECT decision FROM decisions WHERE id = ?1",
                [row_id],
                |row| row.get(0),
            )
            .expect("select");
        assert_eq!(persisted, benign, "benign DB row must be byte-identical");
    }

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn store_redacts_context_field_exact() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("redaction_store_context");
    fs::create_dir_all(&home_dir).expect("create home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let raw_ghp = "ghp_abcdefghijklmnopqrstuvFAKE";
    let raw_context = format!("operator context with {raw_ghp} for deploy config store handler");
    let expected_context = "operator context with [redacted] for deploy config store handler";
    let decision = "Context redaction check for production deployment handler with sufficient specificity and tokens";

    let store = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({
            "decision": decision,
            "context": raw_context,
            "type": "decision",
            "source_agent": "redaction-test",
            "confidence": 0.91
        })),
    )
    .expect("store context");
    assert_eq!(store.body["stored"], true);

    let recall = request_json(
        port,
        "GET",
        "/recall?q=Context%20redaction%20production%20deployment%20handler&budget=300&k=10&agent=redaction-test",
        Some(&token),
        None,
    )
    .expect("recall context");
    let results = recall.body["results"].as_array().expect("results");
    let hit = results
        .iter()
        .find(|item| item["excerpt"].as_str() == Some(decision))
        .unwrap_or_else(|| {
            panic!(
                "recall must find decision {decision:?}, got {}",
                recall.body
            )
        });
    let source = hit["source"].as_str().expect("source");
    assert_eq!(
        source, expected_context,
        "context/source must be redacted exact, got {source:?} expected {expected_context:?}"
    );
    assert!(
        !source.contains(raw_ghp),
        "source must not contain raw secret"
    );
    assert!(source.contains("[redacted]"));

    let db_path = db_path_for_home(&home_dir);
    // The DB location is contract-pinned (health reports db_path = home/cortex.db)
    // and the 200 store above already committed a row, so these DB-layer
    // assertions must never be silently skipped behind an existence check.
    assert!(
        db_path.is_file(),
        "daemon db must exist at {} after a successful store; DB-layer redaction assertions must not be skippable",
        db_path.display()
    );
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        let row_id = hit["id"]
            .as_i64()
            .or_else(|| store.body["entry"]["id"].as_i64())
            .expect("id");
        let persisted_context: Option<String> = conn
            .query_row(
                "SELECT context FROM decisions WHERE id = ?1",
                [row_id],
                |row| row.get(0),
            )
            .expect("select context");
        let persisted = persisted_context.expect("context persisted");
        assert_eq!(persisted, expected_context, "DB context must be redacted");
        assert!(!persisted.contains(raw_ghp));
    }

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn lib_store_redacts_before_graph_and_clock_projection() {
    let mut conn = test_conn();

    let raw_sk = "sk-test1234567890abcdefFAKE";
    let raw_ghp = "ghp_abcdefghijklmnopqrstuvFAKE";
    let raw_decision = format!(
        "Deployment pipeline wires {raw_sk} service into production handler and syncs {raw_ghp} service with staging cache cluster"
    );
    let expected_decision =
        "Deployment pipeline wires [redacted] service into production handler and syncs [redacted] service with staging cache cluster";
    // Secret cores that survive entity/anchor normalization (non-alphanumerics
    // stripped, lowercased): a leaked value may be stored in either form.
    let normalized_sk = "sktest1234567890abcdeffake";
    let normalized_ghp = "ghpabcdefghijklmnopqrstuvfake";

    let (entry, id) = store_decision_with_ttl(
        &mut conn,
        &raw_decision,
        Some("redaction projection context".into()),
        Some("decision".into()),
        "redaction-test".into(),
        Some(0.92),
        None,
        None,
    )
    .unwrap_or_else(|err| panic!("lib store must succeed: {err}"));
    assert_eq!(entry["action"], "inserted", "store must insert, got {entry}");
    let row_id = id.expect("store must return a row id");

    let persisted: String = conn
        .query_row(
            "SELECT decision FROM decisions WHERE id = ?1",
            [row_id],
            |row| row.get(0),
        )
        .expect("select decision");
    assert_eq!(
        persisted, expected_decision,
        "lib-persisted row must be redacted, got {persisted:?}"
    );

    // The projection surface: graph ingest writes entities/entity_aliases,
    // clock projection writes clock_anchors. None of them may ever see the
    // raw secret from the public lib API.
    let column_values = |sql: &str| -> Vec<String> {
        let mut stmt = conn
            .prepare(sql)
            .unwrap_or_else(|err| panic!("prepare {sql}: {err}"));
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap_or_else(|err| panic!("query {sql}: {err}"));
        rows.map(|row| row.expect("row value")).collect()
    };
    let projections = [
        (
            "entities.canonical_name",
            column_values("SELECT canonical_name FROM entities"),
        ),
        (
            "entity_aliases.alias",
            column_values("SELECT alias FROM entity_aliases"),
        ),
        (
            "clock_anchors.value",
            column_values("SELECT value FROM clock_anchors"),
        ),
        (
            "clock_anchors.display_value",
            column_values("SELECT COALESCE(display_value, '') FROM clock_anchors"),
        ),
    ];
    for (label, values) in projections.iter() {
        for value in values {
            let lowered = value.to_lowercase();
            for needle in [raw_sk, raw_ghp, normalized_sk, normalized_ghp] {
                assert!(
                    !lowered.contains(&needle.to_lowercase()),
                    "{label} leaked secret {needle:?}: got {value:?}"
                );
            }
        }
    }

    // Positive control: projection must have run on the redacted text, so the
    // redacted surface lands in every projected table. Without this, a fix
    // that silently disabled projection entirely would pass vacuously.
    let redacted_surface = "[redacted] service";
    for (label, values) in projections.iter().take(3) {
        assert!(
            values.iter().any(|value| value == redacted_surface),
            "{label} must contain the redacted surface {redacted_surface:?}, projection did not run or redacted differently: got {values:?}"
        );
    }
}
