//! Intake: source authorization, exact occurrences, and atomic source cursors.
use cortex_kernel::runtime::{
    CortexRuntime,
    observation::{ObservationEvent, SourceSpec},
};
use cortex_tests::support::run_with_cx;

fn event(key: &str, text: &str) -> ObservationEvent {
    ObservationEvent {
        event_key: key.into(),
        text: text.into(),
        observed_at: None,
    }
}
#[test]
fn file_import_preserves_exact_registered_source_and_reports_sql_failure() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".claude");
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("state.md");
        let text = format!(
            "# Exact source\n## What Was Done\n{}\nrare-tail-527\n",
            "evidence ".repeat(180)
        );
        std::fs::write(&path, &text).unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        let key = format!("file:{}", path.canonicalize().unwrap().to_str().unwrap());
        runtime
            .register_source(&cx, SourceSpec::document(&key, "project"))
            .await
            .unwrap();
        let mut conn = runtime.state().db.lock(&cx).await.unwrap();
        assert_eq!(
            cortex_kernel::indexer::index_all(&mut conn, home.path(), None).unwrap(),
            1
        );
        let captured: Vec<Vec<u8>> = conn
            .prepare("SELECT inline_payload FROM sources WHERE origin_id=?1")
            .unwrap()
            .query_map([&key], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            captured,
            vec![text.as_bytes().to_vec()],
            "indexer must retain the entire registered source, not selected sections or previews"
        );
        assert_eq!(
            cortex_kernel::indexer::index_all(&mut conn, home.path(), None).unwrap(),
            1
        );
        let count: i64 = conn
            .query_row("SELECT count(*) FROM observation_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "unchanged file replay must not duplicate an occurrence"
        );
        conn.execute_batch("CREATE TEMP TRIGGER deny_file_capture BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT, 'file capture rejected'); END;").unwrap();
        std::fs::write(&path, format!("{text}changed\n")).unwrap();
        let result = cortex_kernel::indexer::index_all(&mut conn, home.path(), None);
        assert!(
            format!("{result:?}").contains("file capture rejected"),
            "SQL failure must reach caller: {result:?}"
        );
        let count: i64 = conn
            .query_row("SELECT count(*) FROM observation_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "failed revision must roll back receipt and source"
        );
        assert_eq!(
            cortex_kernel::indexer::index_file(&mut conn, &path, Some(7)).unwrap_err(),
            "source_not_authorized"
        );
        drop(conn);
        drop(runtime);
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        let changed = runtime.observe_file(&cx, &path).await.unwrap();
        assert!(!changed.duplicate);
        assert!(runtime.observe_file(&cx, &path).await.unwrap().duplicate);
        assert_eq!(
            runtime
                .read_observation(&cx, &changed.source_id)
                .await
                .unwrap()
                .text,
            format!("{text}changed\n")
        );
        let other = home.path().join("state.md");
        std::fs::write(&other, &text).unwrap();
        assert_eq!(
            runtime.observe_file(&cx, &other).await.unwrap_err(),
            "source_not_authorized"
        );
        let other_key = format!("file:{}", other.canonicalize().unwrap().to_str().unwrap());
        runtime
            .register_source(&cx, SourceSpec::document(&other_key, "other-project"))
            .await
            .unwrap();
        let separate = runtime.observe_file(&cx, &other).await.unwrap();
        assert_ne!(separate.source_id, changed.source_id);
        runtime.set_source_enabled(&cx, &key, false).await.unwrap();
        assert_eq!(
            runtime.observe_file(&cx, &path).await.unwrap_err(),
            "source_disabled_or_policy_stale"
        );
    });
}

#[test]
fn custom_sources_config_refuses_oversize_toml() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".cortex");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("sources.toml"),
            "#".repeat(cortex_kernel::indexer::INDEXER_MAX_CONFIG_BYTES as usize + 1),
        )
        .unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        let mut conn = runtime.state().db.lock(&cx).await.unwrap();
        let err = cortex_kernel::indexer::index_all(&mut conn, home.path(), None).unwrap_err();
        assert_eq!(err, "source_config_byte_limit");
    });
}

#[test]
fn exact_capture_preserves_tail_and_distinguishes_occurrences_from_retries() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("cortex.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("project-notes", "repo-a"))
            .await
            .unwrap();
        let text = format!("{} raretail987", "retained source paragraph ".repeat(100));
        let first = runtime
            .observe(&cx, "project-notes", "g1", event("one", &text))
            .await
            .unwrap();
        let replay = runtime
            .observe(&cx, "project-notes", "g1", event("one", &text))
            .await
            .unwrap();
        let independent = runtime
            .observe(&cx, "project-notes", "g1", event("two", &text))
            .await
            .unwrap();
        assert_eq!(first.source_id, replay.source_id);
        assert_ne!(first.source_id, independent.source_id);
        assert!(replay.duplicate);
        assert_eq!(
            runtime
                .read_observation(&cx, &first.source_id)
                .await
                .unwrap()
                .text,
            text
        );
        assert!(
            runtime
                .observe(&cx, "project-notes", "g1", event("one", "changed payload"))
                .await
                .is_err()
        );
        drop(runtime);
        let reopened = CortexRuntime::open_db(&home.path().join("cortex.db")).unwrap();
        assert_eq!(
            reopened
                .read_observation(&cx, &first.source_id)
                .await
                .unwrap()
                .text,
            text
        );
    });
}

