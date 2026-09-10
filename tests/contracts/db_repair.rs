//! Real on-disk corruption, repair/quarantine, and degraded-mode contracts.
//! Retired: listener startup, health polling and HTTP status assertions.
//! Runtime open exercises the same production initialization/repair branch.
use cortex_daemon::handlers::health::{build_health_payload, build_readiness_payload};
use cortex_daemon::{CortexRuntime, runtime::LensInput};
use cortex_tests::support::run_with_cx;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};

const SENTINELS: [&str; 3] = [
    "Auto-repair salvage sentinel one: the archive rotator keeps seven nightly snapshots on the vault volume.",
    "Auto-repair salvage sentinel two: the checksum auditor rejects any tile whose crc32 mismatches the manifest.",
    "Auto-repair salvage sentinel three: the replication lag alarm fires when the follower falls 900 seconds behind.",
];

fn archives(home: &std::path::Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(home)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("cortex.corrupt.")
        })
        .collect()
}

fn poison_index(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    let page_size: u32 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap();
    let root: u32 = conn
        .query_row(
            "SELECT rootpage FROM sqlite_master WHERE name = 'idx_memories_status'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);
    // SQLite rootpage numbers are ONE based; never poison the adjacent data page.
    assert!(root > 1);
    let offset = u64::from(root - 1) * u64::from(page_size);
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    assert!(offset + u64::from(page_size) <= file.metadata().unwrap().len());
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&vec![0xff; page_size as usize]).unwrap();
    file.sync_all().unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    for pragma in ["PRAGMA quick_check", "PRAGMA integrity_check"] {
        let ok = conn
            .query_row(pragma, [], |r| r.get::<_, String>(0))
            .map(|s| s.eq_ignore_ascii_case("ok"))
            .unwrap_or(false);
        assert!(!ok, "poison must trip {pragma}");
    }
}

#[test]
fn poisoned_db_boots_via_auto_repair() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("cortex.db");
        let runtime = CortexRuntime::open_db(&path).unwrap();
        for (i, text) in SENTINELS.iter().enumerate() {
            runtime
                .deposit(&cx, &format!("seed-{i}"), text, "db-repair", None)
                .await
                .unwrap();
        }
        drop(runtime);
        poison_index(&path);
        let runtime = CortexRuntime::open_db(&path).expect("production auto-repair boot");
        let health = build_health_payload(&cx, runtime.state(), false)
            .await
            .unwrap();
        assert_eq!(health["status"], "ok");
        assert_eq!(health["degraded"], false);
        assert_eq!(health["db_corrupted"], false);
        assert_eq!(health["stats"]["decisions"], 3);
        let archived = archives(home.path());
        assert_eq!(archived.len(), 1);
        let mut header = [0; 16];
        fs::File::open(&archived[0])
            .unwrap()
            .read_exact(&mut header)
            .unwrap();
        assert_eq!(&header, b"SQLite format 3\0");
        for text in SENTINELS {
            let recall = runtime
                .lens(
                    &cx,
                    LensInput {
                        query: text.into(),
                        k: 10,
                        budget: 320,
                        agent: "db-repair".into(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert!(
                recall["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["excerpt"] == text),
                "{recall}"
            );
        }
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        let quick: String = conn
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(quick, "ok");
    });
}

#[test]
fn repair_failure_degrades_predictably() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("cortex.db");
        let runtime = CortexRuntime::open_db(&path).unwrap();
        for (i, text) in SENTINELS.iter().enumerate() {
            runtime
                .deposit(&cx, &format!("seed-{i}"), text, "db-repair", None)
                .await
                .unwrap();
        }
        drop(runtime);
        poison_index(&path);
        fs::create_dir(home.path().join("cortex.repair_tmp")).unwrap();
        let runtime =
            CortexRuntime::open_db(&path).expect("repair failure must still open degraded");
        for _ in 0..2 {
            let health = build_health_payload(&cx, runtime.state(), false)
                .await
                .unwrap();
            assert_eq!(health["status"], "degraded");
            assert_eq!(health["degraded"], true);
            assert_eq!(health["db_corrupted"], true);
            assert_eq!(health["ready"], true);
        }
        assert_eq!(
            build_readiness_payload(runtime.state(), false)["ready"],
            true
        );
        assert!(
            archives(home.path()).is_empty(),
            "failed repair must not quarantine"
        );
    });
}

#[test]
fn auto_repair_preserves_identity_or_honestly_downgrades() {
    // Retain all former identity cases: intact, missing config, missing users,
    // and a structurally readable but illegal mode value.
    for damage in ["intact", "config", "users", "garbled"] {
        run_with_cx(|cx| async move {
            let home = tempfile::tempdir().unwrap();
            let path = home.path().join("cortex.db");
            {
                let conn = cortex_tests::support::open_file_db(&path);
                cortex_daemon::db::create_team_mode_tables(&conn).unwrap();
                let owner = cortex_daemon::db::upsert_owner_user(
                    &conn,
                    "repair-owner",
                    None,
                    "hash-owner-verbatim",
                )
                .unwrap();
                cortex_daemon::db::migrate_to_team_mode(&conn, owner).unwrap();
                conn.execute("INSERT INTO decisions (decision, type, status) VALUES ('team salvage sentinel', 'decision', 'active')", []).unwrap();
                assert_eq!(cortex_daemon::db::current_mode(&conn), "team");
                match damage {
                    "config" => {
                        conn.execute("DELETE FROM config", []).unwrap();
                    }
                    "users" => {
                        conn.execute("DROP TABLE users", []).unwrap();
                    }
                    "garbled" => {
                        conn.execute(
                            "UPDATE config SET value = 'corrupted' WHERE key = 'mode'",
                            [],
                        )
                        .unwrap();
                        let value: String = conn
                            .query_row("SELECT value FROM config WHERE key = 'mode'", [], |r| {
                                r.get(0)
                            })
                            .unwrap();
                        assert_eq!(value, "corrupted");
                    }
                    _ => {}
                }
            }
            let result = cortex_daemon::db::auto_repair(&path, damage).unwrap();
            assert_eq!(result.decisions_recovered, 1);
            let runtime = CortexRuntime::open_db(&path).unwrap();
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            assert_eq!(
                cortex_daemon::db::current_mode(&conn),
                if damage == "intact" { "team" } else { "solo" }
            );
            if damage != "users" {
                let (users, hash): (i64, String) = conn.query_row("SELECT COUNT(*), COALESCE((SELECT api_key_hash FROM users WHERE username = 'repair-owner'), '') FROM users", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
                assert_eq!(users, 1);
                assert_eq!(hash, "hash-owner-verbatim");
                if damage == "config" {
                    for table in ["teams", "team_members"] {
                        let count: i64 = conn
                            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                            .unwrap();
                        assert_eq!(count, 1);
                    }
                }
            }
        });
    }
}

#[test]
fn sqlite_wal_reset_gate_matches_documented_fix_set() {
    use cortex_daemon::db::{sqlite_version, sqlite_wal_reset_fixed};
    for (version, fixed) in [
        ("3.51.2", false),
        ("3.51.3", true),
        ("3.52.0", true),
        ("3.44.5", false),
        ("3.44.6", true),
        ("3.45.0", false),
        ("3.50.6", false),
        ("3.50.7", true),
        ("garbage", false),
        ("4.0.0", true),
    ] {
        assert_eq!(sqlite_wal_reset_fixed(version), fixed, "{version}");
    }
    assert!(sqlite_version().starts_with("3."));
}
