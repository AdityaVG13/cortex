//! Threads, obligations, attempts: the workflow state machine is
//! checker-gated, verification is bound to an artifact, attempts answer a
//! failure query without a transcript, and a Thread survives a fresh
//! process opening the same database.

use cortex_daemon::db::threads::{OBLIGATION_STATES, transition_allowed};
use cortex_daemon::handlers::operations::{Caller, Operation, dispatch};
use cortex_daemon::runtime::CortexRuntime;
use cortex_tests::support::solo_state;
use serde_json::json;

fn caller() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "thread-agent",
        principal: "solo".into(),
    }
}

#[test]
fn obligation_state_machine_never_reaches_verified_by_transition() {
    for from in OBLIGATION_STATES {
        assert!(
            !transition_allowed(from, "verified_complete"),
            "{from} -> verified_complete must need a checker receipt"
        );
        assert!(
            !transition_allowed("cancelled", from) || from == "cancelled",
            "cancelled is terminal"
        );
    }
    assert!(transition_allowed("proposed", "ready"));
    assert!(transition_allowed("in_progress", "observed_complete"));
    assert!(transition_allowed("verified_complete", "reopened"));
    assert!(
        !transition_allowed("proposed", "observed_complete"),
        "no completion claim without work in progress"
    );
}

#[test]
fn verification_needs_the_registered_predicate_and_a_passing_checker_and_is_bound_to_the_artifact()
{
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let created = dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "payments retry", "action": "obligation", "title": "exercise timeout-after-commit", "predicate": {"predicate": "crash_after_commit_regression"}})).await.unwrap();
        assert_eq!(created["status"], "ok", "{created}");
        let obligation = created["obligation"].as_str().unwrap().to_string();
        let t = |to: &str| json!({"thread": "payments retry", "action": "transition", "obligation": &obligation, "to": to});
        let early = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Checkpoint,
            &t("verified_complete"),
        )
        .await
        .unwrap();
        assert_eq!(
            early["status"], "invalid_request",
            "a transition can never verify: {early}"
        );
        assert_eq!(
            dispatch(
                &cx,
                &state,
                caller(),
                Operation::Checkpoint,
                &t("in_progress")
            )
            .await
            .unwrap()["status"],
            "ok"
        );
        assert_eq!(
            dispatch(
                &cx,
                &state,
                caller(),
                Operation::Checkpoint,
                &t("observed_complete")
            )
            .await
            .unwrap()["status"],
            "ok"
        );
        let wrong_predicate = dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "payments retry", "action": "verify", "obligation": &obligation, "predicate": "tests_passed", "artifact": "a7", "checker": "cargo", "passed": true})).await.unwrap();
        assert_eq!(
            wrong_predicate["status"], "invalid_request",
            "a generic 'tests passed' does not satisfy the named predicate: {wrong_predicate}"
        );
        let failing = dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "payments retry", "action": "verify", "obligation": &obligation, "predicate": "crash_after_commit_regression", "artifact": "a7", "checker": "cargo", "passed": false})).await.unwrap();
        assert_eq!(
            failing["status"], "invalid_request",
            "a failing run is never remembered as verified: {failing}"
        );
        let verified = dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "payments retry", "action": "verify", "obligation": &obligation, "predicate": "crash_after_commit_regression", "artifact": "a7", "checker": "cargo", "passed": true})).await.unwrap();
        assert_eq!(verified["status"], "ok", "{verified}");
        assert_eq!(verified["state"], "verified_complete");
        let same = dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "payments retry", "action": "revalidate", "obligation": &obligation, "artifact": "a7"})).await.unwrap();
        assert_eq!(
            same["reopened"], false,
            "same artifact keeps the verification: {same}"
        );
        let moved = dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "payments retry", "action": "revalidate", "obligation": &obligation, "artifact": "a8"})).await.unwrap();
        assert_eq!(
            moved["reopened"], true,
            "a new artifact revision reopens without erasing: {moved}"
        );
        assert_eq!(moved["state"], "reopened");
        let conn = state.db.lock(&cx).await.expect("lock");
        let revisions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM revisions WHERE record_id = ?1",
                [&obligation],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            revisions >= 5,
            "every claim and verification is a preserved revision: {revisions}"
        );
        let verified_rev: Option<String> = conn
            .query_row(
                "SELECT verification_revision FROM obligations WHERE record_id = ?1",
                [&obligation],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            verified_rev.is_some(),
            "the earlier verification revision is kept for inspection"
        );
    });
}

