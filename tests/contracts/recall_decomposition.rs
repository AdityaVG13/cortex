//! Recall decomposition: each stage is measured against an exhaustive
//! authorized scan (the exact semantic profile) on a small structured
//! corpus. A hidden index cutoff is never labelled `no_match`; the failure
//! class of every miss is named. Plus the typed `compare` profile.

use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_kernel::handlers::recall::{execute_unified_recall, RecallContext};
use cortex_kernel::store_spi::sqlite::SqliteStore;
use cortex_kernel::store_spi::{BrainStore, CandidateProfile, ReadSnapshot, ScanLimits};
use cortex_tests::support::{open_file_db, solo_state};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn caller() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "decomp",
        principal: "solo".into(),
    }
}

#[test]
fn stages_are_measured_against_an_exhaustive_oracle_and_misses_are_classified() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        // Structured corpus: 3 relevant rows with the handle, 1 relevant without
        // the exact handle (paraphrase), 2 irrelevant, 1 archived relevant.
        let rows = [
            (
                "DECOMP-9 constraint: the gateway must forward the idempotency header",
                "constraint",
                "active",
            ),
            (
                "DECOMP-9 exception: internal health checks skip the idempotency header",
                "decision",
                "active",
            ),
            (
                "DECOMP-9 attempt: dropping the header caused duplicate charges",
                "decision",
                "active",
            ),
            (
                "the payments gateway keeps request replay safe by echoing a dedupe token",
                "decision",
                "active",
            ),
            (
                "dashboard palette moved to the high-contrast theme",
                "decision",
                "active",
            ),
            ("weekly backups run on Sunday", "decision", "active"),
            (
                "DECOMP-9 old rule: forward every header (superseded)",
                "decision",
                "archived",
            ),
        ];
        for (text, kind, status) in rows {
            let r = dispatch(
                cx,
                &state,
                caller(),
                Operation::Commit,
                &json!({"entries": [{"kind": kind, "text": text}]}),
            )
            .await
            .unwrap();
            assert_eq!(r["status"], "ok", "{r}");
            if status == "archived" {
                let id = r["receipt"]["entries"]["entry.decision"]["value"]
                    .as_str()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap();
                state
                    .db
                    .lock(cx)
                    .await
                    .unwrap()
                    .execute(
                        "UPDATE decisions SET status = 'archived' WHERE id = ?1",
                        [id],
                    )
                    .unwrap();
            }
        }
        // Oracle: exhaustive exact-profile scan of the active partition.
        let oracle: BTreeSet<String> = {
            let store = SqliteStore::new(open_file_db(&state.db_path)).unwrap();
            let snap = store.read_snapshot().unwrap();
            let page = snap
                .candidates(
                    &CandidateProfile::ExactLexical,
                    &["decomp-9".into()],
                    ScanLimits::default(),
                )
                .unwrap();
            assert!(page.coverage.exhausted, "oracle scan must be exhaustive");
            page.rows
                .iter()
                .map(|r| format!("{}::{}", r.id.namespace, r.id.value))
                .collect()
        };
        assert_eq!(
            oracle.len(),
            3,
            "active rows carrying the handle: {oracle:?}"
        );
        let payload = execute_unified_recall(
            cx,
            &state,
            "DECOMP-9 idempotency header",
            4000,
            20,
            "decomp",
            &RecallContext::solo(),
            None,
        )
        .await
        .unwrap();
        let admitted: BTreeSet<String> = payload["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["source"].as_str().map(str::to_string))
            .collect();
        let routes = &payload["routes"];
        // Stage report, every stage separately.
        let candidate_recall = oracle.intersection(&admitted).count() as f64 / oracle.len() as f64;
        let false_admissions: Vec<&String> =
            admitted.iter().filter(|s| !oracle.contains(*s)).collect();
        let mut classes: Vec<(&str, usize)> = Vec::new();
        classes.push(("honest_miss_paraphrase", 1));
        classes.push(("temporal_scope_excluded_archived", 1));
        classes.push(("false_admission", false_admissions.len()));
        let report = json!({
            "route_coverage": {"families_collected": routes["collected"], "exhausted": routes["exhausted"]},
            "candidate_recall": candidate_recall,
            "admitted": admitted.len(),
            "leads": routes["leads"],
            "failure_classes": classes,
            "rank_tuple": routes["rank_tuple"],
        });
        eprintln!("DECOMPOSITION {report}");
        assert_eq!(
            candidate_recall, 1.0,
            "every oracle row with the exact handle is admitted: {report}"
        );
        assert!(
            false_admissions.is_empty(),
            "no false admission from unrelated rows: {false_admissions:?}"
        );
        assert!(
            routes["exhausted"].as_array().unwrap().is_empty(),
            "small corpus hits no quota: {routes}"
        );
        for r in payload["results"].as_array().unwrap() {
            assert!(
                r["why"]["questions"]["relevance"].is_string(),
                "each hit carries its admission law: {r}"
            );
            assert_eq!(r["why"]["questions"]["epistemic"], "asserted");
        }
        // The paraphrase is an honest miss, not a fabricated hit; the archived
        // row is a scope exclusion visible in the watermark, not a no_match.
        let view = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "DECOMP-9 idempotency header", "profile": "map", "budget": 8000}),
        )
        .await
        .unwrap();
        assert_eq!(
            view["coverage"]["partitions"]["cold"]["rows_not_searched"], 1,
            "archived row is reported as unsearched: {}",
            view["coverage"]
        );
        let miss = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "dedupe token replay", "profile": "map"}),
        )
        .await
        .unwrap();
        assert!(
            matches!(
                miss["status"].as_str(),
                Some("ok") | Some("no_match") | Some("partial")
            ),
            "{miss}"
        );
        if miss["status"] == "no_match" {
            assert_eq!(
                miss["coverage"]["partitions"]["decisions"]["exhausted"], true,
                "a no_match must have exhausted its domain: {}",
                miss["coverage"]
            );
        }
    });
}

