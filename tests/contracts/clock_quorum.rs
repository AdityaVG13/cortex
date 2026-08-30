#[path = "../support/mod.rs"]
mod support;

use cortex_daemon::clockwork::{
    parse_query_frame, rebuild_clock_projections, record_used_with, reject_used_with, ClockTarget,
};

use cortex_daemon::handlers::recall::{execute_unified_recall, RecallContext};
use cortex_daemon::handlers::store::store_decision_with_ttl;
use cortex_tests::support::{solo_state, team_state};
use serde_json::{json, Value};
use std::fs;
use support::{
    daemon_spawn_test_guard, read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon,
    unique_temp_dir, wait_for_exit, wait_for_health,
};

const AGENT: &str = "cqr-agent";

async fn store_owned(
    state: &cortex_daemon::state::RuntimeState,
    text: &str,
    owner_id: Option<i64>,
) -> (Value, i64) {
    store_owned_with_confidence(state, text, owner_id, 0.9).await
}

async fn store_owned_with_confidence(
    state: &cortex_daemon::state::RuntimeState,
    text: &str,
    owner_id: Option<i64>,
    confidence: f64,
) -> (Value, i64) {
    let mut conn = state.db.lock().await;
    let (entry, id) = store_decision_with_ttl(
        &mut conn,
        text,
        None,
        Some("decision".into()),
        AGENT.into(),
        Some(confidence),
        None,
        owner_id,
    )
    .unwrap_or_else(|err| panic!("store {text:?}: {err}"));
    let id = id
        .or_else(|| entry.get("id").and_then(|v| v.as_i64()))
        .expect("stored id");
    (entry, id)
}

async fn store_text(state: &cortex_daemon::state::RuntimeState, text: &str) -> i64 {
    store_owned(state, text, None).await.1
}

async fn recall_results(
    state: &cortex_daemon::state::RuntimeState,
    query: &str,
    ctx: &RecallContext,
) -> Vec<Value> {
    let payload = execute_unified_recall(state, query, 320, 8, AGENT, ctx, None)
        .await
        .unwrap_or_else(|err| panic!("recall {query:?}: {err}"));
    payload["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results missing: {payload}"))
        .clone()
}

fn excerpts(results: &[Value]) -> Vec<String> {
    results
        .iter()
        .filter_map(|item| item["excerpt"].as_str().map(str::to_string))
        .collect()
}

fn why_of(results: &[Value], excerpt: &str) -> Value {
    results
        .iter()
        .find(|item| item["excerpt"].as_str() == Some(excerpt))
        .and_then(|item| item.get("why").cloned())
        .unwrap_or_else(|| panic!("missing why for {excerpt:?} in {results:?}"))
}

#[test]
fn contract_1_empty_home_no_models_dir() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("cqr-empty-home");
    fs::create_dir_all(&home_dir).expect("home");
    let home = home_dir.to_string_lossy().to_string();
    let port = reserve_port();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let health = request_json(port, "GET", "/health", Some(&token), None)
        .unwrap_or_else(|e| panic!("health: {e}"));
    assert_eq!(health.status, 200, "health {}", health.body);
    assert_eq!(health.body["retrieval"]["engine"], json!("clock-quorum"));
    assert_eq!(health.body["retrieval"]["modelFree"], json!(true));
    assert!(
        !home_dir.join("models").exists(),
        "models dir must not be created: {:?}",
        home_dir.join("models")
    );

    let stored = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({"decision":"CQR_EMPTY_HOME_TOKEN_9f2a persist exact recall","source_agent":AGENT,"confidence":0.9})),
    )
    .unwrap_or_else(|e| panic!("store: {e}"));
    assert_eq!(stored.status, 200, "{}", stored.body);

    let recalled = request_json(
        port,
        "GET",
        "/recall?q=CQR_EMPTY_HOME_TOKEN_9f2a&k=5&budget=320",
        Some(&token),
        None,
    )
    .unwrap_or_else(|e| panic!("recall: {e}"));
    assert_eq!(recalled.status, 200, "{}", recalled.body);
    let hits = excerpts(recalled.body["results"].as_array().unwrap_or(&vec![]));
    assert!(
        hits.iter().any(|h| h.contains("CQR_EMPTY_HOME_TOKEN_9f2a")),
        "exact recall failed: {hits:?} body {}",
        recalled.body
    );
    assert!(
        !home_dir.join("models").exists(),
        "store/recall must not create models dir"
    );
    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, std::time::Duration::from_secs(10));
}

