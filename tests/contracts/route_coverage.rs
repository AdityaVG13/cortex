//! Route-coverage contracts for the pass-7 uncontracted surface (bead cortex-l46 / HYP-030).
//!
//! One smoke-plus test per route cluster, driven against the REAL spawned daemon.
//! Each pinned assertion is the observable contract: exact status plus the
//! load-bearing response fields (exact keys/types everywhere; exact values where
//! cheap and deterministic).
//!
//! Facade honesty: the conductor family handlers (and GET /conflicts, POST /archive)
//! are pass-7-flagged in-memory facades. Those tests pin the CURRENT facade behavior
//! exactly (200 + minted-UUID shape + zero-DB-effect proven by a read that stays
//! empty/unchanged) and are marked `FACADE PIN` — this pins today's behavior, not a
//! product claim, until bead cortex-071's product decision lands.

#[path = "../support/mod.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::time::Duration;
use support::{
    daemon_spawn_test_guard, read_token, request_json, request_json_with_headers, reserve_port,
    shutdown_daemon, spawn_daemon, unique_temp_dir, wait_for_exit, wait_for_health,
};

/// Marker prefix long enough that CQR/exact-match arms fire on the stored row.
fn marker(suffix: &str) -> String {
    format!("RCOV_ROUTE_COVERAGE_MARKER_{suffix} deterministic route-coverage probe decision")
}

struct Daemon {
    port: u16,
    token: String,
    home_dir: std::path::PathBuf,
    child: std::process::Child,
}

fn spawn(home_label: &str) -> Daemon {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir(home_label);
    fs::create_dir_all(&home_dir).expect("create temp home");
    let home = home_dir.to_string_lossy().to_string();
    let port = reserve_port();
    let mut child = spawn_daemon(&home, port);
    wait_for_health(port, &mut child);
    let token = read_token(&home_dir);
    Daemon { port, token, home_dir, child }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        shutdown_daemon(self.port, &self.home_dir);
        wait_for_exit(&mut self.child, Duration::from_secs(10));
        let _ = fs::remove_dir_all(&self.home_dir);
    }
}

impl Daemon {
    fn get(&self, path: &str) -> Result<support::JsonHttpResponse, String> {
        request_json(self.port, "GET", path, Some(&self.token), None)
    }

    fn post(&self, path: &str, body: Value) -> Result<support::JsonHttpResponse, String> {
        request_json(self.port, "POST", path, Some(&self.token), Some(body))
    }

    /// POST with explicit headers (replaces the default X-Source-Agent).
    fn post_as(&self, path: &str, agent: &str, body: Value) -> Result<support::JsonHttpResponse, String> {
        request_json_with_headers(
            self.port,
            "POST",
            path,
            &[
                ("Authorization", &format!("Bearer {}", self.token)),
                ("X-Cortex-Request", "true"),
                ("X-Source-Agent", agent),
            ],
            Some(body),
        )
    }

    fn get_without_source_agent(&self, path: &str) -> Result<support::JsonHttpResponse, String> {
        request_json_with_headers(
            self.port,
            "GET",
            path,
            &[
                ("Authorization", &format!("Bearer {}", self.token)),
                ("X-Cortex-Request", "true"),
            ],
            None,
        )
    }

    /// Store a decision and return the persisted decision id.
    fn store_decision(&self, decision: &str, context: Option<&str>) -> i64 {
        let mut body = json!({"decision": decision, "source_agent": "rcov-storer", "confidence": 0.9});
        if let Some(ctx) = context {
            body["context"] = json!(ctx);
        }
        let resp = self.post("/store", body).unwrap_or_else(|e| panic!("store failed: {e}"));
        assert_eq!(resp.status, 200, "store must return 200, body {}", resp.body);
        assert_eq!(resp.body["stored"], true, "store body {}", resp.body);
        resp.body["entry"]["id"].as_i64().unwrap_or_else(|| panic!("store entry id missing: {}", resp.body))
    }

    fn store_decision_as(&self, agent: &str, decision: &str) -> i64 {
        let resp = self
            .post_as("/store", agent, json!({"decision": decision, "confidence": 0.9}))
            .unwrap_or_else(|e| panic!("store as {agent} failed: {e}"));
        assert_eq!(resp.status, 200, "store body {}", resp.body);
        resp.body["entry"]["id"].as_i64().expect("store entry id")
    }
}

// ---------------------------------------------------------------------------
// Wave 1 — core recall family: /recall/semantic, /recall/budget, /recall/explain
// ---------------------------------------------------------------------------

/// SPEC-006 pins "same engine" for the semantic-compat surface. This is the
/// HTTP-level contract: /recall/semantic must route through the clock-quorum
/// engine (model-free) and return the same result rows as the unified engine.
#[test]
fn wave1_recall_semantic_pins_same_engine_http_contract() {
    let daemon = spawn("rcov_semantic");
    let text = marker("SEMANTIC");
    daemon.store_decision(&text, Some("rcov-sem-ctx"));

    let resp = daemon
        .get("/recall/semantic?q=RCOV_ROUTE_COVERAGE_MARKER_SEMANTIC&budget=200&k=10")
        .unwrap_or_else(|e| panic!("semantic recall failed: {e}"));
    assert_eq!(resp.status, 200, "body {}", resp.body);
    assert_eq!(resp.body["mode"], "semantic", "body {}", resp.body);
    assert_eq!(resp.body["semanticAvailable"], true, "body {}", resp.body);
    assert_eq!(
        resp.body["semanticRoute"],
        json!({"engine":"clock-quorum","modelFree":true}),
        "semantic surface must advertise the model-free clock-quorum engine: {}",
        resp.body
    );
    assert_eq!(resp.body["budget"], 200, "budget echo {}", resp.body);
    assert_eq!(resp.body["overBudget"], false, "body {}", resp.body);
    assert!(resp.body["tier"].is_string(), "tier must be a string: {}", resp.body);
    let latency = resp.body["latencyMs"].as_i64().unwrap_or_else(|| panic!("latencyMs missing: {}", resp.body));
    assert!(latency >= 0, "latencyMs {latency}");

    let results = resp.body["results"].as_array().expect("results array");
    let hit = results
        .iter()
        .find(|item| item["excerpt"].as_str() == Some(text.as_str()))
        .unwrap_or_else(|| panic!("stored decision must be recallable via /recall/semantic, got {}", resp.body));
    assert_eq!(hit["method"], "clock-quorum", "hit {hit}");

    let spent = resp.body["spent"].as_u64().expect("spent number");
    let saved = resp.body["saved"].as_i64().expect("saved number");
    assert_eq!(saved, 200 - spent as i64, "saved must be budget minus spent: {}", resp.body);
    assert_eq!(
        resp.body["tokenUsageLine"],
        json!(format!("Cortex recall used {spent} tokens and saved {saved} of 200 budget.")),
        "tokenUsageLine format contract: {}",
        resp.body
    );

    let missing = daemon.get("/recall/semantic").expect("semantic no-q");
    assert_eq!(missing.status, 400, "body {}", missing.body);
    assert_eq!(missing.body["error"], "Missing query parameter: q", "body {}", missing.body);
}

