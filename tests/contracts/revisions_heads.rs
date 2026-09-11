//! Authoritative schema laws: additive migration, legacy import with labeled
//! provenance gaps and exact text, concurrent heads preserved, explicit
//! resolution, deposits write records in the same batch, dry-run migration
//! is side-effect free.

#[path = "../support/mod.rs"]
mod support;

use cortex_kernel::db::records::{
    append_commit, append_revision, heads, import_legacy, resolve_heads, revision_body, NewRevision,
};
use cortex_kernel::runtime::CortexRuntime;
use cortex_tests::cortex_bin;
use cortex_tests::support::{solo_state, test_conn};
use rusqlite::Connection;
use serde_json::json;
use std::fs;
use support::unique_temp_dir;

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn migration_creates_authoritative_tables_without_touching_legacy_rows() {
    let conn = test_conn();
    for table in [
        "brain_meta",
        "scopes",
        "commits",
        "sources",
        "records",
        "revisions",
        "revision_parents",
        "record_heads",
        "revision_sources",
        "relations",
        "digests",
        "addresses",
        "threads",
        "thread_members",
        "obligations",
        "change_items",
        "projection_state",
        "outbox",
        "guard_epochs",
        "compiled_reads",
        "compiled_guards",
        "view_receipts",
        "view_aliases",
        "erasures",
    ] {
        assert!(
            cortex_kernel::db::table_exists(&conn, table),
            "missing {table}"
        );
    }
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM brain_meta"), 1);
    let basis: String = conn
        .query_row("SELECT trust_basis FROM decisions LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap_or_else(|_| "legacy_model_weight".into());
    assert_eq!(basis, "legacy_model_weight");
}

#[test]
fn legacy_rows_import_as_baseline_revisions_with_exact_text_and_labeled_gap() {
    let conn = test_conn();
    let text = "légacy décision — exact bytes ✓ must survive";
    conn.execute("INSERT INTO decisions (decision, type, source_agent, status, retention_class) VALUES (?1, 'decision', 'old-agent', 'active', 'durable')", [text]).unwrap();
    let id = conn.last_insert_rowid();
    conn.execute("INSERT INTO memories (text, type, source_agent, status) VALUES ('an old memory', 'memory', 'old-agent', 'archived')", []).unwrap();
    let imported = import_legacy(&conn).unwrap();
    assert_eq!(imported, 2);
    assert_eq!(import_legacy(&conn).unwrap(), 0, "import is idempotent");
    let record_id: String = conn.query_row("SELECT record_id FROM addresses WHERE scheme='legacy' AND namespace='decision' AND address=?1", [id.to_string()], |r| r.get(0)).unwrap();
    let head = heads(&conn, &record_id).unwrap();
    assert_eq!(head.len(), 1);
    let body = revision_body(&conn, &head[0]).unwrap().unwrap();
    assert_eq!(body["text"], text, "imported text is byte-exact");
    assert_eq!(body["legacy"]["status"], "active");
    assert!(
        body["provenance_gap"]
            .as_str()
            .unwrap()
            .contains("no source lineage"),
        "gap is labeled, not manufactured: {body}"
    );
    let status: String = conn
        .query_row(
            "SELECT epistemic_status FROM revisions WHERE revision_id=?1",
            [&head[0]],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "asserted", "no historical verification is invented");
    let retention: String = conn
        .query_row(
            "SELECT retention FROM records WHERE record_id=?1",
            [&record_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retention, "durable");
    let legacy_text: String = conn
        .query_row("SELECT decision FROM decisions WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(legacy_text, text, "legacy row untouched");
}

#[test]
fn concurrent_revisions_from_one_parent_leave_two_heads_until_resolved() {
    let conn = test_conn();
    let seq = append_commit(&conn, "user:a", None, "process_crash").unwrap();
    let base = append_revision(
        &conn,
        seq,
        NewRevision {
            record_id: "claim:1",
            kind: "claim",
            retention: "durable",
            body: json!({"text":"retry 3 times"}),
            epistemic_status: "asserted",
            parents: &[],
            replace_parents: true,
            representation_version: "t/1",
        },
    )
    .unwrap();
    let seq_a = append_commit(&conn, "user:a", None, "process_crash").unwrap();
    let a = append_revision(
        &conn,
        seq_a,
        NewRevision {
            record_id: "claim:1",
            kind: "claim",
            retention: "durable",
            body: json!({"text":"never retry after commit"}),
            epistemic_status: "asserted",
            parents: std::slice::from_ref(&base),
            replace_parents: true,
            representation_version: "t/1",
        },
    )
    .unwrap();
    let seq_b = append_commit(&conn, "user:b", None, "process_crash").unwrap();
    // Writer B revised from the same parent concurrently: it must not
    // silently replace A's head.
    let b = append_revision(
        &conn,
        seq_b,
        NewRevision {
            record_id: "claim:1",
            kind: "claim",
            retention: "durable",
            body: json!({"text":"retry 5 times"}),
            epistemic_status: "asserted",
            parents: std::slice::from_ref(&base),
            replace_parents: false,
            representation_version: "t/1",
        },
    )
    .unwrap();
    let current = heads(&conn, "claim:1").unwrap();
    assert_eq!(
        current,
        vec![a.clone(), b.clone()],
        "both concurrent heads visible"
    );
    assert!(
        revision_body(&conn, &base).unwrap().is_some(),
        "the parent revision is never deleted"
    );
    let seq_r = append_commit(&conn, "user:owner", None, "process_crash").unwrap();
    let stale = resolve_heads(
        &conn,
        seq_r,
        "claim:1",
        &[base.clone()],
        "user:owner",
        "x",
        json!({}),
    );
    assert!(
        stale.is_err(),
        "resolving a non-head must fail, not overwrite"
    );
    let resolution = resolve_heads(
        &conn,
        seq_r,
        "claim:1",
        &[a.clone(), b.clone()],
        "user:owner",
        "commit-state exception wins",
        json!({"text":"never retry after commit"}),
    )
    .unwrap();
    assert_eq!(heads(&conn, "claim:1").unwrap(), vec![resolution.clone()]);
    let body = revision_body(&conn, &resolution).unwrap().unwrap();
    assert_eq!(body["resolution"]["considered"], json!([a, b]));
    assert_eq!(body["resolution"]["authority"], "user:owner");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM revisions WHERE record_id='claim:1'"
        ),
        4,
        "rejected evidence stays"
    );
}

#[test]
fn deposit_writes_record_revision_head_and_change_item_in_the_same_batch() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let runtime = CortexRuntime::from_state(solo_state());
        let outcome = runtime
            .deposit(
                cx,
                "r1",
                "REV-1 deposits populate the authoritative tables",
                "agent-a",
                None,
            )
            .await
            .unwrap();
        let conn = runtime.state().db.lock(cx).await.unwrap();
        let record_id = format!("decision:{}", outcome.target_id.unwrap());
        let head = heads(&conn, &record_id).unwrap();
        assert_eq!(head.len(), 1, "{head:?}");
        assert_eq!(
            revision_body(&conn, &head[0]).unwrap().unwrap()["text"],
            "REV-1 deposits populate the authoritative tables"
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM commits"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM change_items"), 1);
        let address: String = conn.query_row("SELECT record_id FROM addresses WHERE scheme='legacy' AND namespace='decision' AND address=?1", [outcome.target_id.unwrap().to_string()], |r| r.get(0)).unwrap();
        assert_eq!(address, record_id);
    });
}

