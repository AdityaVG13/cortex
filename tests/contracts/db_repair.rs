//! Contracts for the boot-time DB auto-repair / degraded-mode paths.
//!
//! Spec under test (crates/daemon/src/state/init.rs + crates/daemon/src/db/maintenance.rs):
//! - `PRAGMA quick_check` failure followed by `PRAGMA integrity_check` failure
//!   triggers `auto_repair` (dump-and-rebuild salvage).
//! - Repair success must boot FULLY healthy (db_corrupted stays false) after
//!   quarantining the corrupt file as `cortex.corrupt.<timestamp>`
//!   (`Path::with_extension` REPLACES the "db" suffix) and must preserve
//!   readable row data.
//! - Repair failure with a still-openable DB boots degraded
//!   (`db_corrupted=true` -> /health status "degraded") without quarantining.
//!
//! Poisoning targets derived/index b-tree pages, never the repo, and only in
//! per-test unique temp homes.

#[path = "../support/mod.rs"]
mod support;

use serde_json::json;
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::time::Duration;
use support::{
    read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon, unique_temp_dir,
    wait_for_exit, wait_for_health,
};

// Three topically distinct decisions (store writes to the `decisions` table,
// which auto_repair counts as decisions_recovered). Distinct tokens so the
// agreement-merge dedupe cannot fold them.
const SENTINELS: [&str; 3] = [
    "Auto-repair salvage sentinel one: the archive rotator keeps seven nightly snapshots on the vault volume.",
    "Auto-repair salvage sentinel two: the checksum auditor rejects any tile whose crc32 mismatches the manifest.",
    "Auto-repair salvage sentinel three: the replication lag alarm fires when the follower falls 900 seconds behind.",
];

// Derived structure that quick_check/integrity_check walk but the salvage
// export (SELECT per DATA_TABLE) never reads. Destroying it must trip the
// repair trigger without making the row data itself unreadable.
const DERIVED_INDEX: &str = "idx_memories_status";

fn store_sentinel(port: u16, token: &str, text: &str) {
    let resp = request_json(
        port,
        "POST",
        "/store",
        Some(token),
        Some(json!({
            "decision": text,
            "type": "decision",
            "source_agent": "db-repair",
            "confidence": 0.9,
        })),
    )
    .unwrap_or_else(|e| panic!("store {text:?} failed: {e}"));
    assert_eq!(
        resp.status, 200,
        "store must ack 200, got {} body {}",
        resp.status, resp.body
    );
    assert_eq!(
        resp.body["stored"].as_bool(),
        Some(true),
        "store must ack stored:true for {text:?}, got {}",
        resp.body
    );
}

fn recall_exact(port: u16, token: &str, expected: &str) {
    let encoded: String = expected
        .bytes()
        .flat_map(|b| {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
                vec![b as char]
            } else {
                format!("%{b:02X}").chars().collect()
            }
        })
        .collect();
    let resp = request_json(
        port,
        "GET",
        &format!("/recall?q={encoded}&k=10&budget=320"),
        Some(token),
        None,
    )
    .unwrap_or_else(|e| panic!("recall for {expected:?} failed: {e}"));
    assert_eq!(
        resp.status, 200,
        "recall must return 200, got {} body {}",
        resp.status, resp.body
    );
    let results = resp.body["results"]
        .as_array()
        .unwrap_or_else(|| panic!("recall results missing: {}", resp.body));
    let hit = results
        .iter()
        .any(|item| item["excerpt"].as_str() == Some(expected));
    assert!(
        hit,
        "recall must return the exact stored text {expected:?}, got {results:?}"
    );
}

/// Fold any WAL frames into the main db file so the poison lands on the
/// authoritative page images a fresh boot will read.
fn checkpoint_wal_into_main(db_path: &std::path::Path) {
    let conn = rusqlite::Connection::open(db_path).expect("open db for wal checkpoint");
    let _ = conn.busy_timeout(Duration::from_secs(2));
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        row.get::<_, i64>(0)
    });
}