#[test]
fn denied_capture_and_failed_cursor_batches_do_not_advance() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("cortex.db")).unwrap();
        assert!(
            runtime
                .observe(&cx, "transcript", "g1", event("one", "not granted"))
                .await
                .is_err()
        );
        runtime
            .register_source(&cx, SourceSpec::document("transcript", "repo-a"))
            .await
            .unwrap();
        let first =
            b"{\"event_key\":\"one\",\"text\":\"complete observation\",\"observed_at\":null}\n";
        let mut malformed = first.to_vec();
        malformed.extend_from_slice(b"{bad}\n");
        assert!(
            runtime
                .tail_observations(&cx, "transcript", "g1", 0, &malformed)
                .await
                .is_err()
        );
        assert_eq!(
            runtime
                .source_offset(&cx, "transcript", "g1")
                .await
                .unwrap(),
            0
        );
        let mut partial = first.to_vec();
        partial.extend_from_slice(b"{\"event_key\":\"two\"");
        let receipt = runtime
            .tail_observations(&cx, "transcript", "g1", 0, &partial)
            .await
            .unwrap();
        assert_eq!(receipt.next_offset, first.len() as u64);
        assert_eq!(receipt.accepted.len(), 1);
        assert!(
            !receipt.accepted[0].duplicate,
            "failed batch must have rolled back its first row"
        );
        assert_eq!(receipt.uncommitted_tail_bytes, partial.len() - first.len());
        assert!(
            runtime
                .tail_observations(&cx, "transcript", "g1", 0, first)
                .await
                .is_err()
        );
        runtime
            .set_source_enabled(&cx, "transcript", false)
            .await
            .unwrap();
        assert!(
            runtime
                .observe(&cx, "transcript", "g1", event("two", "revoked"))
                .await
                .is_err()
        );
    });
}

#[test]
fn cursor_and_capture_rollback_when_outbox_write_fails() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("cortex.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("transcript", "repo-a"))
            .await
            .unwrap();
        runtime.state().db.lock(&cx).await.unwrap().execute_batch("CREATE TEMP TRIGGER deny_observation_job BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT, 'capture outbox failure'); END;").unwrap();
        let chunk = b"{\"event_key\":\"one\",\"text\":\"atomic captured observation\",\"observed_at\":null}\n";
        assert!(
            runtime
                .tail_observations(&cx, "transcript", "g1", 0, chunk)
                .await
                .is_err()
        );
        assert_eq!(
            runtime
                .source_offset(&cx, "transcript", "g1")
                .await
                .unwrap(),
            0
        );
        runtime
            .state()
            .db
            .lock(&cx)
            .await
            .unwrap()
            .execute_batch("DROP TRIGGER deny_observation_job;")
            .unwrap();
        let receipt = runtime
            .tail_observations(&cx, "transcript", "g1", 0, chunk)
            .await
            .unwrap();
        assert!(!receipt.accepted[0].duplicate);
    });
}
#[test]
fn source_authority_is_principal_scoped_and_limits_precede_writes() {
    run_with_cx(|cx| async move {
        let alice = cortex_tests::support::team_state(1);
        let mut bob = alice.clone();
        bob.default_owner_id = Some(2);
        let a = CortexRuntime::from_state(alice);
        let b = CortexRuntime::from_state(bob);
        a.register_source(&cx, SourceSpec::document("notes", "repo-a"))
            .await
            .unwrap();
        b.register_source(&cx, SourceSpec::document("notes", "repo-a"))
            .await
            .unwrap();
        let ar = a
            .observe(
                &cx,
                "notes",
                "g1",
                event("one", "alice private observation"),
            )
            .await
            .unwrap();
        let br = b
            .observe(&cx, "notes", "g1", event("one", "bob private observation"))
            .await
            .unwrap();
        assert_ne!(ar.source_id, br.source_id);
        assert!(b.read_observation(&cx, &ar.source_id).await.is_err());
        assert!(
            a.observe(
                &cx,
                "notes",
                "g1",
                event(
                    "secret",
                    "Authorization: Bearer sk-abcdefghijklmnopqrstuvwxyz1234567890"
                )
            )
            .await
            .unwrap_err()
            .contains("secret")
        );
        let mut bounded = SourceSpec::document("bounded", "repo-a");
        bounded.max_bytes = 4;
        a.register_source(&cx, bounded).await.unwrap();
        assert!(
            a.observe(&cx, "bounded", "g1", event("one", "12345"))
                .await
                .is_err()
        );
        assert!(
            !a.observe(&cx, "bounded", "g1", event("one", "1234"))
                .await
                .unwrap()
                .duplicate
        );
    });
}