#[tokio::test]
async fn contract_2_exact_path_excludes_neighbor() {
    let state = solo_state();
    let intended = "payments charge retry lives in src/payments/charge.rs::retry_charge";
    let neighbor = "ui spinner retry lives in src/ui/spinner.rs::retry_render";
    store_text(&state, intended).await;
    store_text(&state, neighbor).await;
    let mut ctx = RecallContext::solo();
    ctx.paths.push("src/payments/charge.rs".into());
    let results = recall_results(&state, "retry_charge src/payments/charge.rs", &ctx).await;
    let got = excerpts(&results);
    assert_eq!(
        got.first().map(String::as_str),
        Some(intended),
        "exact path must win, got {got:?}"
    );
    assert!(
        !got.iter().any(|e| e == neighbor),
        "ui neighbor must be excluded, got {got:?}"
    );
}

#[tokio::test]
async fn contract_3_alias_login_system() {
    let state = solo_state();
    let canonical = "The auth service issues session cookies after OAuth microservice handshake";
    let distractor = "Generic OAuth library tokens should not be cached in redis";
    store_text(&state, canonical).await;
    store_text(&state, distractor).await;
    let results = recall_results(&state, "login system", &RecallContext::solo()).await;
    let got = excerpts(&results);
    assert!(
        got.iter().any(|e| e == canonical),
        "alias recall must return canonical auth fact, got {got:?}"
    );
    assert!(
        !got.first().is_some_and(|e| e == distractor),
        "generic OAuth library fact must not win, got {got:?}"
    );
    let why = why_of(&results, canonical);
    let why_s = why.to_string().to_ascii_lowercase();
    assert!(
        why_s.contains("auth")
            || why_s.contains("oauth")
            || why_s.contains("entity")
            || why_s.contains("login"),
        "why must name alias/entity evidence: {why}"
    );
}

#[tokio::test]
async fn contract_4_multihop_jake_postgres() {
    let state = solo_state();
    let first = "Jake proposed the DB move";
    let second = "the DB move is the Postgres migration";
    store_text(&state, first).await;
    store_text(&state, second).await;
    let results = recall_results(&state, "what did Jake propose", &RecallContext::solo()).await;
    let got = excerpts(&results);
    assert!(
        got.iter().any(|e| e == second),
        "multi-hop must admit Postgres decision, got {got:?}"
    );
    let why = why_of(&results, second);
    let why_s = why.to_string();
    assert!(
        why_s.contains("observed_with") || why_s.contains("links"),
        "why must show relation path: {why}"
    );
}

#[tokio::test]
async fn contract_5_current_truth_and_as_of() {
    let state = solo_state();
    let old = "Always use Redis for caching in payments TEMPORALCQR across all deployments";
    let new = "Never use Redis for caching in payments TEMPORALCQR across all deployments";
    let (old_entry, _) = store_owned_with_confidence(&state, old, None, 0.65).await;
    let old_from = old_entry["validFrom"]
        .as_str()
        .expect("old validFrom")
        .to_string();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let (new_entry, _) = store_owned_with_confidence(&state, new, None, 0.99).await;
    assert_eq!(
        new_entry["classification"],
        json!("CONTRADICTS"),
        "replacement must supersede, {new_entry}"
    );
    let current = recall_results(&state, "TEMPORALCQR Redis caching", &RecallContext::solo()).await;
    let current_excerpts = excerpts(&current);
    assert!(
        current_excerpts.iter().any(|e| e == new),
        "current recall must return the replacement, got {current_excerpts:?}"
    );
    assert!(
        !current_excerpts.iter().any(|e| e == old),
        "current recall must hide the closed fact, got {current_excerpts:?}"
    );
    let mut historical_ctx = RecallContext::solo();
    historical_ctx.as_of = Some(old_from.clone());
    let historical = recall_results(&state, "TEMPORALCQR Redis caching", &historical_ctx).await;
    let historical_excerpts = excerpts(&historical);
    assert!(
        historical_excerpts.iter().any(|e| e == old),
        "as-of must recover the first fact, got {historical_excerpts:?}"
    );
    let why = why_of(&historical, old);
    let why_s = why.to_string().to_ascii_lowercase();
    assert!(
        why_s.contains("valid") || why_s.contains("filter") || why_s.contains("supersed"),
        "historical why must name validity/supersession: {why}"
    );
}