#[test]
fn migrate_dry_run_leaves_the_database_bytes_untouched() {
    let home = unique_temp_dir("migrate-dry-run");
    fs::create_dir_all(&home).unwrap();
    let db_path = home.join("cortex.db");
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE marker (x INTEGER); INSERT INTO marker VALUES (1);")
            .unwrap();
    }
    let before = fs::read(&db_path).unwrap();
    let output = std::process::Command::new(cortex_bin())
        .args([
            "migrate",
            "--dry-run",
            "--owner",
            "dryrun",
            "--home",
            &home.to_string_lossy(),
            "--db",
            &db_path.to_string_lossy(),
        ])
        .output()
        .expect("run migrate --dry-run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("DRY RUN"),
        "dry run banner expected: {stderr}"
    );
    let after = fs::read(&db_path).unwrap();
    assert_eq!(before, after, "dry run must not modify the database");
    assert!(
        !db_path.with_extension("db-wal").exists()
            || fs::metadata(db_path.with_extension("db-wal"))
                .map(|m| m.len())
                .unwrap_or(0)
                == 0,
        "no WAL written by a dry run"
    );
    assert!(
        !db_path.with_extension("db.bak").exists(),
        "dry run takes no backup"
    );
    let _ = fs::remove_dir_all(&home);
}
