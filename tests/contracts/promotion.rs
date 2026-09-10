//! Promotion is type-specific: a preference needs one authenticated user
//! statement; a checker result one passing predicate on one artifact; a
//! procedure independent successful cases with matching preconditions
//! (copies are not trials) and carries its counterexamples; a cross-project
//! lesson explicit widening, authority and a disconfirming check. Cases
//! travel with their nearest counterexample.

use cortex_daemon::handlers::operations::{dispatch, Caller, Operation};
use cortex_tests::support::solo_state;
use serde_json::json;

fn user() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "human",
        principal: "user:aditya".into(),
    }
}
fn agent(name: &'static str) -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: name,
        principal: "agent:x".into(),
    }
}

#[test]
fn promotion_rules_are_type_specific_and_carry_population_and_exclusions() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        // Preference: authenticated user statement yes, agent assertion no.
        let pref = dispatch(
            cx,
            &state,
            user(),
            Operation::Commit,
            &json!({"promote": {"rule": "preference", "text": "prefer tabs in Makefiles"}}),
        )
        .await
        .unwrap();
        assert_eq!(pref["status"], "ok", "{pref}");
        assert_eq!(pref["body"]["authority"], "user:aditya");
        let agent_pref = dispatch(
            cx,
            &state,
            agent("bot"),
            Operation::Commit,
            &json!({"promote": {"rule": "preference", "text": "prefer spaces"}}),
        )
        .await
        .unwrap();
        assert_eq!(
            agent_pref["status"], "invalid_request",
            "an agent cannot mint a user preference: {agent_pref}"
        );
        // Checker result: one predicate on one artifact; failing runs establish nothing.
        let failed = dispatch(cx, &state, agent("cargo"), Operation::Commit, &json!({"promote": {"rule": "checker_result", "text": "regression passes", "preconditions": {"predicate": "crash_after_commit", "artifact": "a7", "checker": "cargo", "passed": false}}})).await.unwrap();
        assert_eq!(failed["status"], "invalid_request", "{failed}");
        let verified = dispatch(cx, &state, agent("cargo"), Operation::Commit, &json!({"promote": {"rule": "checker_result", "text": "regression passes on a7", "preconditions": {"predicate": "crash_after_commit", "artifact": "a7", "checker": "cargo", "passed": true}}})).await.unwrap();
        assert_eq!(verified["status"], "ok", "{verified}");
        assert!(verified["body"]["revocation_triggers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t.as_str().unwrap().contains("artifact")));
        // Procedure: two copies of one report are one trial.
        let pre = json!({"schema": "X", "tx_mode": "Y"});
        for _ in 0..2 {
            dispatch(cx, &state, agent("alice"), Operation::Commit, &json!({"entries": [{"kind": "case", "text": "PROC-1 rollback via savepoint recovered the ledger", "fields": {"preconditions": pre, "outcome": "success", "action": "savepoint rollback"}}]})).await.unwrap();
        }
        let copies = dispatch(cx, &state, agent("alice"), Operation::Commit, &json!({"promote": {"rule": "procedure", "text": "roll back via savepoint", "preconditions": pre}})).await.unwrap();
        assert_eq!(
            copies["status"], "invalid_request",
            "copied successes are not independent trials: {copies}"
        );
        dispatch(cx, &state, agent("bob"), Operation::Commit, &json!({"entries": [{"kind": "case", "text": "PROC-1 savepoint rollback also recovered the audit ledger in staging", "fields": {"preconditions": pre, "outcome": "success", "action": "savepoint rollback"}}]})).await.unwrap();
        dispatch(cx, &state, agent("carol"), Operation::Commit, &json!({"entries": [{"kind": "counterexample", "text": "PROC-1 savepoint rollback lost rows under schema X with WAL disabled", "fields": {"preconditions": pre, "outcome": "failure"}}]})).await.unwrap();
        let proc_ = dispatch(cx, &state, agent("alice"), Operation::Commit, &json!({"promote": {"rule": "procedure", "text": "roll back via savepoint", "preconditions": pre}})).await.unwrap();
        assert_eq!(proc_["status"], "ok", "{proc_}");
        assert_eq!(
            proc_["body"]["population"].as_array().unwrap().len(),
            2,
            "independent successes only: {proc_}"
        );
        assert_eq!(
            proc_["body"]["exclusions"].as_array().unwrap().len(),
            1,
            "the counterexample travels as an exclusion: {proc_}"
        );
        // Nearest counterexample travels with the case in a View.
        let view = dispatch(
            cx,
            &state,
            agent("dave"),
            Operation::Query,
            &json!({"need": "PROC-1 savepoint rollback", "profile": "procedures", "budget": 8000}),
        )
        .await
        .unwrap();
        let case_card = view["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["sidecar"]["kind"] == "case")
            .unwrap_or_else(|| panic!("{view}"));
        assert!(
            case_card["exceptions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e.as_str().unwrap().starts_with("counterexample")),
            "{view}"
        );
        // Cross-project lesson: explicit widening, authority, disconfirming check.
        let implicit = dispatch(cx, &state, user(), Operation::Commit, &json!({"promote": {"rule": "lesson", "text": "savepoints beat manual undo", "sources": [proc_["promoted"]]}})).await.unwrap();
        assert_eq!(
            implicit["status"], "invalid_request",
            "widening is never implicit: {implicit}"
        );
        dispatch(cx, &state, agent("erin"), Operation::Commit, &json!({"entries": [{"kind": "counterexample", "text": "in project ops savepoints deadlocked the migrator", "fields": {"scope": "project:ops"}}]})).await.unwrap();
        let blocked = dispatch(cx, &state, user(), Operation::Commit, &json!({"promote": {"rule": "lesson", "text": "savepoints beat manual undo", "sources": [proc_["promoted"]], "target_scope": "project:ops", "authority": "user:aditya"}})).await.unwrap();
        assert_eq!(
            blocked["status"], "invalid_request",
            "a disconfirming case in the target scope blocks: {blocked}"
        );
        let lesson = dispatch(cx, &state, user(), Operation::Commit, &json!({"promote": {"rule": "lesson", "text": "savepoints beat manual undo", "sources": [proc_["promoted"]], "target_scope": "project:billing", "authority": "user:aditya"}})).await.unwrap();
        assert_eq!(lesson["status"], "ok", "{lesson}");
        assert_eq!(lesson["body"]["scope"], "project:billing");
    });
}