#[test]
fn contract_6_rollback_hides_later_store() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("cqr-rollback");
    fs::create_dir_all(&home_dir).expect("home");
    let home = home_dir.to_string_lossy().to_string();
    let port = reserve_port();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);
    let store = |decision: &str| {
        let resp = request_json(
            port,
            "POST",
            "/store",
            Some(&token),
            Some(json!({"decision":decision,"source_agent":AGENT,"confidence":0.9})),
        )
        .unwrap_or_else(|e| panic!("store {decision:?}: {e}"));
        assert_eq!(resp.status, 200, "{}", resp.body);
        resp.body
    };
    let a = store("We chose sqlite WAL journaling CQRHISTALPHA for the ledger");
    let version_a = a["entry"]["versionId"].as_i64().expect("version A");
    let _b = store("Billing exports move to parquet snapshots CQRHISTBETA nightly");
    let rb = request_json(
        port,
        "POST",
        "/rollback",
        Some(&token),
        Some(json!({"to": version_a})),
    )
    .expect("rollback");
    assert_eq!(rb.status, 200, "{}", rb.body);
    let recalled = request_json(
        port,
        "GET",
        "/recall?q=CQRHISTBETA%20parquet&k=10",
        Some(&token),
        None,
    )
    .expect("recall B");
    let got = excerpts(recalled.body["results"].as_array().unwrap_or(&vec![]));
    for excerpt in &got {
        assert!(
            !excerpt.contains("CQRHISTBETA"),
            "rolled-back token must not surface, got {got:?}"
        );
    }

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, std::time::Duration::from_secs(10));
}

#[tokio::test]
async fn contract_7_task_path_context() {
    let state = solo_state();
    let payments = "reviewed scar: timeout retry in src/payments/charge.rs";
    let ui = "reviewed scar: spinner flicker in src/ui/spinner.rs";
    store_text(&state, payments).await;
    store_text(&state, ui).await;
    let mut pay_ctx = RecallContext::solo();
    pay_ctx.paths.push("src/payments/**".into());
    let pay_hits = excerpts(&recall_results(&state, "reviewed scar timeout", &pay_ctx).await);
    assert!(
        pay_hits.iter().any(|e| e == payments),
        "payments path must admit payments scar, got {pay_hits:?}"
    );
    assert!(
        !pay_hits.iter().any(|e| e == ui),
        "payments path must exclude UI scar, got {pay_hits:?}"
    );
    let why = why_of(
        &recall_results(&state, "reviewed scar timeout", &pay_ctx).await,
        payments,
    );
    let why_s = why.to_string().to_ascii_lowercase();
    assert!(
        why_s.contains("task") || why_s.contains("path") || why_s.contains("hard"),
        "why must name the task/path clock: {why}"
    );
    let mut ui_ctx = RecallContext::solo();
    ui_ctx.paths.push("src/ui/**".into());
    let ui_hits = excerpts(&recall_results(&state, "reviewed scar spinner", &ui_ctx).await);
    assert!(
        ui_hits.iter().any(|e| e == ui),
        "UI path must admit UI scar, got {ui_hits:?}"
    );
    assert!(
        !ui_hits.iter().any(|e| e == payments),
        "UI path must exclude payments scar, got {ui_hits:?}"
    );
}

