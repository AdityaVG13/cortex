//! Learning-loop contracts: each closed loop proves its write→read wiring.
//! Loop 1 wires outcome feedback into `recall_feedback` signals that the
//! live ranker, boosts, and aging immunity already read.

use cortex_kernel::handlers::feedback::compute_boosts;
use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_logic::clockwork::{parse_query_frame, query_signature};
use cortex_tests::support::solo_state;
use serde_json::{json, Value};

fn caller() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "loop",
        principal: "solo".into(),
    }
}

async fn seed(cx: &asupersync::Cx, state: &cortex_kernel::state::RuntimeState) {
    for (i, text) in [
        "LOOP1 ledger commits are never retried after acknowledgement",
        "LOOP1 deploy freeze covers schema migrations on Fridays",
    ]
    .iter()
    .enumerate()
    {
        let committed = dispatch(
            cx,
            state,
            caller(),
            Operation::Commit,
            &json!({"entries": [{"local_id": format!("lp{i}"), "text": text, "kind": "decision"}]}),
        )
        .await
        .unwrap();
        assert_eq!(committed["status"], "ok", "{committed}");
    }
}

async fn ask(
    cx: &asupersync::Cx,
    state: &cortex_kernel::state::RuntimeState,
    need: &str,
) -> Value {
    dispatch(
        cx,
        state,
        caller(),
        Operation::Query,
        &json!({"need": need, "budget": 4000}),
    )
    .await
    .unwrap()
}

fn first_ref(view: &Value) -> String {
    view["cards"][0]["sidecar"]["reference"]
        .as_str()
        .unwrap_or_else(|| panic!("no admitted card: {view}"))
        .to_string()
}

fn signal_rows(conn: &rusqlite::Connection) -> Vec<(String, f64, String, Option<String>)> {
    let mut stmt = conn
        .prepare("SELECT result_source, signal, query_text, query_signature FROM recall_feedback")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn expected_signature(need: &str) -> String {
    let frame = parse_query_frame(need, None, None, None, Vec::new(), Vec::new(), None, None);
    query_signature(&frame)
}

#[test]
fn loop1_success_emits_positive_signal_with_receipt_query() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let need = "LOOP1 ledger retry policy";
        let view = ask(&cx, &state, need).await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        let out = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "receipt": receipt, "memorySources": [source]}),
        )
        .await
        .unwrap();
        assert_eq!(out["stored"], true, "{out}");
        let conn = state.db.lock(&cx).await.unwrap();
        let rows = signal_rows(&conn);
        assert_eq!(rows.len(), 1, "one success must emit one signal: {rows:?}");
        assert_eq!(rows[0].1, 1.0, "{rows:?}");
        assert_eq!(rows[0].2, need, "query text resolves from the receipt: {rows:?}");
        assert_eq!(
            rows[0].3.as_deref(),
            Some(expected_signature(need).as_str()),
            "signature is the deterministic frame hash: {rows:?}"
        );
    });
}

#[test]
fn loop1_explicit_query_overrides_receipt() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "LOOP1 ledger retry policy").await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        let custom = "custom wording the client asked";
        let out = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "receipt": receipt, "query": custom, "memorySources": [source]}),
        )
        .await
        .unwrap();
        assert_eq!(out["stored"], true, "{out}");
        let conn = state.db.lock(&cx).await.unwrap();
        let rows = signal_rows(&conn);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].2, custom, "{rows:?}");
    });
}

#[test]
fn loop1_partial_emits_half_signal() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "LOOP1 ledger retry policy").await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "partial", "receipt": receipt, "memorySources": [source]}),
        )
        .await
        .unwrap();
        let conn = state.db.lock(&cx).await.unwrap();
        let rows = signal_rows(&conn);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].1, 0.5, "{rows:?}");
    });
}