/// Overwrite one b-tree root page with 0xFF bytes, then PROVE the poison
/// trips both init.rs gates (quick_check AND integrity_check) before any
/// daemon boots on it.
fn poison_btree_root_page(home_dir: &std::path::Path, btree: &str) {
    let db_path = home_dir.join("cortex.db");
    let conn = rusqlite::Connection::open(&db_path).expect("open db to locate poison target");
    let _ = conn.busy_timeout(Duration::from_secs(2));
    let page_size: u64 =
        conn.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0)).expect("page_size") as u64;
    let root: u64 = conn
        .query_row(
            "SELECT rootpage FROM sqlite_master WHERE name = ?1",
            [btree],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or_else(|e| panic!("poison target btree {btree:?} missing from sqlite_master: {e}"))
        as u64;
    drop(conn);

    let offset = root * page_size;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&db_path)
        .expect("open cortex.db for poisoning");
    let file_len = file.metadata().expect("db metadata").len();
    assert!(
        offset + page_size <= file_len,
        "poison offset {offset}+{page_size} beyond file length {file_len}"
    );
    file.seek(SeekFrom::Start(offset)).expect("seek to poison offset");
    let garbage = vec![0xFFu8; page_size as usize];
    file.write_all(&garbage).expect("write poison bytes");
    file.sync_all().expect("flush poison bytes");

    let conn = rusqlite::Connection::open(&db_path).expect("reopen poisoned db");
    let _ = conn.busy_timeout(Duration::from_secs(2));
    let quick_ok = conn
        .query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
        .map(|s| s.trim().eq_ignore_ascii_case("ok"))
        .unwrap_or(false);
    assert!(
        !quick_ok,
        "poison of {btree:?} root page must trip PRAGMA quick_check (init.rs repair trigger)"
    );
    let integrity_ok = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map(|s| s.trim().eq_ignore_ascii_case("ok"))
        .unwrap_or(false);
    assert!(
        !integrity_ok,
        "poison of {btree:?} root page must trip PRAGMA integrity_check (init.rs repair gate)"
    );
}

/// Quarantine naming per maintenance.rs: db_path.with_extension(format!(
/// "corrupt.{timestamp}")) on "cortex.db" REPLACES the extension, yielding
/// "cortex.corrupt.<timestamp>".
fn corrupt_archive_entries(home_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut hits: Vec<std::path::PathBuf> = fs::read_dir(home_dir)
        .expect("read home dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("cortex.corrupt."))
                .unwrap_or(false)
        })
        .collect();
    hits.sort();
    hits
}

