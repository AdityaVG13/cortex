//! CLI lifecycle resource/boundary contracts (bucket B10, lens L4).
//!
//! Each test pins a boundary that was live-defect-confirmed before the fix:
//! 1. Legacy lock-wait environment values must not panic the headless worker.
//!    The local-only worker acquires its lock immediately, without HTTP health
//!    checks or the retired plugin-local waiting path.
//! 2. `cortex restore` used to overwrite the database while the pid file
//!    recorded a live process, despite the documented daemon-active gate
//!    (capabilities `dangerous_operations` + robot-docs), and left the
//!    replaced database's WAL sidecars in place for SQLite to replay.
//! 3. Retired team migration must reject before touching either the explicit
//!    home/database or the environment-selected home.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn unique_temp_home(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("cortex-clibnd-{prefix}-{nanos}"));
    fs::create_dir_all(&dir).expect("create temp home");
    dir
}

fn run_bin(args: &[&str], home: &Path) -> Output {
    Command::new(cortex_tests::cortex_bin())
        .args(args)
        .env("CORTEX_HOME", home)
        .env_remove("CORTEX_DB")
        .stdin(Stdio::null())
        .output()
        .expect("run cortex bin")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn lock_wait_env_overflow_must_not_panic_serve() {
    let home = unique_temp_home("lockwait");
    let mut child = Command::new(cortex_tests::cortex_bin())
        .args(["serve", "--home", home.to_str().expect("utf-8 home")])
        .env_remove("CORTEX_DB")
        .env("CORTEX_WAIT_FOR_DAEMON_LOCK", "1")
        // Legacy clients may still supply u64::MAX. The local worker must
        // initialize successfully despite this now-inert lock-wait setting.
        .env("CORTEX_DAEMON_LOCK_WAIT_SECS", "18446744073709551615")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn serve");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut healthy = false;
    while Instant::now() < deadline {
        // Headless readiness is an initialized local database, not an HTTP probe.
        let initialized = rusqlite::Connection::open_with_flags(
            home.join("cortex.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .and_then(|conn| {
            conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'memories'",
                [],
                |row| row.get::<_, i64>(0),
            )
        })
        .unwrap_or(0)
            == 1;
        if initialized && child.try_wait().expect("poll initialized worker").is_none() {
            healthy = true;
            let recorded = fs::read_to_string(home.join("cortex.pid")).unwrap_or_default();
            assert_eq!(
                recorded.trim(),
                child.id().to_string(),
                "serve must write cortex.pid so restore can see a live worker"
            );
            let backup = home.join("restore-while-serve.db");
            fs::write(&backup, b"not-a-database").expect("write dummy backup");
            let restore = run_bin(
                &["restore", backup.to_str().expect("utf-8 backup")],
                &home,
            );
            let restore_err = stderr_of(&restore);
            assert!(
                !restore.status.success(),
                "restore must refuse while serve holds the home; stderr:\n{restore_err}"
            );
            assert!(
                restore_err.contains("daemon appears active"),
                "refusal must name the daemon-active gate; stderr:\n{restore_err}"
            );
            break;
        }
        match child.try_wait().expect("poll serve") {
            Some(status) => {
                let mut stderr = String::new();
                if let Some(handle) = child.stderr.as_mut() {
                    let _ = handle.read_to_string(&mut stderr);
                }
                assert!(
                    !stderr.contains("overflow when adding duration to instant"),
                    "serve panicked on huge CORTEX_DAEMON_LOCK_WAIT_SECS (status={status}):\n{stderr}"
                );
                panic!("serve exited before health (status={status}):\n{stderr}");
            }
            None => thread::sleep(Duration::from_millis(250)),
        }
    }
    // Always reap the headless worker, including the readiness timeout path.
    let _ = child.kill();
    let output = child.wait_with_output().expect("reap headless worker");
    assert!(
        healthy,
        "worker did not initialize the local brain: {}",
        stderr_of(&output)
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn restore_refuses_while_pid_file_records_live_process() {
    let home = unique_temp_home("restore-live");
    fs::write(home.join("cortex.db"), b"live-db-marker").expect("write db");
    // The test process itself is the live "daemon" from the CLI child's
    // perspective: its pid stays alive for the whole test.
    fs::write(home.join("cortex.pid"), std::process::id().to_string()).expect("write pid");
    let backup = home.join("backup.db");
    fs::write(&backup, b"backup-bytes").expect("write backup");

    let output = run_bin(&["restore", backup.to_str().expect("utf-8 backup")], &home);
    let stderr = stderr_of(&output);
    assert!(
        !output.status.success(),
        "restore must refuse while a live pid is recorded; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("daemon appears active"),
        "refusal must name the documented daemon-active gate; stderr:\n{stderr}"
    );
    assert_eq!(
        fs::read(home.join("cortex.db")).expect("read db"),
        b"live-db-marker",
        "restore overwrote the live daemon's database"
    );
    let _ = fs::remove_dir_all(&home);
}

fn sqlite_with_marker(path: &Path, marker: &str) {
    let conn = rusqlite::Connection::open(path).expect("open sqlite");
    conn.execute_batch("CREATE TABLE IF NOT EXISTS marker (text TEXT)")
        .expect("marker table");
    conn.execute("INSERT INTO marker (text) VALUES (?1)", [marker])
        .expect("marker row");
}

fn marker_of(path: &Path) -> String {
    let conn = rusqlite::Connection::open(path).expect("open sqlite");
    conn.query_row(
        "SELECT text FROM marker ORDER BY rowid DESC LIMIT 1",
        [],
        |r| r.get(0),
    )
    .unwrap_or_default()
}

/// Restore goes through the SQLite online backup API into the live file, so
/// stale WAL frames of the replaced database can never be replayed onto the
/// restored pages: the marker after restore is the backup's, not the old
/// WAL's, and a non-database "backup" is refused before anything is touched.
#[test]
fn restore_replaces_content_coherently_and_refuses_non_database_backups() {
    let home = unique_temp_home("restore-wal");
    sqlite_with_marker(&home.join("cortex.db"), "old-db");
    let backup = home.join("backup.db");
    sqlite_with_marker(&backup, "restored-marker");
    let output = run_bin(&["restore", backup.to_str().expect("utf-8 backup")], &home);
    assert!(
        output.status.success(),
        "restore without a live pid must succeed; stderr:\n{}",
        stderr_of(&output)
    );
    assert_eq!(
        marker_of(&home.join("cortex.db")),
        "restored-marker",
        "restored database must carry the backup's content"
    );
    let report =
        fs::read_to_string(home.join(".last_verified_restore.json")).expect("verification report");
    assert!(
        report.contains("\"verified\": true"),
        "restore must record a verification report: {report}"
    );
    let bogus = home.join("bogus.db");
    fs::write(&bogus, b"not-a-database").expect("write bogus");
    let output = run_bin(&["restore", bogus.to_str().expect("utf-8")], &home);
    assert!(
        !output.status.success(),
        "a non-database backup must be refused; stderr:\n{}",
        stderr_of(&output)
    );
    assert_eq!(
        marker_of(&home.join("cortex.db")),
        "restored-marker",
        "refused restore leaves the live database untouched"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn restore_fails_loudly_when_wal_sidecar_cannot_be_removed() {
    // A directory at the -wal path blocks SQLite from opening the target in
    // WAL mode; the failure must surface, never "Restore complete".
    let home = unique_temp_home("restore-wal-stuck");
    sqlite_with_marker(&home.join("cortex.db"), "old-db");
    let backup = home.join("backup.db");
    sqlite_with_marker(&backup, "restored-marker");
    fs::create_dir(home.join("cortex.db-wal")).expect("create wal-dir blocker");
    let output = run_bin(&["restore", backup.to_str().expect("utf-8 backup")], &home);
    let stderr = stderr_of(&output);
    assert!(
        !output.status.success(),
        "restore must fail loudly when the WAL sidecar path is blocked; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("restore failed"),
        "failure must be reported as a restore failure; stderr:\n{stderr}"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn retired_migrate_rejects_without_touching_flag_or_env_home() {
    let env_home = unique_temp_home("mig-env");
    let flag_home = unique_temp_home("mig-flag");
    let explicit_db = flag_home.join("explicit.db");
    let output = Command::new(cortex_tests::cortex_bin())
        .args([
            "migrate",
            "--home",
            flag_home.to_str().expect("utf-8 flag home"),
            "--db",
            explicit_db.to_str().expect("utf-8 explicit db"),
            "--owner",
            "owner-a",
            "--display-name",
            "Owner A",
        ])
        .env("CORTEX_HOME", &env_home)
        .env("CORTEX_DB", env_home.join("env.db"))
        .stdin(Stdio::null())
        .output()
        .expect("run migrate");
    assert_eq!(
        output.status.code(),
        Some(1),
        "retired migrate must fail: {output:?}"
    );
    assert!(output.stdout.is_empty(), "retired migrate wrote stdout");
    assert_eq!(
        stderr_of(&output).replace("\r\n", "\n"),
        "[cortex] Unknown command: migrate\nRun `cortex help` or `cortex capabilities --json` for supported commands.\n"
    );
    assert_eq!(
        fs::read_dir(&flag_home).unwrap().count(),
        0,
        "migration touched flag home"
    );
    assert_eq!(
        fs::read_dir(&env_home).unwrap().count(),
        0,
        "migration touched env home"
    );
    let _ = fs::remove_dir_all(&env_home);
    let _ = fs::remove_dir_all(&flag_home);
}