#[test]
fn normalized_records_cannot_supply_their_own_authority() {
    assert!(
        serde_json::from_str::<ObservationEvent>(
            r#"{"event_key":"one","text":"claimed admin","actor":"operator","observed_at":null}"#
        )
        .is_err()
    );
}
#[test]
fn cli_capture_roundtrip_and_checkpoint_stdout_are_quiet_when_requested() {
    fn invoke(home: &std::path::Path, args: &[&str], input: &[u8]) -> std::process::Output {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new(cortex_tests::cortex_bin())
            .args(args)
            .arg("--home")
            .arg(home)
            .env_remove("CORTEX_DB")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("brain");
    let invalid = invoke(&home, &["capture", "register", "--scope", "repo-a"], b"");
    assert!(!invalid.status.success());
    assert!(
        !home.exists(),
        "invalid registration must fail before opening a brain"
    );
    let registration = invoke(
        &home,
        &[
            "capture", "register", "--source", "notes", "--scope", "repo-a",
        ],
        b"",
    );
    assert!(
        registration.status.success(),
        "{}",
        String::from_utf8_lossy(&registration.stderr)
    );
    let input = br#"{"event_key":"one","text":"full source through CLI","observed_at":null}"#;
    let quiet = invoke(
        &home,
        &[
            "capture",
            "put",
            "--source",
            "notes",
            "--generation",
            "g1",
            "--quiet",
        ],
        input,
    );
    assert!(
        quiet.status.success(),
        "{}",
        String::from_utf8_lossy(&quiet.stderr)
    );
    assert!(
        quiet.stdout.is_empty(),
        "successful capture must not narrate to the model"
    );
    let replay = invoke(
        &home,
        &["capture", "put", "--source", "notes", "--generation", "g1"],
        input,
    );
    assert!(replay.status.success());
    let receipt: serde_json::Value = serde_json::from_slice(&replay.stdout).unwrap();
    assert_eq!(receipt["duplicate"], true);
    let exact = invoke(
        &home,
        &[
            "capture",
            "get",
            "--id",
            receipt["source_id"].as_str().unwrap(),
        ],
        b"",
    );
    assert!(exact.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&exact.stdout).unwrap()["text"],
        "full source through CLI"
    );
    let file = root.path().join("source.md");
    std::fs::write(&file, "  exact file\n").unwrap();
    let file_key = format!("file:{}", file.canonicalize().unwrap().to_str().unwrap());
    let registered = invoke(
        &home,
        &[
            "capture", "register", "--source", &file_key, "--scope", "repo-a",
        ],
        b"",
    );
    assert!(registered.status.success());
    let file_args = ["capture", "file", "--path", file.to_str().unwrap()];
    let imported = invoke(&home, &file_args, b"");
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&imported.stdout).unwrap();
    assert_eq!(receipt["retained_bytes"], 13);
    let replay = invoke(&home, &file_args, b"");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&replay.stdout).unwrap()["duplicate"],
        true
    );
    let quiet = invoke(
        &home,
        &[
            "capture",
            "file",
            "--path",
            file.to_str().unwrap(),
            "--quiet",
        ],
        b"",
    );
    assert!(quiet.status.success());
    assert!(quiet.stdout.is_empty());
    let spec=br#"{"id":"cli-active","scope":"repo-a","cues":["exact"],"max_results":8,"max_bytes":4096,"ttl_seconds":3600}"#;
    let subscribed = invoke(&home, &["capture", "subscribe"], spec);
    assert!(
        subscribed.status.success(),
        "{}",
        String::from_utf8_lossy(&subscribed.stderr)
    );
    let queried = invoke(
        &home,
        &["capture", "query", "--scope", "repo-a", "--query", "exact"],
        b"",
    );
    assert!(
        queried.status.success(),
        "{}",
        String::from_utf8_lossy(&queried.stderr)
    );
    let prepared = invoke(
        &home,
        &[
            "capture",
            "prepare",
            "--id",
            "cli-active",
            "--context",
            "c1",
        ],
        b"",
    );
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    let view: serde_json::Value = serde_json::from_slice(&prepared.stdout).unwrap();
    let token = view["delivery_id"].as_str().unwrap();
    let present = invoke(
        &home,
        &[
            "capture",
            "prepare",
            "--id",
            "cli-active",
            "--context",
            "c1",
            "--present",
            token,
            "--payload",
        ],
        b"",
    );
    assert!(present.status.success());
    assert!(present.stdout.is_empty());
    let removed = invoke(
        &home,
        &["hook-event", "PreCompact"],
        br#"{"hook_event_name":"PreCompact","session_id":"capture-test"}"#,
    );
    assert!(
        !removed.status.success(),
        "hook-event is not a command; the live host entry is hook"
    );
    let checkpoint = invoke(
        &home,
        &["hook", "PreCompact"],
        br#"{"hook_event_name":"PreCompact","session_id":"capture-test"}"#,
    );
    assert!(
        checkpoint.status.success(),
        "{}",
        String::from_utf8_lossy(&checkpoint.stderr)
    );
    assert!(
        checkpoint.stdout.is_empty(),
        "checkpoint acknowledgement must remain outside model context"
    );
}

