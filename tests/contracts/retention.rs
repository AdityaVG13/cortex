//! Retention is not salience: durable rows are never archived by score; an
//! archived row past hot retention moves to a lossless cold segment (exact
//! roundtrip, dictionary-free deflate, digest-checked); expansion hydrates
//! it; history/audit profiles search the cold partition and the watermark
//! says so; other profiles disclose it as not searched.

use cortex_kernel::compiler::{
    empty_rank_components, fetch_rank_candidates, rank_candidates, rank_components_for,
    RankedCandidate,
};
use cortex_kernel::db::cold::{cold_count, decode, encode, hydrate, move_to_cold};
use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_kernel::handlers::recall::{unfold_source, RecallContext};
use cortex_tests::support::{solo_state, test_conn};
use serde_json::json;

fn caller() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "retention",
        principal: "solo".into(),
    }
}

#[test]
fn cold_codec_roundtrips_exact_bytes_including_unicode_and_binaryish_text() {
    for text in [
        "",
        "plain",
        "unicodé ✓ — 日本語",
        "\u{0}\u{1}\u{2} nulls",
        &"x".repeat(200_000),
    ] {
        let blob = encode(text.as_bytes()).expect("deflate encode");
        assert_eq!(decode(&blob).unwrap(), text.as_bytes(), "exact roundtrip");
    }
}

#[test]
fn unreadable_cold_payload_is_not_an_empty_success() {
    let conn = test_conn();
    cortex_kernel::db::cold::ensure_cold_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO decisions (decision, type, source_agent, status, retention_class, score, pinned, last_accessed, created_at) VALUES ('[cold:1] preview', 'constraint', 'a', 'active', 'durable', 0.5, 0, '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    let id: i64 = conn
        .query_row(
            "SELECT id FROM decisions WHERE decision LIKE '[cold:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO cold_sources (namespace, address, codec, byte_length, payload, digest) VALUES ('decision', ?1, 'deflate/1', 64, x'ffff', 'not-a-digest')",
        [id.to_string()],
    )
    .unwrap();
    assert!(
        hydrate(&conn, "decision", id).unwrap().is_none(),
        "corrupt inflate must not hydrate as empty text"
    );
    let expanded = unfold_source(&conn, &format!("decision::{id}"), &RecallContext::solo()).unwrap();
    assert_eq!(expanded["physical_state"], "unavailable");
    assert!(expanded["text"].is_null(), "{expanded}");
    conn.execute(
        "UPDATE cold_sources SET byte_length = 3000000, payload = x'00' WHERE namespace = 'decision' AND address = ?1",
        [id.to_string()],
    )
    .unwrap();
    assert!(
        hydrate(&conn, "decision", id).unwrap().is_none(),
        "oversize declared length must not hydrate as empty text"
    );
}

#[test]
fn durable_low_score_rows_are_never_archived_by_the_aging_gc() {
    let conn = test_conn();
    conn.execute("INSERT INTO decisions (decision, type, source_agent, status, retention_class, score, pinned, last_accessed, created_at) VALUES ('rare old durable constraint', 'constraint', 'a', 'active', 'durable', 0.01, 0, '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z')", []).unwrap();
    conn.execute("INSERT INTO memories (text, type, source_agent, status, retention_class, score, pinned, last_accessed, created_at) VALUES ('stale operational chatter', 'note', 'a', 'active', 'operational', 0.01, 0, '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z')", []).unwrap();
    let report = cortex_kernel::aging::run_aging_pass(&conn);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let durable: String = conn
        .query_row(
            "SELECT status FROM decisions WHERE decision = 'rare old durable constraint'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(durable, "active", "salience never demotes a durable row");
    let chatter: String = conn
        .query_row(
            "SELECT status FROM memories WHERE text = 'stale operational chatter'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        chatter, "archived",
        "operational rows may be demoted to the archive placement"
    );
}

#[test]
fn archived_rows_move_to_a_cold_segment_and_stay_recoverable_and_findable() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        let text =
        "COLD-1 the legacy gateway required the X-Ledger-Epoch header — exact wording matters ✓";
        let r = dispatch(
            cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": text, "context": "runbook §4"}),
        )
        .await
        .unwrap();
        let id: i64 = r["receipt"]["entries"]["decision.decision"]["value"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        {
            let conn = state.db.lock(cx).await.unwrap();
            conn.execute("UPDATE decisions SET status = 'archived', updated_at = '2020-01-01T00:00:00Z' WHERE id = ?1", [id]).unwrap();
            let mut failures = Vec::new();
            let moved = cortex_kernel::compaction::strip_archived_text_with_retention_for_test(
                &conn,
                &mut failures,
                30,
            );
            assert_eq!(moved, 1, "{failures:?}");
            assert_eq!(cold_count(&conn), 1);
            let inline: String = conn
                .query_row("SELECT decision FROM decisions WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .unwrap();
            assert!(
                inline.starts_with("[cold:"),
                "inline text is a route marker, not the source: {inline}"
            );
            let (hydrated, context, intact) =
                hydrate(&conn, "decision", id).unwrap().expect("cold block");
            assert_eq!(hydrated, text, "exact bytes recovered");
            assert_eq!(context.as_deref(), Some("runbook §4"));
            assert!(intact);
            assert!(
                move_to_cold(&conn, "decision", id).unwrap().is_none(),
                "idempotent"
            );
            let expanded =
                unfold_source(&conn, &format!("decision::{id}"), &RecallContext::solo()).unwrap();
            assert!(
                expanded["text"].as_str().unwrap().starts_with(text),
                "expand hydrates the cold block: {expanded}"
            );
            assert_eq!(expanded["physical_state"], "cold_segment");
        }
        // Ordinary profiles do not search the cold partition and say so.
        let warm = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "COLD-1 X-Ledger-Epoch", "profile": "map"}),
        )
        .await
        .unwrap();
        assert_eq!(
            warm["coverage"]["partitions"]["cold"]["searched"], false,
            "{}",
            warm["coverage"]
        );
        assert_eq!(warm["coverage"]["partitions"]["cold"]["cold_segments"], 1);
        assert!(warm["cards"].as_array().unwrap().is_empty(), "{warm}");
        // History searches it: the old named fact routes through its anchors.
        let history = dispatch(cx, &state, caller(), Operation::Query, &json!({"need": "COLD-1 X-Ledger-Epoch", "profile": "history", "evidence": "exact", "budget": 8000})).await.unwrap();
        assert_eq!(
            history["coverage"]["partitions"]["cold"]["searched"], true,
            "{}",
            history["coverage"]
        );
        let card = history["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["sidecar"]["reference"] == format!("decision::{id}"))
            .unwrap_or_else(|| panic!("cold row must be findable through history: {history}"));
        assert!(
            card["exact"].as_str().unwrap().starts_with(text),
            "exact rendering carries the hydrated bytes: {card}"
        );
        assert_ne!(
            card["applicability"], "applicable",
            "an archived row is not presented as currently applicable"
        );
    });
}

