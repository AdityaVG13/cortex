use cortex_daemon::auth::CortexPaths;
use cortex_tests::in_subprocess;
use serde_json::Value;

#[test]
fn cortex_paths_resolve_and_serialize() {
    if !in_subprocess(
        "cortex_paths_resolve_and_serialize",
        &[
            ("CORTEX_HOME", None),
            ("CORTEX_DB", None),
            ("CORTEX_PORT", None),
            ("CORTEX_BIND", None),
        ],
    ) {
        return;
    }

    let paths = CortexPaths::resolve();
    let json = paths.to_json();
    let value: Value = serde_json::from_str(&json).expect("resolved paths serialize to valid JSON");

    for key in ["home", "db", "token", "pid", "port", "bind"] {
        assert!(value.get(key).is_some(), "paths JSON missing key {key}");
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
    let port = value["port"].as_u64().expect("port is a number");
    assert_eq!(port, 7437, "default resolved port must be 7437");
    assert_eq!(
        value["bind"].as_str(),
        Some("127.0.0.1"),
        "default bind must be loopback"
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