#[test]
fn attempts_answer_a_failure_query_without_a_transcript_and_threads_survive_reopen() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let dir = tempfile::Builder::new()
            .prefix("cortex-threads-")
            .tempdir()
            .unwrap();
        let db = dir.path().join("cortex.db");
        let runtime = CortexRuntime::open_db(&db).unwrap();
        let state = runtime.state().clone();
        let attempt = dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "payments retry", "action": "attempt", "attempt": {"inputs": {"retries": 5}, "environment": {"branch": "retry-fix", "artifact": "a7"}, "procedure": "increase retry count", "exit_status": 1, "failure": "duplicate charge under timeout-after-commit", "text": "increasing retry count failed under timeout-after-commit"}})).await.unwrap();
        assert_eq!(attempt["status"], "ok", "{attempt}");
        dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "payments retry", "action": "checkpoint", "goal": "restore requests without duplicate charges", "state": {"blockers": ["timeout-after-commit"]}})).await.unwrap();
        drop(runtime);
        drop(state);
        // Fresh process/handle over the same file: no in-memory state carried.
        let reopened = CortexRuntime::open_db(&db).unwrap();
        let status = dispatch(
            &cx,
            reopened.state(),
            caller(),
            Operation::Checkpoint,
            &json!({"thread": "payments retry", "action": "status"}),
        )
        .await
        .unwrap();
        assert_eq!(status["thread"]["exists"], true, "{status}");
        let attempts = status["thread"]["attempts"].as_array().unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0]["kind"], "failure");
        assert_eq!(
            attempts[0]["body"]["failure"],
            "duplicate charge under timeout-after-commit"
        );
        assert_eq!(attempts[0]["body"]["environment"]["artifact"], "a7");
        assert!(
            attempts[0]["body"].get("transcript").is_none()
                && attempts[0]["body"].get("reasoning").is_none(),
            "no reasoning transcript is retained"
        );
        assert_eq!(
            status["thread"]["checkpoint"]["goal"],
            "restore requests without duplicate charges"
        );
        let orient = dispatch(
            &cx,
            reopened.state(),
            caller(),
            Operation::Orient,
            &json!({"task": "retry", "thread": "payments retry"}),
        )
        .await
        .unwrap();
        assert_eq!(
            orient["thread"]["checkpoint"]["state"]["blockers"][0], "timeout-after-commit",
            "orient carries the Thread state: {orient}"
        );
    });
}

#[test]
fn orient_with_thread_returns_a_self_contained_continuation() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        dispatch(&cx, &state, caller(), Operation::Commit, &json!({"entries": [{"kind": "constraint", "text": "CONT-1 constraint: preserve the existing idempotency contract"}]})).await.unwrap();
        let created = dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "cont", "action": "obligation", "title": "exercise timeout-after-commit", "predicate": {"predicate": "p"}})).await.unwrap();
        let obligation = created["obligation"].as_str().unwrap().to_string();
        dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "cont", "action": "transition", "obligation": &obligation, "to": "in_progress"})).await.unwrap();
        dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "cont", "action": "attempt", "attempt": {"procedure": "raise retries", "exit_status": 1, "failure": "duplicate charge"}})).await.unwrap();
        dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "cont", "action": "checkpoint", "goal": "restore without duplicate charges", "state": {"blockers": ["timeout-after-commit"]}})).await.unwrap();
        let orient = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Orient,
            &json!({"task": "CONT-1 idempotency contract", "thread": "cont"}),
        )
        .await
        .unwrap();
        let c = &orient["continuation"];
        assert_eq!(c["self_contained"], true, "{orient}");
        assert_eq!(c["goal"], "restore without duplicate charges");
        assert_eq!(c["constraints"][0]["label"], "c1", "{orient}");
        assert_eq!(c["unfinished_work"][0]["record"], obligation);
        assert_eq!(
            c["failed_attempts"][0]["body"]["failure"],
            "duplicate charge"
        );
        assert_eq!(c["blockers"]["checkpoint"][0], "timeout-after-commit");
        assert!(
            c["evidence_needs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n == "conflicts" || n == "failed_attempts" || n == "open_obligations")
                || c["evidence_needs"].as_array().unwrap().is_empty()
        );
    });
}

/// Team mode: one owner's capsule never carries another owner's tasks,
/// messages or decisions.
#[test]
fn team_boot_capsules_are_owner_scoped() {
    use cortex_tests::support::test_conn;
    let conn = test_conn();
    cortex_daemon::db::create_team_mode_tables(&conn).unwrap();
    let owner =
        cortex_daemon::db::upsert_owner_user(&conn, "owner", Some("owner"), "hash-owner").unwrap();
    cortex_daemon::db::migrate_to_team_mode(&conn, owner).unwrap();
    conn.execute("INSERT INTO users (username, display_name, api_key_hash, role) VALUES ('other', 'other', 'x', 'member')", []).unwrap();
    let other: i64 = conn
        .query_row("SELECT id FROM users WHERE username = 'other'", [], |r| {
            r.get(0)
        })
        .unwrap();
    conn.execute("INSERT INTO tasks (task_id, title, files_json, priority, required_capability, status, created_at, owner_id) VALUES ('t-mine', 'mine', '[]', 'high', 'any', 'pending', '2026-09-05T00:00:00Z', ?1)", [owner]).unwrap();
    conn.execute("INSERT INTO tasks (task_id, title, files_json, priority, required_capability, status, created_at, owner_id) VALUES ('t-theirs', 'theirs SECRET-TASK', '[]', 'high', 'any', 'pending', '2026-09-05T00:00:00Z', ?1)", [other]).unwrap();
    conn.execute("INSERT INTO messages (id, sender, recipient, message, timestamp, owner_id) VALUES ('m1', 'x', 'agent-1', 'SECRET-MESSAGE', '2026-09-05T00:00:00Z', ?1)", [other]).unwrap();
    let home = std::env::temp_dir();
    let mine =
        cortex_daemon::compiler::compile_for_owner(&conn, &home, "agent-1", 4000, Some(owner));
    assert!(mine.boot_prompt.contains("mine"), "{}", mine.boot_prompt);
    assert!(
        !mine.boot_prompt.contains("SECRET-TASK"),
        "another owner's task leaked: {}",
        mine.boot_prompt
    );
    assert!(
        !mine.boot_prompt.contains("SECRET-MESSAGE"),
        "another owner's message leaked: {}",
        mine.boot_prompt
    );
    let theirs =
        cortex_daemon::compiler::compile_for_owner(&conn, &home, "agent-1", 4000, Some(other));
    assert!(
        theirs.boot_prompt.contains("SECRET-TASK") && theirs.boot_prompt.contains("SECRET-MESSAGE"),
        "{}",
        theirs.boot_prompt
    );
}
