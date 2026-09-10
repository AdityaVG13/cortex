//! Backup/restore laws: coherent online-backup snapshot with manifest; restore
//! mints a new restore epoch, expires aliases and old cursors, rebuilds
//! projections, verifies integrity + sample exact reads, and records a
//! verification report the health surface exposes. Plus the auto-repair
//! salvage list now includes history and authoritative tables.

#[path = "../support/mod.rs"]
mod support;

use cortex_daemon::db::backup::{backup_to, last_verified_restore, restore_from};
use cortex_daemon::db::records::{brain_epochs, heads};
use cortex_daemon::runtime::CortexRuntime;
use cortex_daemon::store_spi::sqlite::{current_frontier, SqliteStore};
use cortex_daemon::store_spi::{BrainStore, StoreSpiError};
use cortex_tests::support::open_file_db;
use std::fs;
use support::unique_temp_dir;

fn seeded_home(label: &str, texts: &[&str]) -> (std::path::PathBuf, std::path::PathBuf) {
    let home = unique_temp_dir(label);
    fs::create_dir_all(&home).unwrap();
    let db = home.join("cortex.db");
    let conn = open_file_db(&db);
    for text in texts {
        conn.execute("INSERT INTO decisions (decision, type, source_agent, status, retention_class) VALUES (?1, 'decision', 'seed', 'active', 'durable')", [text]).unwrap();
    }
    cortex_daemon::db::records::import_legacy(&conn).unwrap();
    (home, db)
}

#[test]
fn backup_is_coherent_and_carries_a_manifest() {
    let (home, db) = seeded_home("bk-manifest", &["backup me exactly ✓", "second row"]);
    let dest = home.join("backups").join("cortex-test.db");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    let (file, manifest_path) = backup_to(&db, &dest).unwrap();
    assert!(file.exists() && manifest_path.exists());
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["counts"]["decisions"], 2);
    assert_eq!(manifest["counts"]["records"], 2);
    assert_eq!(manifest["integrity_ok"], true);
    assert!(manifest["sqlite_version"]
        .as_str()
        .unwrap()
        .starts_with("3."));
    assert_eq!(manifest["restore_epoch"], "0");
    let copy = open_file_db(&file);
    let text: String = copy
        .query_row(
            "SELECT decision FROM decisions ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(text, "backup me exactly ✓");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn restore_mints_a_new_epoch_expires_cursors_and_verifies_reads() {
    let (home, db) = seeded_home("bk-restore", &["kept across restore"]);
    let dest = home.join("snapshot.db");
    backup_to(&db, &dest).unwrap();
    // Diverge the live database after the backup and mint an alias + cursor.
    {
        let conn = open_file_db(&db);
        conn.execute("INSERT INTO decisions (decision, type, source_agent, status) VALUES ('written after backup', 'decision', 'seed', 'active')", []).unwrap();
        conn.execute("INSERT INTO view_receipts (receipt_id, principal_id, brain_epoch, through_sequence, receipt_json) VALUES ('rcpt-1', 'solo', '0', 1, '{}')", []).unwrap();
    }
    let old_cursor = {
        let store = SqliteStore::new(open_file_db(&db)).unwrap();
        current_frontier(store.connection())
    };
    let report = restore_from(&dest, &db, &home).unwrap();
    assert!(
        report.integrity_ok && report.sample_reads_ok,
        "{}",
        report.to_json()
    );
    assert_ne!(report.new_restore_epoch, "0");
    assert_eq!(
        report.decisions, 1,
        "the post-backup row is gone; restore is not a merge"
    );
    let conn = open_file_db(&db);
    assert_eq!(brain_epochs(&conn).1, report.new_restore_epoch);
    let aliases: i64 = conn
        .query_row("SELECT COUNT(*) FROM view_receipts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        aliases, 0,
        "receipts/aliases from before the restore are expired"
    );
    let record: String = conn
        .query_row(
            "SELECT record_id FROM addresses WHERE scheme='legacy' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(heads(&conn, &record).unwrap().len(), 1);
    drop(conn);
    let store = SqliteStore::new(open_file_db(&db)).unwrap();
    match store.read_changes(&old_cursor, 10) {
        Err(StoreSpiError::Unavailable(msg)) => {
            assert!(msg.contains("resnapshot_required"), "{msg}")
        }
        other => panic!("a cursor from the previous epoch must be rejected, got {other:?}"),
    }
    let last = last_verified_restore(&home).expect("report recorded");
    assert_eq!(last["verified"], true);
    assert!(
        home.read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("cortex.pre-restore.")),
        "pre-restore copy kept"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn restore_refuses_a_corrupt_backup_file() {
    let (home, db) = seeded_home("bk-corrupt", &["intact"]);
    let bogus = home.join("bogus.db");
    fs::write(&bogus, b"not a database at all").unwrap();
    let err = restore_from(&bogus, &db, &home).expect_err("corrupt backup must be refused");
    assert!(!err.is_empty(), "error must be reported: {err}");
    let conn = open_file_db(&db);
    let text: String = conn
        .query_row("SELECT decision FROM decisions LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        text, "intact",
        "live database untouched after a refused restore"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn health_exposes_restore_status_durability_and_sqlite_version() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = cortex_tests::support::solo_state();
        let runtime = CortexRuntime::from_state(state.clone());
        let _ = runtime
            .deposit(cx, "r", "health probe", "agent", None)
            .await
            .unwrap();
        let payload = cortex_daemon::handlers::health::build_health_payload(cx, &state, false)
            .await
            .unwrap();
        assert!(payload.get("last_verified_restore").is_some(), "{payload}");
        assert!(payload["sqlite_version"]
            .as_str()
            .unwrap()
            .starts_with("3."));
        assert!(matches!(
            payload["durability_profile"].as_str(),
            Some("durable") | Some("fast")
        ));
    });
}

#[test]
fn auto_repair_salvages_history_and_authoritative_tables() {
    let (home, db) = seeded_home("bk-repair", &["repair keeps history"]);
    {
        let conn = open_file_db(&db);
        cortex_daemon::traces::record_store_write(
            &conn,
            "seed",
            "repair keeps history",
            "stored",
            "decision",
            Some(1),
            None,
        );
    }
    let result = cortex_daemon::db::auto_repair(&db, "t").expect("auto_repair");
    assert_eq!(result.decisions_recovered, 1);
    let conn = open_file_db(&db);
    let versions: i64 = conn
        .query_row("SELECT COUNT(*) FROM versions", [], |r| r.get(0))
        .unwrap();
    assert!(versions >= 1, "versions salvaged");
    let records: i64 = conn
        .query_row("SELECT COUNT(*) FROM records", [], |r| r.get(0))
        .unwrap();
    assert_eq!(records, 1, "authoritative records salvaged");
    let _ = fs::remove_dir_all(&home);
}
