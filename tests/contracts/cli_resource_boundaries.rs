//! CLI lifecycle resource/boundary contracts (bucket B10, lens L4).
//!
//! Each test pins a boundary that was live-defect-confirmed before the fix:
//! 1. A huge `CORTEX_DAEMON_LOCK_WAIT_SECS` used to overflow
//!    `Instant::now() + Duration` and panic `cortex serve` at startup
//!    (whenever `CORTEX_WAIT_FOR_DAEMON_LOCK=1`, which the plugin-local
//!    spawn path always sets).
//! 2. `cortex restore` used to overwrite the database while the pid file
//!    recorded a live process, despite the documented daemon-active gate
//!    (capabilities `dangerous_operations` + robot-docs), and left the
//!    replaced database's WAL sidecars in place for SQLite to replay.
//! 3. `cortex migrate --home <x>` accepted `--home`/`--db` but silently
//!    migrated the env/default home instead.

#[path = "../support/mod.rs"]
mod support;

use std::fs;
use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

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
        .stdin(Stdio::null())
        .output()
        .expect("run cortex bin")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn lock_wait_env_overflow_must_not_panic_serve() {
    let _guard = support::daemon_spawn_test_guard();
    let home = unique_temp_home("lockwait");
    let port = free_port();
    let mut child = Command::new(cortex_tests::cortex_bin())
        .args([
            "serve",
            "--home",
            home.to_str().expect("utf-8 home"),
            "--port",
            &port.to_string(),
        ])
        .env("CORTEX_BIND", "127.0.0.1")
        .env("CORTEX_SINGLE_DAEMON_TEST_BYPASS", "1")
        .env("CORTEX_WAIT_FOR_DAEMON_LOCK", "1")
        // 2^64-1 parses as u64; `Instant::now() + Duration::from_secs(v)`
        // overflows. The lock wait must clamp its window instead of
        // panicking the serve path (cli/daemon/startup.rs).
        .env("CORTEX_DAEMON_LOCK_WAIT_SECS", "18446744073709551615")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn serve");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut healthy = false;
    while Instant::now() < deadline {
        if support::health_ok(port) {
            healthy = true;
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
    assert!(
        healthy,
        "daemon with huge lock-wait env did not become healthy on port {port}"
    );
    support::shutdown_daemon(port, &home);
    support::wait_for_exit(&mut child, Duration::from_secs(30));
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

    let output = run_bin(
        &["restore", backup.to_str().expect("utf-8 backup")],
        &home,
    );
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

#[test]
fn restore_drops_wal_sidecars_of_replaced_database() {
    let home = unique_temp_home("restore-wal");
    fs::write(home.join("cortex.db"), b"old-db").expect("write db");
    fs::write(home.join("cortex.db-wal"), b"stale-wal-frames").expect("write wal");
    fs::write(home.join("cortex.db-shm"), b"stale-shm").expect("write shm");
    let backup = home.join("backup.db");
    fs::write(&backup, b"restored-bytes").expect("write backup");

    let output = run_bin(
        &["restore", backup.to_str().expect("utf-8 backup")],
        &home,
    );
    assert!(
        output.status.success(),
        "restore without a live pid must succeed; stderr:\n{}",
        stderr_of(&output)
    );
    assert_eq!(
        fs::read(home.join("cortex.db")).expect("read db"),
        b"restored-bytes",
        "restored database must equal the backup"
    );
    assert!(
        !home.join("cortex.db-wal").exists() && !home.join("cortex.db-shm").exists(),
        "restore must drop the replaced database's WAL sidecars; stale frames \
         belong to the replaced file and would be replayed onto the restore"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn migrate_honors_home_flag_over_env_home() {
    let env_home = unique_temp_home("mig-env");
    let flag_home = unique_temp_home("mig-flag");
    let output = Command::new(cortex_tests::cortex_bin())
        .args([
            "migrate",
            "--home",
            flag_home.to_str().expect("utf-8 flag home"),
            "--owner",
            "owner-a",
            "--display-name",
            "Owner A",
        ])
        .env("CORTEX_HOME", &env_home)
        .stdin(Stdio::null())
        .output()
        .expect("run migrate");
    assert!(
        output.status.success(),
        "migrate with --owner must succeed; stderr:\n{}",
        stderr_of(&output)
    );
    assert!(
        flag_home.join("cortex.db").exists(),
        "--home flag must select the migration target; stderr:\n{}",
        stderr_of(&output)
    );
    assert!(
        !env_home.join("cortex.db").exists(),
        "migrate must not touch the env home when --home is given \
         (silent wrong-database migration)"
    );
    let _ = fs::remove_dir_all(&env_home);
    let _ = fs::remove_dir_all(&flag_home);
}