#[tokio::test]
async fn contract_8_negative_neighbor() {
    let state = solo_state();
    let keeper = "billing invoices use Stripe Billing API in payments";
    let neighbor = "design review used stripe patterns in the UI kit";
    store_text(&state, keeper).await;
    store_text(&state, neighbor).await;
    let results = recall_results(&state, "Stripe Billing API", &RecallContext::solo()).await;
    let got = excerpts(&results);
    assert!(
        got.iter().any(|e| e == keeper),
        "keeper must return, got {got:?}"
    );
    assert!(
        !got.iter().any(|e| e == neighbor),
        "weak neighbor must not be admitted, got {got:?}"
    );
}

#[tokio::test]
async fn contract_9_feedback_used_with() {
    let state = solo_state();
    let first = "Jake confirmed the payments retry budget PAYRETRYCQR";
    let second = "Postgres WAL is required for the ledger PGWALCQR";
    let first_id = store_text(&state, first).await;
    let second_id = store_text(&state, second).await;
    {
        let conn = state.db.lock().await;
        record_used_with(
            &conn,
            &ClockTarget {
                target_type: "decision".into(),
                target_id: first_id,
            },
            &ClockTarget {
                target_type: "decision".into(),
                target_id: second_id,
            },
            None,
        )
        .expect("record used_with");
    }
    let linked = recall_results(&state, "PAYRETRYCQR", &RecallContext::solo()).await;
    assert!(
        excerpts(&linked).iter().any(|e| e == second),
        "used_with must admit the paired result, got {:?}",
        excerpts(&linked)
    );
    let why = why_of(&linked, second);
    assert!(
        why.to_string().contains("used_with"),
        "why must name used_with: {why}"
    );
    {
        let conn = state.db.lock().await;
        reject_used_with(
            &conn,
            &ClockTarget {
                target_type: "decision".into(),
                target_id: first_id,
            },
            &ClockTarget {
                target_type: "decision".into(),
                target_id: second_id,
            },
        )
        .expect("reject used_with");
    }
    let after = recall_results(&state, "PAYRETRYCQR", &RecallContext::solo()).await;
    assert!(
        !excerpts(&after).iter().any(|e| e == second),
        "rejected used_with must suppress the pair, got {:?}",
        excerpts(&after)
    );
}

#[tokio::test]
async fn contract_10_determinism() {
    let state = solo_state();
    store_text(
        &state,
        "deterministic clock why CQRDET token for byte equality",
    )
    .await;
    let a = execute_unified_recall(
        &state,
        "CQRDET token",
        320,
        5,
        AGENT,
        &RecallContext::solo(),
        None,
    )
    .await
    .unwrap();
    let b = execute_unified_recall(
        &state,
        "CQRDET token",
        320,
        5,
        AGENT,
        &RecallContext::solo(),
        None,
    )
    .await
    .unwrap();
    let wa = serde_json::to_vec(&a["results"]).expect("a");
    let wb = serde_json::to_vec(&b["results"]).expect("b");
    assert_eq!(wa, wb, "byte-identical results+why required\n{a}\n{b}");
}

#[tokio::test]
async fn contract_11_acl_hides_other_owner_private_row() {
    let state = team_state(1);
    let secret = "other-owner private token ACLPRIVCQR must never leak";
    store_owned(&state, secret, Some(2)).await;
    let mut ctx = RecallContext::from_state(&state);
    ctx.caller_id = Some(1);
    let results = recall_results(&state, "ACLPRIVCQR", &ctx).await;
    assert!(
        results.is_empty() || excerpts(&results).iter().all(|e| !e.contains("ACLPRIVCQR")),
        "private other-owner row must not appear as candidate/result/why: {results:?}"
    );
}

#[tokio::test]
async fn contract_12_honest_miss() {
    let state = solo_state();
    store_text(
        &state,
        "The office snack policy prefers salted almonds on Fridays",
    )
    .await;
    let results = recall_results(
        &state,
        "how should we authenticate the payments webhook",
        &RecallContext::solo(),
    )
    .await;
    assert!(
        results.is_empty() || excerpts(&results).iter().all(|e| !e.contains("almonds")),
        "honest miss must not return snack policy for auth query: {results:?}"
    );
}

