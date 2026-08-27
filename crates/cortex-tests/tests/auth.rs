//! Curated behavioral contract for the auth / paths layer.
use cortex_daemon::auth::CortexPaths;

#[test]
fn cortex_paths_resolve_and_serialize() {
    let paths = CortexPaths::resolve();
    let json = paths.to_json();
    assert!(!json.is_empty(), "resolved paths must serialize to non-empty JSON");
}