#[test]
fn loop1_failure_writes_no_signal_but_records_outcome() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "LOOP1 ledger retry policy").await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        let out = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "failure", "receipt": receipt, "memorySources": [source]}),
        )
        .await
        .unwrap();
        assert_eq!(out["stored"], true, "{out}");
        let conn = state.db.lock(&cx).await.unwrap();
        assert!(signal_rows(&conn).is_empty(), "task failure must not indict sources");
        let outcomes: i64 = conn
            .query_row("SELECT COUNT(*) FROM outcome_feedback", [], |r| r.get(0))
            .unwrap();
        assert_eq!(outcomes, 1, "the outcome itself is still recorded");
    });
}

#[test]
fn loop1_harmful_reuse_writes_negative_signal() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "LOOP1 ledger retry policy").await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "receipt": receipt, "memorySources": [source], "harmful_reuse": true}),
        )
        .await
        .unwrap();
        let conn = state.db.lock(&cx).await.unwrap();
        let rows = signal_rows(&conn);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].1, -1.0, "harmful reuse penalizes even on success: {rows:?}");
    });
}

#[test]
fn loop1_missing_query_skips_signals_but_records_outcome() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let out = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "memorySources": ["decision::1"]}),
        )
        .await
        .unwrap();
        assert_eq!(out["stored"], true, "{out}");
        let conn = state.db.lock(&cx).await.unwrap();
        assert!(signal_rows(&conn).is_empty(), "no query text means no signal rows");
        let outcomes: i64 = conn
            .query_row("SELECT COUNT(*) FROM outcome_feedback", [], |r| r.get(0))
            .unwrap();
        assert_eq!(outcomes, 1);
    });
}

#[test]
fn loop1_signals_feed_live_ranking() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "LOOP1 ledger retry policy").await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "receipt": receipt, "memorySources": [source.clone()]}),
        )
        .await
        .unwrap();
        let conn = state.db.lock(&cx).await.unwrap();
        let boosts = compute_boosts(&conn, &[source.clone()], None);
        assert!(
            boosts.get(&source).is_some_and(|b| *b > 0.0),
            "the emitted signal must move the live boost: {boosts:?}"
        );
    });
}

async fn seed_zephyr(cx: &asupersync::Cx, state: &cortex_kernel::state::RuntimeState) {
    for (i, text) in [
        "zephyr capacitors require cryogenic storage",
        "office ficus prefers indirect morning light",
    ]
    .iter()
    .enumerate()
    {
        let committed = dispatch(
            cx,
            state,
            caller(),
            Operation::Commit,
            &json!({"entries": [{"local_id": format!("zx{i}"), "text": text, "kind": "decision"}]}),
        )
        .await
        .unwrap();
        assert_eq!(committed["status"], "ok", "{committed}");
    }
}

fn memory_rows(conn: &rusqlite::Connection) -> Vec<(String, i64, i64, String)> {
    let mut stmt = conn
        .prepare("SELECT signature, asks, successes, terms_json FROM query_memory")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn loop2_query_persisted_with_signature_and_counts() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_zephyr(&cx, &state).await;
        let need = "zephyr maintenance schedule";
        ask(&cx, &state, need).await;
        let conn = state.db.lock(&cx).await.unwrap();
        let rows = memory_rows(&conn);
        assert_eq!(rows.len(), 1, "one query persists one row: {rows:?}");
        assert_eq!(rows[0].1, 1, "{rows:?}");
        assert_eq!(rows[0].2, 0, "no outcome yet: {rows:?}");
        assert_eq!(rows[0].0, expected_signature(need), "stable frame signature: {rows:?}");
        assert!(rows[0].3.contains("zephyr"), "terms stored: {rows:?}");
    });
}

#[test]
fn loop2_repeat_ask_increments_asks() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_zephyr(&cx, &state).await;
        ask(&cx, &state, "zephyr maintenance schedule").await;
        ask(&cx, &state, "zephyr maintenance schedule").await;
        let conn = state.db.lock(&cx).await.unwrap();
        let rows = memory_rows(&conn);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].1, 2, "repeat asks count, not duplicate: {rows:?}");
    });
}