#[test]
fn wave1_recall_budget_pins_budget_echo_and_usage_arithmetic() {
    let daemon = spawn("rcov_budget");
    let text = marker("BUDGET");
    daemon.store_decision(&text, None);

    let resp = daemon
        .get("/recall/budget?q=RCOV_ROUTE_COVERAGE_MARKER_BUDGET&budget=333&k=10")
        .unwrap_or_else(|e| panic!("budget recall failed: {e}"));
    assert_eq!(resp.status, 200, "body {}", resp.body);
    assert_eq!(resp.body["budget"], 333, "budget must echo the request: {}", resp.body);
    let spent = resp.body["spent"].as_u64().expect("spent number");
    let saved = resp.body["saved"].as_i64().expect("saved number");
    assert_eq!(saved, 333 - spent as i64, "saved must be budget minus spent: {}", resp.body);
    assert_eq!(
        resp.body["tokenUsageLine"],
        json!(format!("Cortex recall used {spent} tokens and saved {saved} of 333 budget.")),
        "body {}",
        resp.body
    );
    let results = resp.body["results"].as_array().expect("results array");
    assert!(
        results.iter().any(|item| item["excerpt"].as_str() == Some(text.as_str())),
        "stored decision must come back, got {}",
        resp.body
    );
    // /recall/budget does NOT attach policy-mode fields (unlike /recall and /recall/explain).
    assert!(resp.body.get("mode").is_none(), "no mode key on budget recall: {}", resp.body);
    assert!(resp.body.get("policyMode").is_none(), "no policyMode key on budget recall: {}", resp.body);

    let missing = daemon.get("/recall/budget").expect("budget no-q");
    assert_eq!(missing.status, 400, "body {}", missing.body);
    assert_eq!(missing.body["error"], "Missing query parameter: q", "body {}", missing.body);
}

#[test]
fn wave1_recall_explain_pins_policy_and_explain_shape() {
    let daemon = spawn("rcov_explain");
    let text = marker("EXPLAIN");
    daemon.store_decision(&text, None);

    let resp = daemon
        .get("/recall/explain?q=RCOV_ROUTE_COVERAGE_MARKER_EXPLAIN&budget=200&k=10")
        .unwrap_or_else(|e| panic!("recall explain failed: {e}"));
    assert_eq!(resp.status, 200, "body {}", resp.body);
    assert_eq!(resp.body["budget"], 200, "body {}", resp.body);
    // budget 200 <= 220 resolves to the "fast" policy mode.
    assert_eq!(resp.body["mode"], "fast", "body {}", resp.body);
    assert_eq!(resp.body["policyMode"], "fast", "body {}", resp.body);
    let policy = &resp.body["policy"];
    assert_eq!(policy["name"], "adaptive-recall-policy", "policy {policy}");
    assert_eq!(policy["mode"], "fast", "policy {policy}");
    assert_eq!(policy["budget"], 200, "policy {policy}");
    assert_eq!(policy["requestedK"], 10, "policy {policy}");
    assert_eq!(policy["poolK"], 30, "default pool_k is (max(k,8)*3).min(64): {policy}");

    let explain = &resp.body["explain"];
    assert_eq!(
        explain["shadowSemantic"],
        json!({"enabled":false,"status":"skipped","reason":"model_free","topK":30}),
        "explain {explain}"
    );
    assert_eq!(
        explain["rerank"],
        json!({"status":"skipped","reason":"model_free","mode":"off"}),
        "explain {explain}"
    );
    assert_eq!(explain["familyCompactions"], json!([]), "explain {explain}");
    assert!(explain["droppedCandidates"].is_array(), "explain {explain}");
    let returned = explain["returned"].as_array().expect("explain.returned array");
    let hit = returned
        .iter()
        .find(|item| item["source"].is_string())
        .unwrap_or_else(|| panic!("explain.returned must list ranked candidates: {explain}"));
    assert_eq!(hit["rank"], 1, "first returned candidate ranks 1: {hit}");
    assert_eq!(hit["method"], "clock-quorum", "hit {hit}");
    assert!(hit["rankingFactors"]["entityMatches"].is_i64(), "rankingFactors present: {hit}");

    let results = resp.body["results"].as_array().expect("results array");
    assert!(
        results.iter().any(|item| item["excerpt"].as_str() == Some(text.as_str())),
        "stored decision must be in explain results, got {}",
        resp.body
    );

    let missing = daemon.get("/recall/explain").expect("explain no-q");
    assert_eq!(missing.status, 400, "body {}", missing.body);
    assert_eq!(missing.body["error"], "Missing query parameter: q", "body {}", missing.body);
}

// ---------------------------------------------------------------------------
// Wave 2 — /unfold, /focus/*, /feed* reads (+ /forget via the focus chain)
// ---------------------------------------------------------------------------