/// Boot once, store the three sentinels, shut down cleanly, checkpoint WAL.
fn seed_home(home_dir: &std::path::Path) -> String {
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(home_dir);
    for text in &SENTINELS {
        store_sentinel(port, &token, text);
    }
    shutdown_daemon(port, home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    checkpoint_wal_into_main(&home_dir.join("cortex.db"));
    home
}

#[test]
fn poisoned_db_boots_via_auto_repair() {
    let home_dir = unique_temp_dir("dbrepair_salvage");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let home = seed_home(&home_dir);
    poison_btree_root_page(&home_dir, DERIVED_INDEX);

    // Fresh daemon on the SAME home must self-heal via auto_repair.
    let port = reserve_port();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    // Repair success boots FULLY healthy: init.rs never sets db_corrupted on
    // the auto_repair Ok path.
    let health = request_json(port, "GET", "/health", None, None).expect("health after repair boot");
    assert_eq!(
        health.status, 200,
        "health must be served after repair, got {} body {}",
        health.status, health.body
    );
    assert_eq!(
        health.body["status"].as_str(),
        Some("ok"),
        "successful auto-repair must boot with status ok, got {}",
        health.body
    );
    assert_eq!(
        health.body["degraded"].as_bool(),
        Some(false),
        "successful auto-repair must not set degraded, got {}",
        health.body
    );
    assert_eq!(
        health.body["db_corrupted"].as_bool(),
        Some(false),
        "successful auto-repair must not set db_corrupted, got {}",
        health.body
    );

    // Quarantine contract: exactly one timestamped archive of the corrupt file.
    let archives = corrupt_archive_entries(&home_dir);
    assert_eq!(
        archives.len(),
        1,
        "auto-repair must quarantine exactly one corrupt db archive, got {archives:?}"
    );
    let mut header = [0u8; 16];
    fs::File::open(&archives[0])
        .and_then(|mut file| file.read_exact(&mut header))
        .expect("read quarantined archive header");
    assert_eq!(
        &header, b"SQLite format 3\0",
        "quarantined archive must be the original sqlite file, got header {header:?}"
    );

    // Salvage contract: every acked store must survive the dump-and-rebuild.
    for text in &SENTINELS {
        recall_exact(port, &token, text);
    }
    let health2 = request_json(port, "GET", "/health", None, None).expect("health stats after repair");
    assert_eq!(
        health2.body["stats"]["decisions"].as_i64(),
        Some(3),
        "salvage must keep exactly the 3 stored decisions, got {}",
        health2.body
    );

    // The repaired database must be structurally sound, not merely readable.
    let conn = rusqlite::Connection::open(home_dir.join("cortex.db"))
        .expect("open repaired db");
    let quick: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .expect("quick_check on repaired db");
    assert_eq!(
        quick.trim().to_lowercase(),
        "ok",
        "repaired db must pass quick_check, got {quick:?}"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn repair_failure_degrades_predictably() {
    let home_dir = unique_temp_dir("dbrepair_degraded");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let home = seed_home(&home_dir);
    poison_btree_root_page(&home_dir, DERIVED_INDEX);

    // Repair can never succeed: auto_repair builds the fresh db at
    // <home>/cortex.repair_tmp (with_extension REPLACES "db"). A DIRECTORY at
    // that exact path makes Connection::open fail -> RepairError::OpenFresh,
    // while cortex.db itself stays openable so init.rs takes the
    // failed-repair branch instead of failing boot.
    fs::create_dir_all(home_dir.join("cortex.repair_tmp"))
        .expect("create repair blocker directory");

    // init.rs failed-repair path must still boot the server (degraded), not
    // exit: db_corrupted=true is set AFTER initialize_with_conn succeeds.
    let port = reserve_port();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);

    let health = request_json(port, "GET", "/health", None, None).expect("degraded health");
    assert_eq!(
        health.status, 200,
        "degraded daemon must still serve /health with 200, got {} body {}",
        health.status, health.body
    );
    assert_eq!(
        health.body["status"].as_str(),
        Some("degraded"),
        "failed repair must surface status degraded, got {}",
        health.body
    );
    assert_eq!(
        health.body["degraded"].as_bool(),
        Some(true),
        "failed repair must set degraded, got {}",
        health.body
    );
    assert_eq!(
        health.body["db_corrupted"].as_bool(),
        Some(true),
        "failed repair must set db_corrupted, got {}",
        health.body
    );
    assert_eq!(
        health.body["ready"].as_bool(),
        Some(true),
        "degraded boot must still reach ready, got {}",
        health.body
    );

    // A failed repair must NOT quarantine: the corrupt file stays in place.
    let archives = corrupt_archive_entries(&home_dir);
    assert!(
        archives.is_empty(),
        "failed repair must not quarantine the corrupt db, found {archives:?}"
    );

    // Serving must be sustained, not a one-shot: a second health read and the
    // readiness gate must both answer normally.
    let again = request_json(port, "GET", "/health", None, None).expect("second health read");
    assert_eq!(
        again.status, 200,
        "second health read must still be 200, got {} body {}",
        again.status, again.body
    );
    assert_eq!(
        again.body["status"].as_str(),
        Some("degraded"),
        "second health read must still report degraded, got {}",
        again.body
    );
    let readiness = request_json(port, "GET", "/readiness", None, None).expect("readiness");
    assert_eq!(
        readiness.status, 200,
        "readiness must be served with 200 in degraded mode, got {} body {}",
        readiness.status, readiness.body
    );
    assert_eq!(
        readiness.body["ready"].as_bool(),
        Some(true),
        "degraded daemon must report ready=true on /readiness, got {}",
        readiness.body
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}