#[test]
fn loop2_feedback_records_success_and_evidence() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_zephyr(&cx, &state).await;
        let view = ask(&cx, &state, "zephyr maintenance schedule").await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "receipt": receipt, "memorySources": [source]}),
        )
        .await
        .unwrap();
        let conn = state.db.lock(&cx).await.unwrap();
        let rows = memory_rows(&conn);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].2, 1, "success recorded: {rows:?}");
        let evidence: String = conn
            .query_row(
                "SELECT last_evidence_json FROM query_memory",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(evidence.contains("decision::"), "used evidence kept: {evidence}");
    });
}

#[test]
fn loop2_similar_past_success_expands_recall() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_zephyr(&cx, &state).await;
        // Q1 shares `zephyr` with the doc: admits lexically, then succeeds.
        let q1 = ask(&cx, &state, "zephyr maintenance schedule").await;
        let source = first_ref(&q1);
        assert!(q1.to_string().contains("cryogenic"), "{q1}");
        let receipt = q1["receipt"].as_str().unwrap().to_string();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "receipt": receipt, "memorySources": [source]}),
        )
        .await
        .unwrap();
        // Q2 shares nothing with the doc, but overlaps Q1's remembered frame.
        let q2 = ask(&cx, &state, "maintenance schedule checklist").await;
        assert!(
            q2.to_string().contains("cryogenic"),
            "Q1's success must route Q2 to the doc: {q2}"
        );
    });
}

#[test]
fn loop2_no_success_no_expansion() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_zephyr(&cx, &state).await;
        // Q1 asked and failed: no success, so Q2 must not route anywhere new.
        let q1 = ask(&cx, &state, "zephyr maintenance schedule").await;
        let source = first_ref(&q1);
        let receipt = q1["receipt"].as_str().unwrap().to_string();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "failure", "receipt": receipt, "memorySources": [source]}),
        )
        .await
        .unwrap();
        let q2 = ask(&cx, &state, "maintenance schedule checklist").await;
        assert!(
            !q2.to_string().contains("cryogenic"),
            "failed queries must not expand recall: {q2}"
        );
    });
}

fn learning_rows(conn: &rusqlite::Connection) -> Vec<(String, String, String, String, i64, String)> {
    let mut stmt = conn
        .prepare("SELECT origin, target, kind, training_unit, reward, cues_json FROM learning_events")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn loop3_success_records_verified_use_event() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let need = "LOOP1 ledger retry policy";
        let view = ask(&cx, &state, need).await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "receipt": receipt, "memorySources": [source]}),
        )
        .await
        .unwrap();
        let conn = state.db.lock(&cx).await.unwrap();
        let rows = learning_rows(&conn);
        assert_eq!(rows.len(), 1, "one success records one event: {rows:?}");
        assert_eq!(rows[0].0, "outcome", "{rows:?}");
        assert_eq!(rows[0].2, "verified_use", "{rows:?}");
        assert_eq!(rows[0].3, receipt, "training unit is the receipt: {rows:?}");
        assert_eq!(rows[0].4, 1, "{rows:?}");
        let cues: Vec<String> = serde_json::from_str(&rows[0].5).unwrap();
        assert_eq!(
            cues,
            cortex_kernel::runtime::assembly::tokenize_cues(need),
            "event cues speak the read path's token vocabulary"
        );
    });
}

#[test]
fn loop3_singleton_assembly_covers_used_decision() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "LOOP1 ledger retry policy").await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "receipt": receipt, "memorySources": [source]}),
        )
        .await
        .unwrap();
        let conn = state.db.lock(&cx).await.unwrap();
        let kinds: Vec<String> = conn
            .prepare("SELECT kind FROM assemblies")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(kinds, vec!["learned_singleton".to_string()], "{kinds:?}");
    });
}