#[test]
fn wave2_unfold_pins_full_text_and_not_found_shape() {
    let daemon = spawn("rcov_unfold");

    // Fixture: an imported memory with a past valid_from is deterministically
    // unfoldable by exact source match (POST /import is the contracted fixture
    // writer; adapter_conformance covers its roundtrip separately).
    let text = marker("UNFOLD");
    let imported = daemon
        .post(
            "/import",
            json!({"memories": [{"text": text, "source": "rcov-unfoldable-src", "valid_from": "2020-01-01T00:00:00.000Z"}]}),
        )
        .expect("import fixture");
    assert_eq!(imported.status, 200, "body {}", imported.body);
    assert_eq!(imported.body["imported"]["memories"], 1, "body {}", imported.body);

    let resp = daemon
        .get("/unfold?sources=rcov-unfoldable-src,rcov-never-stored-404")
        .unwrap_or_else(|e| panic!("unfold failed: {e}"));
    assert_eq!(resp.status, 200, "body {}", resp.body);
    let results = resp.body["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2, "one result per requested source: {}", resp.body);

    let found = &results[0];
    assert_eq!(found["source"], "rcov-unfoldable-src", "handler stamps the requested source: {found}");
    assert_eq!(found["type"], "fact", "memory unfolds with its type: {found}");
    assert_eq!(found["text"], json!(text), "unfold returns the full stored text: {found}");
    let tokens = found["tokens"].as_u64().expect("tokens number");
    assert!(tokens > 0, "tokens {tokens}");

    let missing = &results[1];
    assert_eq!(
        missing,
        &json!({"source":"rcov-never-stored-404","text":null,"type":"not_found","tokens":0}),
        "unknown source pins exact not-found shape, got {missing}"
    );
    assert_eq!(resp.body["count"], 1, "count excludes not_found rows: {}", resp.body);
    assert_eq!(resp.body["totalTokens"], tokens, "totalTokens is the sum of item tokens: {}", resp.body);

    // CURRENT-BEHAVIOR PIN (suspected defect, reported to the bead): a decision
    // stored TODAY is invisible to /unfold. The active filter compares the
    // RFC3339 valid_from ("2026-...T...") against datetime('now')
    // ("2026-... ...") as STRINGS, and 'T' > ' ' makes the comparison false for
    // same-day rows — same-day decisions and same-day focus summaries unfold as
    // not_found. If this is ever fixed, this pin goes red on purpose and must
    // be upgraded to the positive decision-unfold contract.
    let ctx = "rcov-unfold-context-77a2";
    let ctx_decision = marker("UNFOLD_CTX");
    daemon.store_decision(&ctx_decision, Some(ctx));
    let same_day = daemon.get(&format!("/unfold?sources={ctx}")).expect("unfold same-day decision");
    assert_eq!(same_day.status, 200, "body {}", same_day.body);
    assert_eq!(
        same_day.body["results"][0],
        json!({"source": ctx, "text": null, "tokens": 0, "type": "not_found"}),
        "SAME-DAY DECISIONS ARE not_found TODAY (valid_from string-compare defect): {}",
        same_day.body
    );

    let no_sources = daemon.get("/unfold").expect("unfold no-sources");
    assert_eq!(no_sources.status, 400, "body {}", no_sources.body);
    assert_eq!(
        no_sources.body["error"],
        "Missing query parameter: sources (comma-separated)",
        "body {}",
        no_sources.body
    );

    let too_many = daemon
        .get(&format!("/unfold?sources={}", (0..51).map(|i| format!("s{i}")).collect::<Vec<_>>().join(",")))
        .expect("unfold too many");
    assert_eq!(too_many.status, 400, "body {}", too_many.body);
    assert_eq!(too_many.body["error"], "Too many sources (max 50)", "body {}", too_many.body);
}

/// /focus/start + /focus/end lifecycle, and POST /forget which needs a real
/// memories row (the focus summary is what creates one over plain HTTP).
#[test]
fn wave2_focus_lifecycle_and_forget_decay_contract() {
    let daemon = spawn("rcov_focus");
    let agent = "rcov-focus-agent";

    let no_label = daemon.post("/focus/start", json!({})).expect("focus start no label");
    assert_eq!(no_label.status, 400, "body {}", no_label.body);
    assert_eq!(no_label.body["error"], "Missing field: label", "body {}", no_label.body);

    let label = "rcov-focus-label";
    let started = daemon
        .post("/focus/start", json!({"label": label, "agent": agent}))
        .expect("focus start");
    assert_eq!(started.status, 200, "body {}", started.body);
    let session_id = started.body["id"].as_i64().expect("session id");
    assert_eq!(started.body["label"], label, "body {}", started.body);
    assert_eq!(started.body["status"], "opened", "body {}", started.body);

    let restarted = daemon
        .post("/focus/start", json!({"label": label, "agent": agent}))
        .expect("focus start again");
    assert_eq!(restarted.status, 200, "body {}", restarted.body);
    assert_eq!(restarted.body["status"], "already_open", "body {}", restarted.body);
    assert_eq!(restarted.body["id"], session_id, "same label must reuse the open session: {}", restarted.body);

    // A store under the same source agent is captured as the session's raw entry.
    let decision = marker("FOCUS");
    let stored_id = daemon.store_decision_as(agent, &decision);
    assert!(stored_id > 0);

    let ended = daemon
        .post("/focus/end", json!({"label": label, "agent": agent}))
        .expect("focus end");
    assert_eq!(ended.status, 200, "body {}", ended.body);
    assert_eq!(ended.body["id"], session_id, "body {}", ended.body);
    assert_eq!(ended.body["status"], "closed", "body {}", ended.body);
    assert_eq!(ended.body["entries"], 1, "the stored decision is one raw entry: {}", ended.body);
    assert_eq!(ended.body["summary"], decision, "single entry consolidates to itself: {}", ended.body);
    assert_eq!(ended.body["savings"], "0%", "identical before/after tokens: {}", ended.body);

    let not_open = daemon
        .post("/focus/end", json!({"label": "rcov-focus-missing", "agent": agent}))
        .expect("focus end unknown label");
    assert_eq!(not_open.status, 500, "honest pin: unknown label is a 500 today, body {}", not_open.body);
    assert_eq!(
        not_open.body["error"],
        "No open focus session with label 'rcov-focus-missing'",
        "body {}",
        not_open.body
    );

    // POST /forget multiplies matching memories' scores by 0.3; the focus summary
    // written above is the matching row.
    let no_keyword = daemon.post("/forget", json!({})).expect("forget no keyword");
    assert_eq!(no_keyword.status, 400, "body {}", no_keyword.body);
    assert_eq!(no_keyword.body["error"], "Missing field: keyword", "body {}", no_keyword.body);

    let forget = daemon
        .post("/forget", json!({"keyword": "RCOV_ROUTE_COVERAGE_MARKER_FOCUS"}))
        .expect("forget");
    assert_eq!(forget.status, 200, "body {}", forget.body);
    assert_eq!(forget.body, json!({"affected": 1}), "exactly the focus-summary memory decays: {}", forget.body);
}

#[test]
fn wave2_feed_read_routes_pin_list_get_by_id_and_404() {
    let daemon = spawn("rcov_feed");

    let invalid = daemon
        .post("/feed", json!({"agent": "rcov-writer", "kind": "note"}))
        .expect("feed missing summary");
    assert_eq!(invalid.status, 400, "body {}", invalid.body);
    assert_eq!(
        invalid.body["error"],
        "Missing required fields: agent, kind, summary",
        "body {}",
        invalid.body
    );

    let summary = "Rcov feed summary line";
    let posted = daemon
        .post(
            "/feed",
            json!({"agent": "rcov-writer", "kind": "note", "summary": summary, "content": "Rcov feed full content body"}),
        )
        .expect("feed post");
    assert_eq!(posted.status, 201, "feed post is 201 CREATED, body {}", posted.body);
    assert_eq!(posted.body["recorded"], true, "body {}", posted.body);
    let feed_id = posted.body["feedId"].as_str().expect("feedId string").to_string();
    assert_eq!(feed_id.len(), 36, "feedId is a minted UUID string: {feed_id}");

    let list = daemon.get("/feed").expect("feed list");
    assert_eq!(list.status, 200, "body {}", list.body);
    let entries = list.body["entries"].as_array().expect("entries array");
    let entry = entries
        .iter()
        .find(|e| e["id"].as_str() == Some(feed_id.as_str()))
        .expect("feed entry must be listed within the default 1h window");
    assert_eq!(entry["agent"], "rcov-writer", "entry {entry}");
    assert_eq!(entry["kind"], "note", "entry {entry}");
    assert_eq!(entry["summary"], summary, "entry {entry}");
    assert_eq!(entry["priority"], "normal", "default priority: {entry}");
    // summary tokens = ceil(len/4): "Rcov feed summary line" is 22 chars -> 6.
    assert_eq!(entry["tokens"], 6, "tokens contract ceil(len/4): {entry}");
    assert!(entry.get("content").is_none(), "list form must omit content: {entry}");

    let by_id = daemon.get(&format!("/feed/{feed_id}")).expect("feed by id");
    assert_eq!(by_id.status, 200, "body {}", by_id.body);
    assert_eq!(by_id.body["content"], "Rcov feed full content body", "by-id form includes content: {}", by_id.body);
    assert_eq!(by_id.body["id"], feed_id, "body {}", by_id.body);

    let not_found = daemon.get("/feed/rcov-nonexistent-feed-id").expect("feed 404");
    assert_eq!(not_found.status, 404, "body {}", not_found.body);
    assert_eq!(not_found.body["error"], "feed_entry_not_found", "body {}", not_found.body);
}

// ---------------------------------------------------------------------------
// Wave 3 — /permissions*, feedback family, health/ops tail, mutate family,
// conductor facades.
// ---------------------------------------------------------------------------

#[test]
fn wave3_permissions_grant_list_revoke_roundtrip() {
    let daemon = spawn("rcov_permissions");

    let fresh = daemon.get("/permissions").expect("permissions fresh");
    assert_eq!(fresh.status, 200, "body {}", fresh.body);
    assert_eq!(fresh.body, json!({"permissions": []}), "fresh DB has no grants: {}", fresh.body);

    let granted = daemon
        .post(
            "/permissions/grant",
            json!({"client": "rcov-client", "permission": "read", "scope": "memory:*", "grantedBy": "rcov-admin"}),
        )
        .expect("grant");
    assert_eq!(granted.status, 200, "body {}", granted.body);
    assert_eq!(granted.body, json!({"granted": true}), "body {}", granted.body);

    let listed = daemon.get("/permissions").expect("permissions after grant");
    assert_eq!(listed.status, 200, "body {}", listed.body);
    let perms = listed.body["permissions"].as_array().expect("permissions array");
    assert_eq!(perms.len(), 1, "exactly one grant: {}", listed.body);
    assert_eq!(perms[0]["client"], "rcov-client", "perm {}", perms[0]);
    assert_eq!(perms[0]["permission"], "read", "perm {}", perms[0]);
    assert_eq!(perms[0]["scope"], "memory:*", "perm {}", perms[0]);
    assert_eq!(perms[0]["grantedBy"], "rcov-admin", "perm {}", perms[0]);
    assert!(perms[0]["grantedAt"].as_str().is_some(), "grantedAt timestamp string: {}", perms[0]);

    let revoked = daemon
        .post("/permissions/revoke", json!({"client": "rcov-client", "permission": "read", "scope": "memory:*"}))
        .expect("revoke");
    assert_eq!(revoked.status, 200, "body {}", revoked.body);
    assert_eq!(revoked.body, json!({"revoked": 1}), "body {}", revoked.body);

    let revoked_again = daemon
        .post("/permissions/revoke", json!({"client": "rcov-client", "permission": "read", "scope": "memory:*"}))
        .expect("revoke again");
    assert_eq!(revoked_again.body, json!({"revoked": 0}), "second revoke deletes nothing: {}", revoked_again.body);

    let empty_again = daemon.get("/permissions").expect("permissions after revoke");
    assert_eq!(empty_again.body, json!({"permissions": []}), "body {}", empty_again.body);
}

#[test]
fn wave3_feedback_family_pins_exact_response_shapes() {
    let daemon = spawn("rcov_feedback");

    let empty_sources = daemon.post("/feedback", json!({"sources": []})).expect("feedback empty");
    assert_eq!(empty_sources.status, 400, "body {}", empty_sources.body);
    assert_eq!(empty_sources.body["error"], "sources array is empty", "body {}", empty_sources.body);

    let fresh_stats = daemon.get("/feedback/stats").expect("feedback stats fresh");
    assert_eq!(fresh_stats.status, 200, "body {}", fresh_stats.body);
    assert_eq!(
        fresh_stats.body,
        json!({"total": 0, "positive": 0, "negative": 0, "uniqueSources": 0, "topBoosted": []}),
        "fresh stats are all zero: {}",
        fresh_stats.body
    );

    let stored = daemon
        .post(
            "/feedback",
            json!({"query": "rcov feedback query", "sources": ["decision::1", "memory::alpha"], "signal": 0.8}),
        )
        .expect("feedback store");
    assert_eq!(stored.status, 200, "body {}", stored.body);
    assert_eq!(
        stored.body,
        json!({"stored": 2, "signal": 0.8, "sources": ["decision::1", "memory::alpha"]}),
        "body {}",
        stored.body
    );

    let clamped = daemon
        .post("/feedback", json!({"sources": ["decision::1"], "signal": 42.0}))
        .expect("feedback clamp");
    assert_eq!(clamped.status, 200, "body {}", clamped.body);
    assert_eq!(clamped.body["signal"], 1.0, "signal clamps to 1.0: {}", clamped.body);

    let stats = daemon.get("/feedback/stats").expect("feedback stats");
    assert_eq!(stats.status, 200, "body {}", stats.body);
    assert_eq!(stats.body["total"], 3, "body {}", stats.body);
    assert_eq!(stats.body["positive"], 3, "body {}", stats.body);
    assert_eq!(stats.body["negative"], 0, "body {}", stats.body);
    assert_eq!(stats.body["uniqueSources"], 2, "body {}", stats.body);
    let top = stats.body["topBoosted"].as_array().expect("topBoosted array");
    let decision_row = top
        .iter()
        .find(|row| row["source"] == "decision::1")
        .expect("decision::1 must be top-boosted");
    assert_eq!(decision_row["totalSignal"], 1.8, "0.8 + clamped 1.0: {}", stats.body);
    assert_eq!(decision_row["hits"], 2, "{}", stats.body);

    let bad_outcome = daemon
        .post("/agent-feedback", json!({"agent": "rcov-agent", "outcome": "excellent"}))
        .expect("agent-feedback invalid outcome");
    assert_eq!(bad_outcome.status, 400, "body {}", bad_outcome.body);
    assert_eq!(
        bad_outcome.body["error"],
        "Missing or invalid outcome (expected success|partial|failure)",
        "body {}",
        bad_outcome.body
    );

    let recorded = daemon
        .post(
            "/agent-feedback",
            json!({"agent": "rcov-agent", "taskClass": "Code", "outcome": "success"}),
        )
        .expect("agent-feedback record");
    assert_eq!(recorded.status, 200, "body {}", recorded.body);
    // Defaults: success -> outcomeScore 1.0, qualityScore 0.7, task class lowercased.
    assert_eq!(
        recorded.body,
        json!({"stored": true, "ownerId": 0, "agent": "rcov-agent", "taskClass": "code",
               "outcome": "success", "outcomeScore": 1.0, "qualityScore": 0.7, "memorySources": []}),
        "body {}",
        recorded.body
    );

    let af_stats = daemon.get("/agent-feedback/stats").expect("agent-feedback stats");
    assert_eq!(af_stats.status, 200, "body {}", af_stats.body);
    assert_eq!(af_stats.body["ownerId"], 0, "body {}", af_stats.body);
    assert_eq!(af_stats.body["horizonDays"], 30, "default horizon: {}", af_stats.body);
    assert_eq!(af_stats.body["limit"], 400, "default limit: {}", af_stats.body);
    assert_eq!(af_stats.body["sampled"], 1, "body {}", af_stats.body);
    assert_eq!(af_stats.body["outcomes"], json!({"success": 1, "partial": 0, "failure": 0}), "body {}", af_stats.body);
    let by_agent = af_stats.body["byAgent"].as_array().expect("byAgent array");
    assert_eq!(by_agent.len(), 1, "body {}", af_stats.body);
    assert_eq!(by_agent[0]["name"], "rcov-agent", "agg {}", by_agent[0]);
    assert_eq!(by_agent[0]["count"], 1, "agg {}", by_agent[0]);
    // 1.0*0.6 + 0.7*0.4 = 0.88 weighted reliability, in the strong band.
    let reliability = af_stats.body["reliability"].as_f64().expect("reliability number");
    assert!((0.87..=0.89).contains(&reliability), "reliability {reliability}");
    assert_eq!(
        af_stats.body["recommendation"],
        "Reliability is strong; continue reinforcing high-quality runs and memory-source coverage.",
        "strong-band recommendation: {}",
        af_stats.body
    );
}

#[test]
fn wave3_health_ops_routes_pin_digest_savings_stats_storage_compact() {
    let daemon = spawn("rcov_health_ops");

    // --- GET /digest (fresh) ---
    let digest = daemon.get("/digest").expect("digest");
    assert_eq!(digest.status, 200, "body {}", digest.body);
    assert!(digest.body["date"].as_str().expect("date string").len() == 10, "body {}", digest.body);
    assert_eq!(digest.body["totals"], json!({"memories": 0, "decisions": 0, "conflicts": 0}), "body {}", digest.body);
    assert_eq!(
        digest.body["tokenSavings"]["allTime"],
        json!({"saved": 0, "served": 0, "boots": 0}),
        "body {}",
        digest.body
    );
    assert_eq!(digest.body["topRecalled"], json!([]), "body {}", digest.body);
    assert_eq!(digest.body["agentBoots"], json!([]), "body {}", digest.body);
    assert!(
        digest.body["oneliner"].as_str().expect("oneliner").starts_with("Cortex Daily — "),
        "body {}",
        digest.body
    );

    // --- GET /savings (fresh; called before any recall so the payload is zeros) ---
    let savings = daemon.get("/savings").expect("savings");
    assert_eq!(savings.status, 200, "body {}", savings.body);
    assert_eq!(savings.body["schemaVersion"], 1, "body {}", savings.body);
    assert_eq!(savings.body["windowDays"], 30, "body {}", savings.body);
    assert_eq!(
        savings.body["totals"],
        json!({"saved": 0, "served": 0, "baseline": 0, "percent": 0}),
        "fresh savings totals are zero: {}",
        savings.body
    );
    assert_eq!(savings.body["boot"], json!({"saved": 0, "served": 0, "baseline": 0, "boots": 0}), "body {}", savings.body);
    assert_eq!(savings.body["recall"], json!({"saved": 0, "spent": 0, "queries": 0}), "body {}", savings.body);

    // --- GET /stats (fresh) ---
    let stats = daemon.get("/stats").expect("stats");
    assert_eq!(stats.status, 200, "body {}", stats.body);
    assert_eq!(stats.body, json!({"queries": 0, "saved": 0, "spent": 0}), "fresh stats zero: {}", stats.body);

    // --- GET /storage (fresh): fixed 11-table breakdown, all zero ---
    let storage = daemon.get("/storage").expect("storage");
    assert_eq!(storage.status, 200, "body {}", storage.body);
    assert!(storage.body["totalBytes"].as_i64().expect("totalBytes") > 0, "body {}", storage.body);
    assert!(storage.body["totalMB"].is_string(), "totalMB formatted string: {}", storage.body);
    let tables = storage.body["tables"].as_array().expect("tables array");
    let names: Vec<&str> = tables.iter().filter_map(|t| t["table"].as_str()).collect();
    assert_eq!(
        names,
        vec![
            "memories",
            "decisions",
            "embeddings",
            "events",
            "recall_feedback",
            "co_occurrence",
            "memory_clusters",
            "cluster_members",
            "event_savings_rollups",
            "context_cache",
            "feed",
        ],
        "storage breakdown is the fixed table list in order: {}",
        storage.body
    );
    let fresh_rows: Vec<(String, i64)> = tables
        .iter()
        .map(|t| (t["table"].as_str().expect("name").to_string(), t["rows"].as_i64().expect("rows")))
        .collect();
    assert_eq!(
        fresh_rows.iter().find(|(n, _)| n == "decisions"),
        Some(&(("decisions").to_string(), 0)),
        "no decisions on a fresh home: {}",
        storage.body
    );
    assert_eq!(fresh_rows.iter().find(|(n, _)| n == "memories"), Some(&("memories".to_string(), 0)), "{}", storage.body);
    assert_eq!(fresh_rows.iter().find(|(n, _)| n == "feed"), Some(&("feed".to_string(), 0)), "{}", storage.body);

    // --- one store, one recall ---
    let text = marker("OPS");
    daemon.store_decision(&text, None);
    let recall = daemon
        .get("/recall?q=RCOV_ROUTE_COVERAGE_MARKER_OPS&budget=200&k=10")
        .unwrap_or_else(|e| panic!("recall failed: {e}"));
    assert_eq!(recall.status, 200, "body {}", recall.body);

    let digest2 = daemon.get("/digest").expect("digest after store");
    assert_eq!(digest2.body["totals"]["decisions"], 1, "body {}", digest2.body);
    assert_eq!(digest2.body["totals"]["memories"], 0, "plain stores do not write memories: {}", digest2.body);
    assert_eq!(digest2.body["today"]["newDecisions"], 1, "body {}", digest2.body);
    assert_eq!(digest2.body["today"]["stores"], 1, "decision_stored event counted: {}", digest2.body);

    let stats2 = daemon.get("/stats").expect("stats after recall");
    assert_eq!(stats2.body["queries"], 1, "exactly one recall_query event: {}", stats2.body);
    assert!(stats2.body["spent"].as_i64().expect("spent") > 0, "body {}", stats2.body);

    let storage2 = daemon.get("/storage").expect("storage after store");
    let rows: Vec<(String, i64)> = storage2.body["tables"]
        .as_array()
        .expect("tables")
        .iter()
        .map(|t| (t["table"].as_str().expect("name").to_string(), t["rows"].as_i64().expect("rows")))
        .collect();
    assert_eq!(rows.iter().find(|(n, _)| n == "decisions"), Some(&("decisions".to_string(), 1)), "{}", storage2.body);
    assert_eq!(rows.iter().find(|(n, _)| n == "feed"), Some(&("feed".to_string(), 0)), "{}", storage2.body);

    // --- POST /compact ---
    let compact = daemon.post("/compact", json!({})).expect("compact");
    assert_eq!(compact.status, 200, "body {}", compact.body);
    let expected_compact_keys = [
        "eventsPruned",
        "benchmarkPruned",
        "archivedTextStripped",
        "expiredPruned",
        "crystalEmbeddingsPruned",
        "clusterMembersPruned",
        "feedbackAggregated",
        "staleEmbeddingsPruned",
        "coOccurrencePruned",
        "legacyEmbeddingsMigrated",
        "ftsOptimized",
        "bytesBefore",
        "bytesAfter",
        "savedKB",
        "failures",
    ];
    for key in expected_compact_keys {
        assert!(compact.body.get(key).is_some(), "compact must report {key}: {}", compact.body);
    }
    assert_eq!(compact.body["failures"], json!([]), "fresh compaction has no failures: {}", compact.body);
    let before = compact.body["bytesBefore"].as_i64().expect("bytesBefore");
    let after = compact.body["bytesAfter"].as_i64().expect("bytesAfter");
    assert!(before >= after, "bytes {before} -> {after}");
    assert_eq!(compact.body["savedKB"], (before - after) / 1024, "savedKB arithmetic: {}", compact.body);

    // --- POST /compact/benchmark purges benchmark-sourced decisions ---
    daemon.store_decision_as("amb-cortex-bench", &marker("BENCH"));
    let bench = daemon.post("/compact/benchmark", json!({})).expect("compact benchmark");
    assert_eq!(bench.status, 200, "body {}", bench.body);
    for key in [
        "decisionsDeleted",
        "embeddingsDeleted",
        "clusterMembersDeleted",
        "decisionConflictsDeleted",
        "recallFeedbackDeleted",
        "coOccurrenceDeleted",
        "eventsDeleted",
        "bytesBefore",
        "bytesAfter",
        "savedKB",
        "failures",
    ] {
        assert!(bench.body.get(key).is_some(), "benchmark purge must report {key}: {}", bench.body);
    }
    assert_eq!(bench.body["decisionsDeleted"], 1, "the amb-cortex-bench decision is purged: {}", bench.body);
    assert_eq!(bench.body["failures"], json!([]), "body {}", bench.body);
    let storage3 = daemon.get("/storage").expect("storage after benchmark purge");
    let decisions_rows: i64 = storage3.body["tables"]
        .as_array()
        .expect("tables")
        .iter()
        .filter(|t| t["table"] == "decisions")
        .filter_map(|t| t["rows"].as_i64())
        .next()
        .expect("decisions row");
    assert_eq!(decisions_rows, 1, "only the non-benchmark decision remains: {}", storage3.body);
}

#[test]
fn wave3_boot_audit_pins_row_shape_and_default_retention() {
    let daemon = spawn("rcov_boot_audit");
    let agent = "rcov-boot-agent";

    let fresh = daemon.get(&format!("/boot/audit?agent={agent}")).expect("boot audit fresh");
    assert_eq!(fresh.status, 200, "body {}", fresh.body);
    assert_eq!(fresh.body, json!({"audits": [], "count": 0, "retention_days": 90}), "body {}", fresh.body);

    // Boot without the default X-Source-Agent header so the query agent is recorded.
    let boot = daemon.get_without_source_agent(&format!("/boot?agent={agent}&budget=420")).expect("boot");
    assert_eq!(boot.status, 200, "body {}", boot.body);

    let audited = daemon.get(&format!("/boot/audit?agent={agent}")).expect("boot audit after boot");
    assert_eq!(audited.status, 200, "body {}", audited.body);
    assert_eq!(audited.body["count"], 1, "body {}", audited.body);
    assert_eq!(audited.body["retention_days"], 90, "default retention: {}", audited.body);
    let audits = audited.body["audits"].as_array().expect("audits array");
    assert_eq!(audits.len(), 1, "body {}", audited.body);
    assert_eq!(audits[0]["agent"], agent, "audit {}", audits[0]);
    assert_eq!(audits[0]["profile"], "full", "default profile: {}", audits[0]);
    assert_eq!(audits[0]["budget_tokens"], 420, "budget echo: {}", audits[0]);
    assert!(audits[0]["capsules_count"].is_i64(), "capsules_count int: {}", audits[0]);
    assert!(audits[0]["latency_ms"].is_i64(), "latency_ms int: {}", audits[0]);
    assert!(audits[0]["created_at"].as_str().is_some(), "created_at string: {}", audits[0]);
}

/// GET /conflicts, POST /resolve, POST /conflicts/resolve, POST /archive, POST /diary.
/// /conflicts and /archive are FACADE PINs (see module docs).
#[test]
fn wave3_conflicts_resolve_archive_diary_contract() {
    let daemon = spawn("rcov_mutate");

    // FACADE PIN: GET /conflicts returns this exact constant payload; the DB
    // connection is unused by the handler (list_conflicts_payload ignores it).
    let conflicts = daemon.get("/conflicts").expect("conflicts");
    assert_eq!(conflicts.status, 200, "body {}", conflicts.body);
    assert_eq!(
        conflicts.body,
        json!({
            "statusFilter": "open",
            "classificationFilter": null,
            "conflictIdFilter": null,
            "openCount": 0,
            "resolvedCount": 0,
            "count": 0,
            "pairs": [],
            "conflicts": [],
            "conflict": null
        }),
        "FACADE PIN: /conflicts payload is a constant today: {}",
        conflicts.body
    );

    let bad_status = daemon.get("/conflicts?status=bogus").expect("conflicts bad filter");
    assert_eq!(bad_status.status, 400, "body {}", bad_status.body);
    assert_eq!(
        bad_status.body["error"],
        "Invalid status filter. Expected open, resolved, or all.",
        "body {}",
        bad_status.body
    );

    let keep_a = daemon.store_decision(&marker("RESOLVE_A"), None);
    let keep_b = daemon.store_decision(&marker("RESOLVE_B"), None);

    let missing = daemon.post("/resolve", json!({})).expect("resolve missing fields");
    assert_eq!(missing.status, 400, "body {}", missing.body);
    assert_eq!(missing.body["error"], "Missing fields: keepId, action", "body {}", missing.body);

    let bad_action = daemon
        .post("/resolve", json!({"keepId": keep_a, "action": "explode"}))
        .expect("resolve bad action");
    assert_eq!(bad_action.status, 500, "honest pin: invalid action is a 500 today, body {}", bad_action.body);
    assert_eq!(
        bad_action.body["error"],
        "Invalid action. Expected keep, merge, or archive.",
        "body {}",
        bad_action.body
    );

    let resolved = daemon
        .post("/resolve", json!({"keepId": keep_a, "action": "keep", "supersededId": keep_b}))
        .expect("resolve");
    assert_eq!(resolved.status, 200, "body {}", resolved.body);
    assert_eq!(
        resolved.body,
        json!({"resolved": true, "keepId": keep_a, "winnerId": keep_a, "supersededId": keep_b, "action": "keep"}),
        "body {}",
        resolved.body
    );

    // /conflicts/resolve is the same handler aliased; same contract.
    let keep_c = daemon.store_decision(&marker("RESOLVE_C"), None);
    let via_alias = daemon
        .post("/conflicts/resolve", json!({"keepId": keep_c, "action": "archive"}))
        .expect("resolve alias");
    assert_eq!(via_alias.status, 200, "body {}", via_alias.body);
    assert_eq!(
        via_alias.body,
        json!({"resolved": true, "keepId": keep_c, "winnerId": keep_c, "supersededId": null, "action": "archive"}),
        "body {}",
        via_alias.body
    );

    // FACADE PIN: /archive echoes the request and has zero DB effect. The
    // handler is a pure echo by construction; observably, the decisions table
    // row count is unchanged after the call.
    let arch_ctx = "rcov-archive-context-31b7";
    let arch_text = marker("ARCHIVE");
    let arch_id = daemon.store_decision(&arch_text, Some(arch_ctx));
    let storage_before = daemon.get("/storage").expect("storage before archive");
    let decisions_before = table_rows(&storage_before.body, "decisions");
    let archived = daemon.post("/archive", json!({"table": "decisions", "ids": [arch_id]})).expect("archive");
    assert_eq!(archived.status, 200, "body {}", archived.body);
    assert_eq!(
        archived.body,
        json!({"archived": 1, "table": "decisions"}),
        "FACADE PIN: /archive echoes counts without touching the DB: {}",
        archived.body
    );
    let storage_after = daemon.get("/storage").expect("storage after archive");
    assert_eq!(
        table_rows(&storage_after.body, "decisions"),
        decisions_before,
        "FACADE proof: decisions row count unchanged by /archive"
    );

    // POST /diary writes ~/.claude/state.md under the daemon home.
    let diary = daemon
        .post(
            "/diary",
            json!({"accomplished": "Rcov accomplished item", "nextSteps": "Rcov next steps", "keyDecisions": "Rcov decision one"}),
        )
        .expect("diary");
    assert_eq!(diary.status, 200, "body {}", diary.body);
    assert_eq!(diary.body["written"], true, "body {}", diary.body);
    assert_eq!(diary.body["agent"], "adapter-conformance", "agent from X-Source-Agent: {}", diary.body);
    let path = diary.body["path"].as_str().expect("path string");
    assert!(path.ends_with("/.claude/state.md"), "path {path}");
    let state_md = fs::read_to_string(daemon.home_dir.join(".claude/state.md")).expect("read state.md");
    assert!(state_md.starts_with("# Session State — "), "state.md {state_md}");
    assert!(state_md.contains("## What Was Done This Session\nRcov accomplished item"), "state.md {state_md}");
    assert!(state_md.contains("## Next Session\nRcov next steps"), "state.md {state_md}");
    assert!(state_md.contains("## Key Decisions\nRcov decision one"), "state.md {state_md}");
}

/// FACADE PIN — the whole conductor family. Every POST mints a UUID response
/// with no persistence, proven by the matching GET list staying empty and the
/// lifecycle accepting nonexistent ids. Pins CURRENT behavior until bead
/// cortex-071's product decision lands.
#[test]
fn wave3_conductor_family_facade_pins() {
    let daemon = spawn("rcov_conductor");

    // --- /lock + /locks + /unlock ---
    let lock = daemon
        .post("/lock", json!({"path": "/repo/file.rs", "agent": "rcov-a"}))
        .expect("lock");
    assert_eq!(lock.status, 200, "body {}", lock.body);
    assert_eq!(lock.body["locked"], true, "body {}", lock.body);
    let lock_id = lock.body["lockId"].as_str().expect("lockId string").to_string();
    assert_eq!(lock_id.len(), 36, "lockId is a minted UUID: {lock_id}");
    let expires = lock.body["expiresAt"].as_str().expect("expiresAt string");
    // Default TTL 300s: RFC3339 timestamp within [now+4m, now+6m].
    let expires_secs = chrono_parse_unix(expires);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64;
    assert!(
        expires_secs >= now + 240 && expires_secs <= now + 360,
        "default lock TTL ~300s, got {expires} (now={now})"
    );

    let locks = daemon.get("/locks").expect("locks");
    assert_eq!(locks.status, 200, "body {}", locks.body);
    assert_eq!(locks.body, json!({"locks": []}), "FACADE PIN: minted lock is not listed: {}", locks.body);

    let no_fields = daemon.post("/lock", json!({"path": "/repo/file.rs"})).expect("lock missing agent");
    assert_eq!(no_fields.status, 400, "body {}", no_fields.body);
    assert_eq!(no_fields.body["error"], "Missing required fields: path, agent", "body {}", no_fields.body);

    let unlock = daemon
        .post("/unlock", json!({"path": "/never/locked.rs", "agent": "rcov-a"}))
        .expect("unlock");
    assert_eq!(unlock.status, 200, "body {}", unlock.body);
    assert_eq!(unlock.body, json!({"unlocked": true}), "FACADE PIN: unlock succeeds for never-locked path: {}", unlock.body);

    // --- /activity ---
    let activity = daemon
        .post("/activity", json!({"agent": "rcov-a", "description": "Rcov activity"}))
        .expect("post activity");
    assert_eq!(activity.status, 200, "body {}", activity.body);
    assert_eq!(activity.body["recorded"], true, "body {}", activity.body);
    assert_eq!(activity.body["activityId"].as_str().expect("activityId").len(), 36, "body {}", activity.body);
    let activities = daemon.get("/activity").expect("get activity");
    assert_eq!(activities.body, json!({"activities": []}), "FACADE PIN: activity list stays empty: {}", activities.body);

    // --- /message + /messages ---
    let message = daemon
        .post("/message", json!({"from": "rcov-a", "to": "rcov-b", "message": "Rcov hello"}))
        .expect("post message");
    assert_eq!(message.status, 200, "body {}", message.body);
    assert_eq!(message.body["sent"], true, "body {}", message.body);
    assert_eq!(message.body["messageId"].as_str().expect("messageId").len(), 36, "body {}", message.body);
    let messages = daemon.get("/messages").expect("get messages");
    assert_eq!(messages.body, json!({"messages": []}), "FACADE PIN: sent message is not listed: {}", messages.body);

    // --- /session/* + /sessions ---
    let session = daemon.post("/session/start", json!({"agent": "rcov-a"})).expect("session start");
    assert_eq!(session.status, 200, "body {}", session.body);
    assert_eq!(session.body["sessionId"].as_str().expect("sessionId").len(), 36, "body {}", session.body);
    assert_eq!(session.body["heartbeatInterval"], 60, "body {}", session.body);
    assert_eq!(session.body["freshened"], false, "body {}", session.body);

    let heartbeat = daemon.post("/session/heartbeat", json!({"agent": "rcov-a"})).expect("heartbeat");
    assert_eq!(heartbeat.status, 200, "body {}", heartbeat.body);
    assert_eq!(heartbeat.body["renewed"], true, "body {}", heartbeat.body);
    let hb_expires = chrono_parse_unix(heartbeat.body["expiresAt"].as_str().expect("expiresAt"));
    assert!(hb_expires > now, "heartbeat expiry in the future: {}", heartbeat.body);

    let heartbeat_no_agent = daemon.post("/session/heartbeat", json!({})).expect("heartbeat no agent");
    assert_eq!(heartbeat_no_agent.status, 400, "body {}", heartbeat_no_agent.body);
    assert_eq!(
        heartbeat_no_agent.body["error"],
        "Missing or invalid required field: agent",
        "body {}",
        heartbeat_no_agent.body
    );

    let session_end = daemon.post("/session/end", json!({"agent": "rcov-a"})).expect("session end");
    assert_eq!(session_end.status, 200, "body {}", session_end.body);
    assert_eq!(session_end.body, json!({"ended": true}), "body {}", session_end.body);

    let sessions = daemon.get("/sessions").expect("sessions");
    assert_eq!(sessions.body, json!({"sessions": []}), "FACADE PIN: session list stays empty: {}", sessions.body);

    // --- /tasks family ---
    let created = daemon.post("/tasks", json!({"title": "Rcov task"})).expect("create task");
    assert_eq!(created.status, 201, "task creation is 201 CREATED, body {}", created.body);
    assert_eq!(created.body["status"], "pending", "body {}", created.body);
    assert_eq!(created.body["taskId"].as_str().expect("taskId").len(), 36, "body {}", created.body);

    let tasks = daemon.get("/tasks").expect("get tasks");
    assert_eq!(tasks.body, json!({"tasks": []}), "FACADE PIN: created task is not listed: {}", tasks.body);
    let next = daemon.get("/tasks/next").expect("next task");
    assert_eq!(next.body, json!({"task": null}), "FACADE PIN: next task is always null: {}", next.body);

    for (path, field) in [
        ("/tasks/claim", "claimed"),
        ("/tasks/complete", "completed"),
        ("/tasks/abandon", "abandoned"),
    ] {
        let ack = daemon
            .post(path, json!({"taskId": "rcov-nonexistent-task", "agent": "rcov-a"}))
            .unwrap_or_else(|e| panic!("{path} failed: {e}"));
        assert_eq!(ack.status, 200, "body {}", ack.body);
        let mut expected = json!({"taskId": "rcov-nonexistent-task"});
        expected[field] = json!(true);
        assert_eq!(
            ack.body,
            expected,
            "FACADE PIN: {path} acks a nonexistent task: {}",
            ack.body
        );
    }

    let deleted = daemon
        .post("/tasks/delete", json!({"taskId": "rcov-nonexistent-task"}))
        .expect("delete task");
    assert_eq!(deleted.status, 200, "body {}", deleted.body);
    assert_eq!(
        deleted.body,
        json!({"deleted": true, "taskId": "rcov-nonexistent-task"}),
        "FACADE PIN: delete acks a nonexistent task: {}",
        deleted.body
    );
}

fn table_rows(storage_body: &Value, table: &str) -> i64 {
    storage_body["tables"]
        .as_array()
        .expect("tables array")
        .iter()
        .find(|t| t["table"] == table)
        .and_then(|t| t["rows"].as_i64())
        .unwrap_or_else(|| panic!("table {table} missing from storage breakdown: {storage_body}"))
}

/// Extract unix seconds from an RFC3339 timestamp without pulling a chrono
/// dependency into the test crate: parse the date-time fields directly.
fn chrono_parse_unix(rfc3339: &str) -> i64 {
    let (date, time) = rfc3339
        .split_once('T')
        .unwrap_or_else(|| panic!("expected RFC3339 timestamp, got {rfc3339}"));
    let date_parts: Vec<i64> = date.split('-').filter_map(|p| p.parse().ok()).collect();
    let time = time.trim_end_matches('Z');
    let time_parts: Vec<i64> = time.split(':').filter_map(|p| p.parse().ok()).collect();
    assert!(date_parts.len() == 3 && time_parts.len() >= 2, "unexpected timestamp {rfc3339}");
    let (year, month, day) = (date_parts[0], date_parts[1], date_parts[2]);
    // Days-from-civil algorithm (Howard Hinnant) for UTC.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + time_parts[0] * 3_600 + time_parts[1] * 60
}