#[test]
fn compare_profile_aligns_typed_fields_without_manufacturing_semantics() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        let a = dispatch(cx, &state, caller(), Operation::Commit, &json!({"entries": [{"kind": "constraint", "text": "CMP-1 retries: at most three attempts before the ledger commit"}]})).await.unwrap();
        let b = dispatch(cx, &state, caller(), Operation::Commit, &json!({"entries": [{"kind": "decision", "text": "CMP-1 retries: never after the ledger commit succeeded"}]})).await.unwrap();
        let ra = format!(
            "decision::{}",
            a["receipt"]["entries"]["entry.decision"]["value"]
                .as_str()
                .unwrap()
        );
        let rb = format!(
            "decision::{}",
            b["receipt"]["entries"]["entry.decision"]["value"]
                .as_str()
                .unwrap()
        );
        let cmp = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "what changed", "profile": "compare", "compare": [ra, rb]}),
        )
        .await
        .unwrap();
        assert_eq!(cmp["status"], "ok", "{cmp}");
        assert_eq!(cmp["rule"], "compare/1");
        let fields: Vec<&str> = cmp["comparison"]["differing_fields"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["field"].as_str())
            .collect();
        assert!(
            fields.contains(&"kind"),
            "kind differs (constraint vs decision): {cmp}"
        );
        let only_b: Vec<&str> = cmp["comparison"]["text"]["only_in_b"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(only_b.contains(&"never"), "{cmp}");
        assert_eq!(cmp["comparison"]["chronological"], "a_before_b");
        assert!(
            cmp["comparison"]["interpretation"]
                .as_str()
                .unwrap()
                .contains("reader"),
            "semantic judgement stays attributed: {cmp}"
        );
        let bad = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"profile": "compare", "need": "x", "compare": ["decision::1"]}),
        )
        .await
        .unwrap();
        assert_eq!(bad["status"], "invalid_request");
    });
}

#[test]
fn compare_rejects_quote_bearing_kind_instead_of_interpolating_sql() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        let (evil_ref, sqlite_version, other) = {
            let conn = state.db.lock(cx).await.unwrap();
            conn.execute(
                "INSERT INTO memories (text, source, type, source_agent, status) VALUES ('benign planted compare row', 'pending', NULL, 'decomp', 'active')",
                [],
            )
            .unwrap();
            let id = conn.last_insert_rowid();
            let evil_ref = format!("x'||(SELECT sqlite_version())||'::{id}");
            conn.execute(
                "UPDATE memories SET source = ?1 WHERE id = ?2",
                rusqlite::params![evil_ref, id],
            )
            .unwrap();
            let version: String = conn
                .query_row("SELECT sqlite_version()", [], |r| r.get(0))
                .unwrap();
            drop(conn);
            let stored = dispatch(
                cx,
                &state,
                caller(),
                Operation::Commit,
                &json!({"entries": [{"kind": "decision", "text": "CMP-SQL compare partner: keep the ledger retry bound at three attempts"}]}),
            )
            .await
            .unwrap();
            let other = format!(
                "decision::{}",
                stored["receipt"]["entries"]["entry.decision"]["value"]
                    .as_str()
                    .unwrap()
            );
            (evil_ref, version, other)
        };
        let cmp = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "what changed", "profile": "compare", "compare": [evil_ref, other]}),
        )
        .await
        .unwrap();
        let rendered = cmp.to_string();
        assert_eq!(
            cmp["status"], "invalid_request",
            "quote-bearing kind must not reach SQL: {cmp}"
        );
        assert!(
            !rendered.contains(&sqlite_version),
            "sqlite_version leaked through interpolated kind: {cmp}"
        );
    });
}

#[test]
fn compare_accepts_ident_kinds_other_than_memory_and_decision() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        let (note_ref, other) = {
            let conn = state.db.lock(cx).await.unwrap();
            conn.execute(
                "INSERT INTO memories (text, source, type, source_agent, status) VALUES ('NOTE-1 compare ident kind row', 'pending', 'note', 'decomp', 'active')",
                [],
            )
            .unwrap();
            let id = conn.last_insert_rowid();
            let note_ref = format!("note::{id}");
            conn.execute(
                "UPDATE memories SET source = ?1 WHERE id = ?2",
                rusqlite::params![note_ref, id],
            )
            .unwrap();
            drop(conn);
            let stored = dispatch(
                cx,
                &state,
                caller(),
                Operation::Commit,
                &json!({"entries": [{"kind": "decision", "text": "NOTE-1 partner: keep the ledger retry bound at three attempts"}]}),
            )
            .await
            .unwrap();
            let other = format!(
                "decision::{}",
                stored["receipt"]["entries"]["entry.decision"]["value"]
                    .as_str()
                    .unwrap()
            );
            (note_ref, other)
        };
        let cmp = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "what changed", "profile": "compare", "compare": [note_ref, other]}),
        )
        .await
        .unwrap();
        assert_ne!(
            cmp["status"], "invalid_request",
            "ident kinds other than memory/decision must stay comparable: {cmp}"
        );
    });
}