#[test]
fn loop3_partial_and_failure_record_no_events() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        for outcome in ["partial", "failure"] {
            let view = ask(&cx, &state, "LOOP1 ledger retry policy").await;
            let source = first_ref(&view);
            let receipt = view["receipt"].as_str().unwrap().to_string();
            dispatch(
                &cx,
                &state,
                caller(),
                Operation::Feedback,
                &json!({"outcome": outcome, "receipt": receipt, "memorySources": [source]}),
            )
            .await
            .unwrap();
        }
        let conn = state.db.lock(&cx).await.unwrap();
        assert!(learning_rows(&conn).is_empty(), "only success/harmful teach");
    });
}

#[test]
fn loop3_harmful_reuse_records_negative_event() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "LOOP1 ledger retry policy").await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "receipt": receipt, "memorySources": [source], "harmful_reuse": true}),
        )
        .await
        .unwrap();
        let conn = state.db.lock(&cx).await.unwrap();
        let rows = learning_rows(&conn);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].4, -1, "harmful reuse is negative mass: {rows:?}");
    });
}

#[test]
fn loop3_retry_same_receipt_votes_once() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "LOOP1 ledger retry policy").await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        for _ in 0..2 {
            dispatch(
                &cx,
                &state,
                caller(),
                Operation::Feedback,
                &json!({"outcome": "success", "receipt": receipt, "memorySources": [source]}),
            )
            .await
            .unwrap();
        }
        let runtime = cortex_kernel::CortexRuntime::from_state(state.clone());
        runtime.rebuild_assembly_routes(&cx, "default").await.unwrap();
        let conn = state.db.lock(&cx).await.unwrap();
        let edges: i64 = conn
            .query_row("SELECT COUNT(*) FROM assembly_route_edges", [], |r| r.get(0))
            .unwrap();
        assert!(edges > 0, "rebuild compiled edges");
        let overvoted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM assembly_route_edges WHERE positive != 1.0 OR negative != 0.0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(overvoted, 0, "one training unit votes once per edge");
    });
}

#[test]
fn loop3_edges_explain_with_positive_utility() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let need = "LOOP1 ledger retry policy";
        let view = ask(&cx, &state, need).await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "receipt": receipt, "memorySources": [source]}),
        )
        .await
        .unwrap();
        let runtime = cortex_kernel::CortexRuntime::from_state(state.clone());
        runtime.rebuild_assembly_routes(&cx, "default").await.unwrap();
        let cues = cortex_kernel::runtime::assembly::tokenize_cues(need);
        let explanations = runtime
            .explain_assembly_routes(&cx, "default", &cues, 4)
            .await
            .unwrap();
        assert_eq!(explanations.len(), 1, "{explanations:?}");
        assert!(explanations[0].utility > 0.0, "{explanations:?}");
        assert!(!explanations[0].training_units.is_empty(), "{explanations:?}");
    });
}

fn bridge_rows(conn: &rusqlite::Connection) -> Vec<(String, String, i64, i64)> {
    let mut stmt = conn
        .prepare("SELECT query_term, doc_term, positive, negative FROM term_bridges")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

async fn succeed_with(
    cx: &asupersync::Cx,
    state: &cortex_kernel::state::RuntimeState,
    need: &str,
) -> Value {
    let view = ask(cx, state, need).await;
    let source = first_ref(&view);
    let receipt = view["receipt"].as_str().unwrap().to_string();
    dispatch(
        cx,
        state,
        caller(),
        Operation::Feedback,
        &json!({"outcome": "success", "receipt": receipt, "memorySources": [source]}),
    )
    .await
    .unwrap()
}

#[test]
fn loop4_success_records_term_pairs() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_zephyr(&cx, &state).await;
        succeed_with(&cx, &state, "zephyr schedule upkeep").await;
        let conn = state.db.lock(&cx).await.unwrap();
        let rows = bridge_rows(&conn);
        assert!(!rows.is_empty(), "success records bridges: {rows:?}");
        assert!(
            rows.iter().any(|(q, d, p, _)| q == "schedule" && d == "zephyr" && *p == 1),
            "query-term x doc-term pair counted once: {rows:?}"
        );
        assert!(
            rows.iter().all(|(_, _, _, n)| *n == 0),
            "no negative mass on success: {rows:?}"
        );
    });
}

