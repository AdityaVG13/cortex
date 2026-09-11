//! HYP-023 / cortex-e1f: the repo's first property test — migration version
//! monotonicity.
//!
//! Spec under test (crates/daemon/src/db/migrations.rs):
//! - P1 (declared table): `migration_definitions()` versions, under
//!   leading-digit parsing, are strictly increasing (sorted + unique) and every
//!   version carries a positive numeric prefix. Kills: swapped/duplicated/
//!   renumbered rows in the hand-maintained 22-entry table.
//! - P2 (proptest, generated): `pending_migration_versions` is a pure function
//!   of the SET of applied versions — never of their insertion order — and
//!   always yields exactly the declared-order complement, strictly increasing.
//!   Applying the pending set records each migration exactly once, a second
//!   run applies zero, and PRAGMA user_version (computed by the product's
//!   private parser) equals the last declared version. Kills: order-dependent
//!   scheduling, double-recording, non-idempotent replay, parse drift.
//! - P3 (behavioral, file-backed DBs): a full run records the declared list
//!   in declared application order, exactly once; a second
//!   `initialize_schema` + `run_pending_migrations` applies zero; a mid-version
//!   DB (schema_migrations seeded through declared entry K) applies only the
//!   versions above K, in declared order, and reaches the same final state.
//!
//! Determinism: proptest 1.11 has no `Config::source_seed`, so the fixed-seed
//! equivalent is used — `TestRunner::new_with_rng` with
//! `TestRng::deterministic_rng(RngAlgorithm::ChaCha)` (fixed library seed) and
//! `Config::with_cases(64)`. The generated case stream is identical across
//! runs on the same proptest version.

#[path = "../support/mod.rs"]
mod support;

use std::collections::HashSet;

use cortex_kernel::db;
use proptest::prelude::*;
use proptest::test_runner::{RngAlgorithm, TestRng};
use rusqlite::{params, Connection};

/// Local mirror of `db::migration_user_version` (pub(crate)-sealed): leading
/// ASCII digits as i32, 0 when absent. It cannot drift unnoticed: the DB-side
/// properties (P2, P3) assert PRAGMA user_version — computed by the product's
/// private parser during migration application — equals this parse of the
/// last declared version.
fn leading_version(version: &str) -> i32 {
    version
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse::<i32>()
        .unwrap_or(0)
}

fn declared_versions() -> Vec<&'static str> {
    db::migration_definitions()
        .iter()
        .map(|(version, _)| *version)
        .collect()
}

/// P1 — the declared table itself is the invariant carrier.
#[test]
fn declared_migration_versions_are_strictly_increasing_and_unique() {
    let defs = db::migration_definitions();
    assert!(!defs.is_empty(), "migration table must not be empty");
    let mut seen_versions: HashSet<&str> = HashSet::new();
    let mut seen_names: HashSet<&str> = HashSet::new();
    let mut parsed: Vec<i32> = Vec::with_capacity(defs.len());
    for (version, name) in defs {
        let n = leading_version(version);
        assert!(
            n > 0,
            "version {version:?} must carry a positive numeric prefix"
        );
        assert!(
            seen_versions.insert(version),
            "duplicate migration version {version:?}"
        );
        assert!(seen_names.insert(name), "duplicate migration name {name:?}");
        parsed.push(n);
    }
    for pair in parsed.windows(2) {
        assert!(
            pair[0] < pair[1],
            "migration versions must be strictly increasing: {} immediately followed by {}",
            pair[0],
            pair[1]
        );
    }
}

/// File-backed connection through the real boot path up to (but not past)
/// migration application, so the test controls schema_migrations state.
fn seeded_file_conn(dir: &std::path::Path) -> Connection {
    let conn = Connection::open(dir.join("cortex.db")).expect("open sqlite file db");
    db::configure(&conn).expect("configure db");
    db::initialize_schema(&conn).expect("initialize schema");
    conn
}

fn recorded_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
        row.get(0)
    })
    .expect("count schema_migrations")
}

/// P3a — full run records each migration exactly once; re-running the boot
/// init applies nothing.
#[test]
fn full_run_records_each_migration_once_and_reinitialize_applies_zero() {
    let defs = db::migration_definitions();
    let declared = declared_versions();
    let dir = support::unique_temp_dir("migration-property-full");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let conn = seeded_file_conn(&dir);

    let first = db::run_pending_migrations(&conn);
    assert_eq!(
        first,
        defs.len(),
        "first run must apply every declared migration"
    );
    let recorded = db::applied_migration_versions(&conn).expect("read applied");
    assert_eq!(
        recorded, declared,
        "schema_migrations must record the declared list in application order"
    );
    let count = recorded_count(&conn);
    assert_eq!(
        count as usize,
        defs.len(),
        "each migration recorded exactly once"
    );
    assert_eq!(
        db::current_schema_user_version(&conn).expect("user_version"),
        leading_version(declared.last().expect("non-empty table")),
        "PRAGMA user_version (product parse) must equal the last declared version"
    );

    // Idempotence: second boot init + migration sweep is a no-op.
    db::initialize_schema(&conn).expect("second initialize_schema");
    let second = db::run_pending_migrations(&conn);
    assert_eq!(
        second, 0,
        "second initialize + run must apply zero migrations"
    );
    assert_eq!(
        recorded_count(&conn),
        count,
        "schema_migrations count must not change"
    );
}

