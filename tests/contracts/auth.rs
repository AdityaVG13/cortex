use cortex_kernel::auth::{
    cleanup_stale_pid_lock, pid_file_live_pid, CortexPaths, MAX_PID_FILE_BYTES,
};
use cortex_tests::in_subprocess;
use serde_json::Value;

#[test]
fn cortex_paths_resolve_and_serialize() {
    if !in_subprocess(
        "cortex_paths_resolve_and_serialize",
        &[
            ("CORTEX_HOME", None),
            ("CORTEX_DB", None),
        ],
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
        assert!(value.get(key).is_none(), "paths JSON must not advertise listen leftovers {key}");
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
