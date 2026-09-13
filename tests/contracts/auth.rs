use cortex_kernel::auth::{
    acquire_daemon_lock, cleanup_stale_pid_file, cleanup_stale_pid_lock, pid_file_live_pid,
    read_secret_file, read_token_from, write_secret_file, CortexPaths, MAX_PID_FILE_BYTES,
};
use cortex_kernel::runtime::CortexRuntime;
use cortex_tests::in_subprocess;
use serde_json::Value;
use std::time::{Duration, Instant};

#[test]
fn cortex_paths_resolve_and_serialize() {
    if !in_subprocess(
        "cortex_paths_resolve_and_serialize",
        &[("CORTEX_HOME", None), ("CORTEX_DB", None)],
    ) {
        return;
    }

    let paths = CortexPaths::resolve();
    let json = paths.to_json();
    let value: Value = serde_json::from_str(&json).expect("resolved paths serialize to valid JSON");

    for key in ["home", "db", "token", "pid"] {
        assert!(value.get(key).is_some(), "paths JSON missing key {key}");
    }
    for key in ["port", "bind", "ipc_endpoint", "ipc_kind"] {
        assert!(
            value.get(key).is_none(),
            "paths JSON must not advertise listen leftovers {key}"
        );
    }

    let home = value["home"].as_str().expect("home is a string");
    assert!(
        home.ends_with(".cortex"),
        "home must resolve under the .cortex directory, got {home}"
    );
    let db = value["db"].as_str().expect("db is a string");
    assert!(
        db.ends_with("cortex.db"),
        "db path must end with cortex.db, got {db}"
    );
    let token = value["token"].as_str().expect("token is a string");
    assert!(
        token.ends_with("cortex.token"),
        "token path must end with cortex.token, got {token}"
    );
}

#[test]
fn cortex_paths_honor_cortex_home() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("custom-home");
    if !in_subprocess(
        "cortex_paths_honor_cortex_home",
        &[("CORTEX_HOME", Some(home.as_os_str())), ("CORTEX_DB", None)],
    ) {
        return;
    }
    let home =
        std::path::PathBuf::from(std::env::var_os("CORTEX_HOME").expect("child home override"));
    let paths = CortexPaths::resolve();
    assert_eq!(paths.home, home, "CORTEX_HOME must become paths.home");
    assert_eq!(paths.db, home.join("cortex.db"));
    assert_eq!(paths.token, home.join("cortex.token"));
    assert_eq!(paths.pid, home.join("cortex.pid"));
}

#[test]
fn pid_file_reads_refuse_an_oversize_replacement() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let db = home.join("cortex.db");
    let paths = CortexPaths::resolve_with_overrides(
        Some(&home.to_string_lossy()),
        Some(&db.to_string_lossy()),
    );
    let padded = format!("1\n{}", " ".repeat(MAX_PID_FILE_BYTES as usize));
    assert!(padded.len() as u64 > MAX_PID_FILE_BYTES);
    std::fs::write(&paths.pid, padded).expect("write padded pid");
    assert!(
        pid_file_live_pid(&paths).is_none(),
        "oversize pid must not parse as live pid 1"
    );
}

#[test]
fn cleanup_stale_pid_lock_does_not_remove_this_process() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let db = home.join("cortex.db");
    let paths = CortexPaths::resolve_with_overrides(
        Some(&home.to_string_lossy()),
        Some(&db.to_string_lossy()),
    );
    std::fs::write(&paths.pid, format!("{}\n", std::process::id())).expect("write live pid");
    assert!(
        cleanup_stale_pid_lock(&paths).is_none(),
        "cleanup must not unlink a pid that is still running"
    );
    let recorded = std::fs::read_to_string(&paths.pid).expect("pid still present");
    assert_eq!(recorded.trim(), std::process::id().to_string());
}

#[test]
fn cleanup_stale_pid_lock_removes_a_dead_occupant_under_the_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let db = home.join("cortex.db");
    let paths = CortexPaths::resolve_with_overrides(
        Some(&home.to_string_lossy()),
        Some(&db.to_string_lossy()),
    );
    // Pid 0 is never a recorded daemon; process_is_running rejects it.
    std::fs::write(&paths.pid, "0\n").expect("write dead pid");
    assert_eq!(cleanup_stale_pid_lock(&paths), Some(0));
    assert!(
        !paths.pid.exists(),
        "stale pid 0 must be unlinked after the cleanup lock is held"
    );
}

#[test]
fn cortex_paths_blank_cortex_home_falls_back_to_default() {
    if !in_subprocess(
        "cortex_paths_blank_cortex_home_falls_back_to_default",
        &[
            ("CORTEX_HOME", Some(std::ffi::OsStr::new("   "))),
            ("CORTEX_DB", None),
        ],
    ) {
        return;
    }
    let paths = CortexPaths::resolve();
    let home = paths.home.to_string_lossy();
    assert!(
        home.ends_with(".cortex"),
        "whitespace CORTEX_HOME must not become the token home, got {home}"
    );
    assert_eq!(paths.token, paths.home.join("cortex.token"));
}

#[test]
fn cleanup_stale_pid_lock_does_not_block_when_caller_holds_flock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let db = home.join("cortex.db");
    let paths = CortexPaths::resolve_with_overrides(
        Some(&home.to_string_lossy()),
        Some(&db.to_string_lossy()),
    );
    let _lock = acquire_daemon_lock(&paths).expect("hold home lock");
    std::fs::write(&paths.pid, "0\n").expect("write stale pid");
    let started = Instant::now();
    assert!(
        cleanup_stale_pid_lock(&paths).is_none(),
        "second flock on the same inode must fail closed, not wait"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "re-acquire must not deadlock with the held serve/restore lock"
    );
    assert!(
        paths.pid.exists(),
        "failed re-acquire must leave the pid file for the lock holder"
    );
    assert_eq!(cleanup_stale_pid_file(&paths), Some(0));
    assert!(!paths.pid.exists());
}

#[test]
fn write_secret_file_replaces_world_readable_without_leaving_new_bytes_at_0644() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cortex.token");
    std::fs::write(&path, b"old-token\n").expect("seed token");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    }
    write_secret_file(&path, b"new-token\n").expect("replace token");
    assert_eq!(
        read_secret_file(&path).expect("read replaced token"),
        b"new-token\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "replaced secret must be owner-only");
    }
}

#[test]
fn solo_open_persists_missing_shared_token_for_tauri() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let db = home.join("cortex.db");
    let paths = CortexPaths::resolve_with_overrides(
        Some(&home.to_string_lossy()),
        Some(&db.to_string_lossy()),
    );
    assert!(!paths.token.exists());
    CortexRuntime::open(&paths).expect("open brain");
    let token = read_token_from(&paths).expect("open must persist cortex.token for Control Center");
    assert!(!token.is_empty(), "persisted token must be non-empty");
    let again = CortexRuntime::open(&paths).expect("reopen");
    drop(again);
    assert_eq!(
        read_token_from(&paths).as_deref(),
        Some(token.as_str()),
        "reopen must not rotate the file token Tauri already holds"
    );
}