#[test]
fn loop4_bridge_threshold_gates_paraphrase_routing() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_zephyr(&cx, &state).await;
        // Q2 shares one spec-1 term with Q1 (no Loop 2 fire) and nothing
        // with the doc: only a mass-2 bridge can route it.
        succeed_with(&cx, &state, "zephyr schedule upkeep").await;
        let before = ask(&cx, &state, "schedule checklist inventory").await;
        assert!(
            !before.to_string().contains("cryogenic"),
            "one success is below the bridge threshold: {before}"
        );
        succeed_with(&cx, &state, "zephyr schedule roster").await;
        let after = ask(&cx, &state, "schedule checklist inventory").await;
        assert!(
            after.to_string().contains("cryogenic"),
            "two successes bridge the paraphrase: {after}"
        );
    });
}

#[test]
fn loop4_harmful_vetoes_bridge() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_zephyr(&cx, &state).await;
        succeed_with(&cx, &state, "zephyr schedule upkeep").await;
        succeed_with(&cx, &state, "zephyr schedule roster").await;
        let view = ask(&cx, &state, "zephyr schedule upkeep").await;
        let source = first_ref(&view);
        let receipt = view["receipt"].as_str().unwrap().to_string();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Feedback,
            &json!({"outcome": "success", "receipt": receipt, "memorySources": [source], "harmful_reuse": true}),
        )
        .await
        .unwrap();
        let q2 = ask(&cx, &state, "schedule checklist inventory").await;
        assert!(
            !q2.to_string().contains("cryogenic"),
            "harmful reuse must veto the bridge: {q2}"
        );
    });
}

async fn seed_threaded(cx: &asupersync::Cx, state: &cortex_kernel::state::RuntimeState) {
    // D1 is deliberately unthreaded: a strong top hit would set a
    // relevance floor that cuts the weak-but-admitted sibling. Purpose
    // routing matters exactly when nothing dominates lexically.
    for (id, text, thread, paths) in [
        ("t1", "zephyr capacitors require cryogenic storage", None, vec!["/repo"]),
        ("t2", "harbor freight manifests need signatures", Some("T5"), vec!["/repo"]),
        ("t3", "ballast pump inspections every quarter", None, vec!["/repo"]),
        ("t4", "ledger paperclips inventory count", Some("T5"), vec!["/other"]),
    ] {
        let entry = json!({"local_id": id, "text": text, "kind": "decision", "thread": thread, "paths": paths});
        let committed = dispatch(cx, state, caller(), Operation::Commit, &json!({"entries": [entry]}))
            .await
            .unwrap();
        assert_eq!(committed["status"], "ok", "{committed}");
    }
}

#[test]
fn loop5_threaded_deposit_joins_thread() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_threaded(&cx, &state).await;
        let conn = state.db.lock(&cx).await.unwrap();
        let members: Vec<(String, String)> = conn
            .prepare("SELECT thread_id, role FROM thread_members ORDER BY record_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            members,
            vec![
                ("thread:t5".to_string(), "deposit".to_string()),
                ("thread:t5".to_string(), "deposit".to_string()),
            ],
            "the two threaded deposits join: {members:?}"
        );
    });
}

#[test]
fn loop5_activity_arm_surfaces_thread_sibling() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_threaded(&cx, &state).await;
        let view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "zephyr maintenance", "thread": "T5", "budget": 4000}),
        )
        .await
        .unwrap();
        let body = view.to_string();
        assert!(body.contains("cryogenic"), "seed doc admits: {view}");
        assert!(
            body.contains("harbor freight"),
            "thread sibling routes via activity + hop: {view}"
        );
        assert!(
            !body.contains("ballast pump"),
            "hop without thread membership is not enough: {view}"
        );
        assert!(
            !body.contains("paperclips"),
            "thread membership without a second line is not enough: {view}"
        );
        let cards = view["cards"].as_array().cloned().unwrap_or_default();
        let sibling = cards
            .iter()
            .find(|c| c["statement"].as_str().is_some_and(|s| s.contains("harbor freight")))
            .unwrap_or_else(|| panic!("sibling card missing: {view}"));
        let arms: Vec<String> = sibling["sidecar"]["arms"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|a| a.as_str().map(str::to_string))
            .collect();
        assert!(arms.contains(&"activity".to_string()), "provenance names the arm: {arms:?}");
    });
}

