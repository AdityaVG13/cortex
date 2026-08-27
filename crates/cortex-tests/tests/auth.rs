//! Curated behavioral contract for the auth / paths layer.
use cortex_daemon::auth::CortexPaths;
use serde_json::Value;

#[test]
fn cortex_paths_resolve_and_serialize() {
    let paths = CortexPaths::resolve();
    let json = paths.to_json();
    let value: Value = serde_json::from_str(&json).expect("resolved paths serialize to valid JSON");

    // Every resolved path key must be present and well-formed -- not just
    // "non-empty JSON" (which a degenerate `{{}}` would satisfy).
    for key in ["home", "db", "token", "pid", "port", "bind", "models"] {
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
