//! Hash-epoch upgrade: a pre-cutover brain (SHA-256 markers, 16-hex
//! SipHash/FNV seals, legacy cache ids) opens cleanly under the new binary.
//! Memories survive byte-for-byte and stay recallable; caches purge or
//! self-heal; committed ledger history is kept and fails closed with a clear
//! epoch message instead of a misleading conflict.

use cortex_kernel::{CortexRuntime, LensInput};
use cortex_tests::support::run_with_cx;

fn seed_legacy_brain(path: &std::path::Path) {
    let mut conn = rusqlite::Connection::open(path).expect("open");
    cortex_kernel::db::configure(&conn).expect("configure");
    cortex_kernel::db::initialize_schema(&conn).expect("schema");
    // Build the faithful v24 shape: withhold 025, run everything else so the
    // compiled tables exist under their real DDL, seed legacy bytes, then
    // release 025 for the production open below.
    cortex_kernel::db::ensure_schema_migrations_table(&conn).expect("ledger table");
    conn.execute(
        "INSERT INTO schema_migrations (version, name) VALUES ('025_blake3_digests', 'blake3_digests')",
        [],
    )
    .expect("withhold 025");
    cortex_kernel::db::run_pending_migrations(&mut conn);
    conn.execute(
        "INSERT OR IGNORE INTO scopes (scope_id, owner_id, kind, descriptor) VALUES ('s', 't', 'test', '{}')",
        [],
    )
    .expect("scope row");

    // A warm memory that must stay recallable after the upgrade.
    conn.execute(
        "INSERT INTO decisions (decision, type, source_agent, status) VALUES ('UPG-2 warm memory migrates across the hash cutover', 'decision', 't', 'active')",
        [],
    )
    .unwrap();
    // A cold memory sealed under the legacy SipHash epoch.
    conn.execute(
        "INSERT INTO decisions (decision, type, source_agent, status) VALUES ('UPG-1 cold memory sealed before the cutover', 'decision', 't', 'active')",
        [],
    )
    .unwrap();
    let cold_id = conn.last_insert_rowid();
    cortex_kernel::db::cold::move_to_cold(&conn, "decision", cold_id).unwrap();
    conn.execute("UPDATE cold_sources SET digest = '0123456789abcdef'", [])
        .unwrap();

    // Pre-cutover host-capture markers under the old column name.
    conn.execute_batch(
        "CREATE TABLE host_capture_metadata (principal TEXT NOT NULL, grant_key TEXT NOT NULL, generation TEXT NOT NULL, byte_offset INTEGER NOT NULL, kind TEXT NOT NULL, byte_length INTEGER NOT NULL, sha256 TEXT NOT NULL, PRIMARY KEY(principal,grant_key,generation,byte_offset));",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO host_capture_metadata VALUES ('p','g','file/1:9:9:9:9:deadbeef',0,'tool',4,'deadbeef')",
        [],
    )
    .unwrap();

    // Legacy compiled-plan cache rows keyed by `compiled:{16hex}`.
    conn.execute(
        "INSERT INTO compiled_reads (compiled_id, recipe_id, operator_versions, scope_id, principal_id, brain_epoch, policy_epoch, parameters_json, environment_ref, through_sequence, result_json) VALUES ('compiled:0123456789abcdef', 'r', 'v', 's', 'p', 'b', 'e', '{}', 'env', 0, '{}')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO compiled_guards (compiled_id, scope_id, guard_key, expected_generation, kind) VALUES ('compiled:0123456789abcdef', 's', 'g', 0, 'positive')",
        [],
    )
    .unwrap();

    // Legacy identity-capsule cache row: 16-hex FNV seal, never re-readable.
    conn.execute(
        "INSERT INTO context_cache (cache_key, content_hash, compressed, tokens) VALUES ('identity_capsule', 'fedcba9876543210', 'old', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "DELETE FROM schema_migrations WHERE version = '025_blake3_digests'",
        [],
    )
    .expect("release 025");
}