async fn seed_couse(cx: &asupersync::Cx, state: &cortex_kernel::state::RuntimeState) {
    // No threads anywhere: the activity arm must stay silent so the
    // feedback grant is the only learned line in play.
    for (id, text, paths) in [
        ("c1", "zephyr capacitors require cryogenic storage", vec!["/repo"]),
        ("c2", "harbor freight manifests need signatures", vec!["/repo"]),
        ("c3", "ballast pump inspections every quarter", vec!["/repo"]),
        ("c4", "ledger paperclips inventory count", vec!["/other"]),
    ] {
        let entry = json!({"local_id": id, "text": text, "kind": "decision", "paths": paths});
        let committed = dispatch(cx, state, caller(), Operation::Commit, &json!({"entries": [entry]}))
            .await
            .unwrap();
        assert_eq!(committed["status"], "ok", "{committed}");
    }
}

async fn couse_d1_d2(cx: &asupersync::Cx, state: &cortex_kernel::state::RuntimeState, harmful: bool) {
    let view = ask(cx, state, "zephyr harbor logistics").await;
    let cards = view["cards"].as_array().cloned().unwrap_or_default();
    assert!(cards.len() >= 2, "Q1 must admit both docs: {view}");
    let sources: Vec<String> = cards
        .iter()
        .take(2)
        .filter_map(|c| c["sidecar"]["reference"].as_str().map(str::to_string))
        .collect();
    let receipt = view["receipt"].as_str().unwrap().to_string();
    dispatch(
        cx,
        state,
        caller(),
        Operation::Feedback,
        &json!({"outcome": "success", "receipt": receipt, "memorySources": sources, "harmful_reuse": harmful}),
    )
    .await
    .unwrap();
}

#[test]
fn loop6_couse_records_used_with_link() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_couse(&cx, &state).await;
        couse_d1_d2(&cx, &state, false).await;
        let conn = state.db.lock(&cx).await.unwrap();
        let links: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM clock_links WHERE relation = 'used_with' AND status != 'rejected'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(links, 1, "one co-use records one link");
    });
}

#[test]
fn loop6_single_use_records_no_link() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_couse(&cx, &state).await;
        succeed_with(&cx, &state, "zephyr maintenance schedule").await;
        let conn = state.db.lock(&cx).await.unwrap();
        let links: i64 = conn
            .query_row("SELECT COUNT(*) FROM clock_links WHERE relation = 'used_with'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(links, 0, "one used source is not a pair: {links}");
    });
}

#[test]
fn loop6_coused_sibling_admits_via_feedback_grant() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_couse(&cx, &state).await;
        couse_d1_d2(&cx, &state, false).await;
        let view = ask(&cx, &state, "zephyr maintenance").await;
        let body = view.to_string();
        assert!(body.contains("cryogenic"), "seed admits: {view}");
        assert!(
            body.contains("harbor freight"),
            "co-used sibling admits via the feedback grant: {view}"
        );
        assert!(!body.contains("ballast pump"), "hop without co-use is not enough: {view}");
        assert!(!body.contains("paperclips"), "unlinked doc stays out: {view}");
        assert!(!body.contains("\"activity\""), "no thread, no activity line: {view}");
    });
}

#[test]
fn loop6_harmful_rejects_link() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed_couse(&cx, &state).await;
        couse_d1_d2(&cx, &state, false).await;
        couse_d1_d2(&cx, &state, true).await;
        let view = ask(&cx, &state, "zephyr maintenance").await;
        assert!(
            !view.to_string().contains("harbor freight"),
            "rejected co-use grants nothing: {view}"
        );
    });
}