#[test]
fn cli_capture_query_with_path_stays_in_that_repository() {
    fn invoke(home: &std::path::Path, args: &[&str], input: &[u8]) -> std::process::Output {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new(cortex_tests::cortex_bin())
            .args(args)
            .arg("--home")
            .arg(home)
            .env_remove("CORTEX_DB")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }
    const REPO_A: &str = "/Users/x/repoa";
    const REPO_B: &str = "/Users/x/repob";
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("brain");
    let registered_a = invoke(
        &home,
        &[
            "capture", "register", "--source", "notes-a", "--scope", REPO_A,
        ],
        b"",
    );
    assert!(
        registered_a.status.success(),
        "{}",
        String::from_utf8_lossy(&registered_a.stderr)
    );
    let registered_b = invoke(
        &home,
        &[
            "capture", "register", "--source", "notes-b", "--scope", REPO_B,
        ],
        b"",
    );
    assert!(
        registered_b.status.success(),
        "{}",
        String::from_utf8_lossy(&registered_b.stderr)
    );
    let put_a = invoke(
        &home,
        &[
            "capture", "put", "--source", "notes-a", "--generation", "g1",
        ],
        br#"{"event_key":"a1","text":"CLI-PATH-SHARED ledger in repo A","observed_at":null}"#,
    );
    assert!(
        put_a.status.success(),
        "{}",
        String::from_utf8_lossy(&put_a.stderr)
    );
    let put_b = invoke(
        &home,
        &[
            "capture", "put", "--source", "notes-b", "--generation", "g1",
        ],
        br#"{"event_key":"b1","text":"CLI-PATH-SHARED cache in repo B","observed_at":null}"#,
    );
    assert!(
        put_b.status.success(),
        "{}",
        String::from_utf8_lossy(&put_b.stderr)
    );
    let queried_a = invoke(
        &home,
        &[
            "capture", "query", "--path", REPO_A, "--query", "CLI-PATH-SHARED",
        ],
        b"",
    );
    assert!(
        queried_a.status.success(),
        "{}",
        String::from_utf8_lossy(&queried_a.stderr)
    );
    let view_a: serde_json::Value = serde_json::from_slice(&queried_a.stdout).unwrap();
    let texts_a: Vec<&str> = view_a["evidence"]
        .as_array()
        .unwrap_or_else(|| panic!("path query must return evidence: {view_a}"))
        .iter()
        .filter_map(|item| item["text"].as_str())
        .collect();
    assert!(
        texts_a.iter().any(|text| text.contains("ledger in repo A")),
        "same-repo path query must return that repository's observation: {view_a}"
    );
    assert!(
        texts_a.iter().all(|text| !text.contains("cache in repo B")),
        "sibling observation leaked into path query: {view_a}"
    );
    let queried_b = invoke(
        &home,
        &[
            "capture", "query", "--path", REPO_B, "--query", "CLI-PATH-SHARED",
        ],
        b"",
    );
    assert!(
        queried_b.status.success(),
        "{}",
        String::from_utf8_lossy(&queried_b.stderr)
    );
    let view_b: serde_json::Value = serde_json::from_slice(&queried_b.stdout).unwrap();
    let texts_b: Vec<&str> = view_b["evidence"]
        .as_array()
        .unwrap_or_else(|| panic!("sibling path query must return evidence: {view_b}"))
        .iter()
        .filter_map(|item| item["text"].as_str())
        .collect();
    assert!(
        texts_b.iter().any(|text| text.contains("cache in repo B")),
        "{view_b}"
    );
    assert!(
        texts_b.iter().all(|text| !text.contains("ledger in repo A")),
        "repo A must not appear under repo B: {view_b}"
    );
    let missing_scope = invoke(
        &home,
        &["capture", "query", "--query", "CLI-PATH-SHARED"],
        b"",
    );
    assert!(
        !missing_scope.status.success(),
        "query without --path still requires --scope"
    );
}