#[test]
fn legacy_brain_upgrades_cleanly_and_memories_stay_recallable() {
    run_with_cx(|cx| async move {
        let dir = tempfile::Builder::new()
            .prefix("cortex-epoch-upgrade-")
            .tempdir()
            .expect("tempdir");
        let db = dir.path().join("cortex.db");
        seed_legacy_brain(&db);

        // Production open runs the pending migrations, including 025.
        let rt = CortexRuntime::open_db(&db).expect("open legacy brain");
        {
            let conn = rt.state().db.lock(&cx).await.expect("db lock");
            let pending = cortex_kernel::db::pending_migration_versions(&conn).expect("pending");
            assert!(pending.is_empty(), "upgrade left pending: {pending:?}");
            let version =
                cortex_kernel::db::current_schema_user_version(&conn).expect("schema version");
            assert_eq!(version, 25);

            // Marker bytes preserved under the new name, never reinterpreted.
            let kept: String = conn
                .query_row("SELECT digest FROM host_capture_metadata", [], |r| r.get(0))
                .expect("marker");
            assert_eq!(kept, "deadbeef");
            // Legacy compiled cache purged; legacy history kept.
            let reads: i64 = conn
                .query_row("SELECT COUNT(*) FROM compiled_reads", [], |r| r.get(0))
                .unwrap();
            let guards: i64 = conn
                .query_row("SELECT COUNT(*) FROM compiled_guards", [], |r| r.get(0))
                .unwrap();
            assert_eq!((reads, guards), (0, 0), "legacy plan cache purged");
            // Warm memory row byte-identical.
            let warm: String = conn
                .query_row(
                    "SELECT decision FROM decisions WHERE decision LIKE 'UPG-2%'",
                    [],
                    |r| r.get(0),
                )
                .expect("warm memory");
            assert!(warm.contains("warm memory migrates"));
            // Legacy capsule row misses under the new seal, then self-heals.
            let miss = cortex_kernel::compiler::cache_get(
                &conn,
                "identity_capsule",
                &cortex_kernel::traces::content_hash("fresh-bytes"),
            );
            assert!(miss.is_none(), "legacy seals never verify");
            cortex_kernel::compiler::cache_set(
                &conn,
                "identity_capsule",
                &cortex_kernel::traces::content_hash("fresh-bytes"),
                "new",
                2,
            );
            assert!(
                cortex_kernel::compiler::cache_get(
                    &conn,
                    "identity_capsule",
                    &cortex_kernel::traces::content_hash("fresh-bytes"),
                )
                .is_some(),
                "cache self-heals on next write"
            );
        }

        // Cold memory: text intact, seal unverifiable once, then re-sealed.
        let cold_id: i64 = {
            let conn = rt.state().db.lock(&cx).await.expect("db lock");
            conn.query_row(
                "SELECT CAST(address AS INTEGER) FROM cold_sources WHERE namespace = 'decision'",
                [],
                |r| r.get(0),
            )
            .expect("cold address")
        };
        let (text, _ctx, intact) = {
            let conn = rt.state().db.lock(&cx).await.expect("db lock");
            cortex_kernel::db::cold::hydrate(&conn, "decision", cold_id)
                .expect("hydrate")
                .expect("cold row")
        };
        assert_eq!(text, "UPG-1 cold memory sealed before the cutover");
        assert!(!intact, "legacy seals cannot attest integrity");
        let (_text, _ctx, intact) = {
            let conn = rt.state().db.lock(&cx).await.expect("db lock");
            cortex_kernel::db::cold::hydrate(&conn, "decision", cold_id)
                .expect("hydrate")
                .expect("cold row")
        };
        assert!(intact, "row re-sealed to BLAKE3");

        // The migrated brain recalls the old memory and accepts new writes.
        let view = rt
            .lens(
                &cx,
                LensInput {
                    query: "UPG-2".into(),
                    agent: "t".into(),
                    ..Default::default()
                },
            )
            .await
            .expect("lens");
        assert!(
            view.to_string().contains("UPG-2"),
            "old memory must stay recallable: {view}"
        );
        let out = rt
            .deposit(
                &cx,
                "upg-new-1",
                "UPG-3 written after the upgrade",
                "t",
                None,
            )
            .await
            .expect("post-upgrade deposit");
        assert!(out.receipt.is_locally_durable(), "{:?}", out.receipt);

        // A key sealed before the upgrade fails closed with the epoch
        // message, never a misleading "different payload" claim, and writes
        // no duplicate.
        rt.deposit_with_key(
            &cx,
            "upg-k1",
            Some("upg/legacy"),
            "UPG-4 epoch key",
            "t",
            None,
        )
        .await
        .expect("seal key");
        {
            let conn = rt.state().db.lock(&cx).await.expect("db lock");
            conn.execute(
                "UPDATE operation_ledger SET canonical_hash = '0123456789abcdef' WHERE idempotency_key = 'upg/legacy'",
                [],
            )
            .expect("age the seal");
        }
        let conflict = rt
            .deposit_with_key(
                &cx,
                "upg-k2",
                Some("upg/legacy"),
                "UPG-4 epoch key",
                "t",
                None,
            )
            .await;
        assert!(
            matches!(conflict, Err(cortex_kernel::CortexError::Conflict(ref m)) if m.contains("previous hash epoch")),
            "legacy keys fail closed with the epoch message: {conflict:?}"
        );
        let count: i64 = {
            let conn = rt.state().db.lock(&cx).await.expect("db lock");
            conn.query_row(
                "SELECT COUNT(*) FROM decisions WHERE decision LIKE 'UPG-4%'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(count, 1, "no duplicate row was written");
    });
}

#[test]
fn served_dedup_fingerprint_is_stable_blake3() {
    // Pins the in-memory served-set fingerprint to BLAKE3 (first 4 bytes,
    // little-endian u32). The expected value comes from the vendored
    // ZeroStack fixture blob `alpha` ("alpha\nbeta\ngamma\n" hashes to
    // ed8b7a77…), independent of this implementation.
    assert_eq!(
        cortex_kernel::handlers::recall::hash_content("alpha\nbeta\ngamma\n"),
        0x777a_8bed_u32
    );
    assert_eq!(
        cortex_kernel::handlers::recall::hash_content("abc"),
        cortex_kernel::handlers::recall::hash_content("abc")
    );
    assert_ne!(
        cortex_kernel::handlers::recall::hash_content("abc"),
        cortex_kernel::handlers::recall::hash_content("abd")
    );
}