#[tokio::test]
async fn contract_13_rebuild_matches_fresh_projection() {
    let state = solo_state();
    store_text(&state, "rebuild projection unique CQRREBUILD token").await;
    {
        let conn = state.db.lock().await;
        conn.execute_batch(
            "DELETE FROM clock_anchor_evidence; DELETE FROM clock_links; DELETE FROM clock_anchors;",
        )
        .expect("clear projections");
        let n = rebuild_clock_projections(&conn, 32).expect("rebuild");
        assert!(n >= 1, "rebuild should project stored decisions, got {n}");
    }
    let results = recall_results(&state, "CQRREBUILD token", &RecallContext::solo()).await;
    assert!(
        excerpts(&results).iter().any(|e| e.contains("CQRREBUILD")),
        "rebuilt projections must serve recall: {results:?}"
    );
}

#[tokio::test]
async fn contract_14_current_truth_caching_nl() {
    let state = solo_state();
    let old = "We are using Redis for caching";
    let current = "We are not using Redis for caching, we moved off Redis to rediska last sprint";
    store_text(&state, old).await;
    store_owned_with_confidence(&state, current, None, 0.95).await;
    {
        let conn = state.db.lock().await;
        conn.execute(
            "UPDATE decisions SET status = 'superseded', updated_at = datetime('now') WHERE decision = ?1 AND status = 'active'",
            rusqlite::params![old],
        )
        .expect("supersede old caching fact");
    }
    let frame = parse_query_frame(
        "what do we use for caching",
        None,
        None,
        None,
        Vec::new(),
        Vec::new(),
        None,
        None,
    );
    let results =
        recall_results(&state, "what do we use for caching", &RecallContext::solo()).await;
    let got = excerpts(&results);
    assert!(
        got.iter().any(|e| e == current),
        "NL caching query must return current fact; terms={:?} quotes={:?} got={got:?} payload={results:?}",
        frame.terms,
        frame.quoted_phrases
    );
    assert!(
        !got.iter().any(|e| e == old),
        "superseded caching fact must stay hidden, got {got:?}"
    );
}

#[tokio::test]
async fn contract_15_morph_cache_nl() {
    let state = solo_state();
    let stored = "Redis is our cache layer in payments CACHENL15";
    store_text(&state, stored).await;
    let results =
        recall_results(&state, "what do we use for caching", &RecallContext::solo()).await;
    let got = excerpts(&results);
    assert!(
        got.iter().any(|e| e == stored),
        "cache↔caching morphology must admit the stored fact, got {got:?}"
    );
}

#[tokio::test]
async fn contract_16_webhook_paraphrase() {
    let state = solo_state();
    let stored = "HMAC verifies Stripe webhooks for payments AUTHWH16";
    store_text(&state, stored).await;
    let results = recall_results(
        &state,
        "how should we authenticate the payments webhook",
        &RecallContext::solo(),
    )
    .await;
    let got = excerpts(&results);
    assert!(
        got.iter().any(|e| e == stored),
        "webhook/payments paraphrase must admit the HMAC fact, got {got:?}"
    );
}

#[tokio::test]
async fn contract_17_vague_auth_query() {
    let state = solo_state();
    let stored = "The auth service issues session cookies after OAuth microservice handshake";
    store_text(&state, stored).await;
    let results = recall_results(&state, "how does auth work", &RecallContext::solo()).await;
    let got = excerpts(&results);
    assert!(
        got.iter().any(|e| e == stored),
        "cluster expansion of auth must recover the login/OAuth fact, got {got:?}"
    );
}

#[tokio::test]
async fn contract_18_honest_miss_still_holds() {
    let state = solo_state();
    store_text(
        &state,
        "The office snack policy prefers salted almonds on Fridays",
    )
    .await;
    let results = recall_results(
        &state,
        "how should we authenticate the payments webhook",
        &RecallContext::solo(),
    )
    .await;
    assert!(
        results.is_empty() || excerpts(&results).iter().all(|e| !e.contains("almonds")),
        "bridge expansion must not admit snack policy: {results:?}"
    );
}