#[test]
fn blank_updated_at_falls_through_created_at_for_cold_move() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO memories (text, type, source_agent, status, retention_class, score, pinned, updated_at, created_at) VALUES ('blank-updated archived memory', 'note', 'a', 'archived', 'operational', 0.5, 0, '', '2020-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO decisions (decision, type, source_agent, status, retention_class, score, pinned, updated_at, created_at) VALUES ('blank-updated archived decision', 'constraint', 'a', 'archived', 'operational', 0.5, 0, '   ', '2020-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    let mut failures = Vec::new();
    let moved = cortex_kernel::compaction::strip_archived_text_with_retention_for_test(
        &conn,
        &mut failures,
        30,
    );
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        moved, 2,
        "blank/whitespace updated_at must age from created_at, not skip cold-move"
    );
    let mem: String = conn
        .query_row(
            "SELECT text FROM memories WHERE text LIKE '[cold:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(mem.starts_with("[cold:"), "{mem}");
    let dec: String = conn
        .query_row(
            "SELECT decision FROM decisions WHERE decision LIKE '[cold:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(dec.starts_with("[cold:"), "{dec}");
}

fn ranked_memory(id: i64, updated_at: Option<&str>, created_at: Option<&str>) -> RankedCandidate {
    RankedCandidate {
        source_kind: "memory",
        source_id: id,
        retention_class: "operational".into(),
        body: format!("body-{id}"),
        updated_at: updated_at.map(str::to_string),
        created_at: created_at.map(str::to_string),
        last_accessed: None,
        retrievals: 0,
        relevance: 0.5,
        status: "active".into(),
        confirmed_by: None,
        valid_from: None,
        valid_until: None,
        components: empty_rank_components(),
    }
}

#[test]
fn blank_updated_at_falls_through_created_at_for_boot_recency() {
    let now = cortex_kernel::compiler::parse_timestamp(Some("2026-09-13T12:00:00.000Z"))
        .expect("now");
    let fresh = rank_components_for(
        &ranked_memory(1, Some(""), Some("2026-09-13T11:00:00.000Z")),
        now,
    );
    let whitespace = rank_components_for(
        &ranked_memory(2, Some("   "), Some("2026-09-13T11:00:00.000Z")),
        now,
    );
    let ancient = rank_components_for(
        &ranked_memory(3, Some("2020-01-01T00:00:00.000Z"), Some("2020-01-01T00:00:00.000Z")),
        now,
    );
    assert!(
        fresh.recency_score > 0.8,
        "blank updated_at must use created_at, not the ancient bucket: {}",
        fresh.recency_score
    );
    assert!(
        whitespace.recency_score > 0.8,
        "whitespace updated_at must use created_at: {}",
        whitespace.recency_score
    );
    assert!(
        ancient.recency_score < 0.2,
        "a 2020 stamp must stay in the ancient bucket: {}",
        ancient.recency_score
    );
    let ranked = rank_candidates(
        vec![
            ranked_memory(3, Some("2020-01-01T00:00:00.000Z"), Some("2020-01-01T00:00:00.000Z")),
            ranked_memory(1, Some(""), Some("2026-09-13T11:00:00.000Z")),
        ],
        2,
        now,
    );
    assert_eq!(
        ranked[0].source_id, 1,
        "blank updated_at plus recent created_at must outrank a 2020 stamp"
    );
}

#[test]
fn blank_updated_at_still_enters_the_boot_rank_window() {
    let conn = test_conn();
    for i in 0..80 {
        conn.execute(
            "INSERT INTO memories (text, type, source_agent, status, score, pinned, updated_at, created_at) VALUES (?1, 'note', 'a', 'active', 0.5, 0, '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z')",
            [format!("old-ranked-{i}")],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO memories (text, type, source_agent, status, score, pinned, updated_at, created_at) VALUES ('blank-updated recent memory', 'note', 'a', 'active', 0.5, 0, '', '2026-09-13T11:00:00Z')",
        [],
    )
    .unwrap();
    let candidates = fetch_rank_candidates(&conn).expect("rank candidates");
    assert!(
        candidates
            .iter()
            .any(|c| c.source_kind == "memory" && c.body.contains("blank-updated recent memory")),
        "blank updated_at must fall through created_at so the row is inside LIMIT 80, not dropped: {} candidates",
        candidates.len()
    );
}