/// P3b — a mid-version DB (seeded through declared entry K) is brought to
/// head by applying only versions > K, in declared order.
#[test]
fn mid_version_db_applies_only_versions_above_k_in_declared_order() {
    let defs = db::migration_definitions();
    let k = defs.len() / 2;
    let dir = support::unique_temp_dir("migration-property-mid");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let conn = seeded_file_conn(&dir);

    for (version, name) in &defs[..k] {
        conn.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            params![version, name],
        )
        .expect("seed applied migration row");
    }
    let expected_suffix: Vec<&str> = defs[k..].iter().map(|(version, _)| *version).collect();
    assert_eq!(
        db::pending_migration_versions(&conn).expect("pending before run"),
        expected_suffix,
        "pending must be exactly the declared suffix above K"
    );

    let applied = db::run_pending_migrations(&conn);
    assert_eq!(applied, defs.len() - k, "only versions above K may apply");

    let recorded = db::applied_migration_versions(&conn).expect("read applied after run");
    assert_eq!(
        recorded.len(),
        defs.len(),
        "all migrations recorded after catch-up"
    );
    assert_eq!(
        &recorded[k..],
        expected_suffix.as_slice(),
        "newly applied rows must be the declared suffix in declared order"
    );
    let recorded_set: HashSet<&str> = recorded.iter().map(|s| s.as_str()).collect();
    for (version, _) in defs {
        assert!(
            recorded_set.contains(version),
            "migration {version:?} missing after catch-up"
        );
    }
    assert_eq!(
        db::current_schema_user_version(&conn).expect("user_version after catch-up"),
        leading_version(defs.last().expect("non-empty table").0),
        "PRAGMA user_version (product parse) must equal the last declared version"
    );
}

/// P2 — generated permutation invariance. For every generated unique index
/// set S (in generated insertion order): pending == declared \ S in declared
/// order, independent of insertion permutation; the catch-up run applies
/// exactly pending, records each once, a rerun applies zero, and user_version
/// reaches the product maximum.
#[test]
fn pending_migrations_depends_only_on_applied_set_not_insertion_order() {
    let defs = db::migration_definitions();
    let n = defs.len() as u32;
    let strategy = proptest::collection::vec(0u32..n, 0..=n as usize);
    let mut config = proptest::test_runner::Config::with_cases(64);
    config.test_name = Some(
        "migration_property::pending_migrations_depends_only_on_applied_set_not_insertion_order"
            .into(),
    );
    let mut runner = proptest::test_runner::TestRunner::new_with_rng(
        config,
        TestRng::deterministic_rng(RngAlgorithm::ChaCha),
    );
    let result = runner.run(&strategy, |raw_indices: Vec<u32>| {
        // Dedupe preserving first occurrence: unique applied set + generated
        // (permuted) insertion order.
        let mut seen: HashSet<u32> = HashSet::new();
        let indices: Vec<u32> = raw_indices
            .into_iter()
            .filter(|i| seen.insert(*i))
            .collect();
        let applied: HashSet<&str> = indices.iter().map(|&i| defs[i as usize].0).collect();

        let conn = Connection::open_in_memory().expect("open in-memory db");
        db::configure(&conn).expect("configure db");
        db::initialize_schema(&conn).expect("initialize schema");
        for &i in &indices {
            let (version, name) = defs[i as usize];
            conn.execute(
                "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                params![version, name],
            )
            .expect("seed applied migration row");
        }

        let expected: Vec<&str> = declared_versions()
            .into_iter()
            .filter(|version| !applied.contains(version))
            .collect();
        let expected_len = expected.len();
        let pending = db::pending_migration_versions(&conn).expect("pending");
        let parsed_pending: Vec<i32> = pending.iter().map(|v| leading_version(v)).collect();
        prop_assert_eq!(
            pending,
            expected,
            "pending must depend only on the applied SET"
        );
        for pair in parsed_pending.windows(2) {
            prop_assert!(
                pair[0] < pair[1],
                "pending versions must be strictly increasing: {} !< {}",
                pair[0],
                pair[1]
            );
        }

        let catch_up = db::run_pending_migrations(&conn);
        prop_assert_eq!(
            catch_up,
            expected_len,
            "catch-up must apply exactly the pending set"
        );
        prop_assert_eq!(
            db::run_pending_migrations(&conn),
            0,
            "replay must apply zero"
        );
        let recorded = db::applied_migration_versions(&conn).expect("recorded");
        prop_assert_eq!(
            recorded.len(),
            defs.len(),
            "each migration recorded exactly once"
        );
        let recorded_set: HashSet<String> = recorded.into_iter().collect();
        for (version, _) in defs {
            prop_assert!(
                recorded_set.contains(*version),
                "migration {version:?} missing"
            );
        }
        prop_assert_eq!(
            db::current_schema_user_version(&conn).expect("user_version"),
            leading_version(defs.last().expect("non-empty table").0),
            "user_version (product parse) must reach the last declared version"
        );
        Ok(())
    });
    result.expect("property pending_migrations_depends_only_on_applied_set failed");
}
