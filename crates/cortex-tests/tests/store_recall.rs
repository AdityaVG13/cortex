//! Curated behavioral contract for the store / recall path.
//!
//! Exercises the promoted test-support harness (`solo_state`, `test_conn`)
//! which is the seam the in-tree store/recall tests previously relied on.
use cortex_tests::support::{solo_state, test_conn};

#[test]
fn solo_state_has_expected_defaults() {
    let state = solo_state();
    assert!(!state.team_mode, "solo state must not be team mode");
    assert_eq!(state.port, 7437);
    assert_eq!(state.token.as_str(), "test-token");
}

#[test]
fn test_conn_builds_migrated_in_memory_db() {
    let conn = test_conn();

    // The connection must carry the migrated schema, not just open without
    // panicking. Probe for the core tables the migration is required to create.
    let table_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master \
             WHERE type='table' AND name IN ('memories','decisions','schema_migrations')",
            [],
            |row| row.get(0),
        )
        .expect("schema table probe");
    assert_eq!(
        table_count, 3,
        "migrated schema must contain memories, decisions, and schema_migrations"
    );

    // The connection must be writable and queryable -- prove a real round trip
    // rather than relying on construction not panicking.
    conn.execute("CREATE TABLE _contract_probe (v INTEGER)", [])
        .expect("probe table create");
    conn.execute("INSERT INTO _contract_probe (v) VALUES (7)", [])
        .expect("probe insert");
    let v: i64 = conn
        .query_row("SELECT v FROM _contract_probe", [], |row| row.get(0))
        .expect("probe select");
    assert_eq!(v, 7, "in-memory db must round-trip writes");
}
