//! Wave 8: backend certification boundary. The reference SQLite provider is
//! certified for every level the in-process suite can exercise; the memory
//! oracle passes `core` only; integrity descriptors rotate side by side and
//! degrade to `unverified`, never to a silent pass; the address overlay
//! rebases without touching a record; a shadowed collector reports
//! disagreements instead of asserting equivalence.

use cortex_kernel::db::addresses;
use cortex_kernel::store_spi::conformance::{
    digest_set, resolve_index, resolve_security, run_suite, verify_digest, verify_rotation,
    ConformanceLevel, DigestDescriptor, IndexResolution, IntegrityVerdict,
};
use cortex_kernel::store_spi::dispatch::{shadow_compare, StoreHandle};
use cortex_kernel::store_spi::memory::MemoryStore;
use cortex_kernel::store_spi::sqlite::SqliteStore;
use cortex_kernel::store_spi::{BrainStore, Durability, Op, WriteIntent, WriteTransaction};
use cortex_tests::support::open_file_db;

fn temp_db() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("cortex-conf-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("cortex.db");
    (dir, path)
}

#[test]
fn sqlite_reference_is_certified_for_every_level_it_claims() {
    let (_dir, path) = temp_db();
    let mut store = StoreHandle::Sqlite(SqliteStore::new(open_file_db(&path)).expect("store"));
    let manifest = store.manifest();
    let report = run_suite(&mut store, &manifest);
    assert!(report.failures().is_empty(), "{:#?}", report.failures());
    let certified = report.certified();
    assert!(
        certified.contains(&ConformanceLevel::Core)
            && certified.contains(&ConformanceLevel::DurableLocal)
            && certified.contains(&ConformanceLevel::TamperEvident),
        "{certified:?}"
    );
    // ConcurrentLocal is claimed but needs a multi-process harness: reported, not certified.
    assert!(
        report
            .unverified_claims
            .contains(&ConformanceLevel::ConcurrentLocal),
        "{:?}",
        report.unverified_claims
    );
    assert!(report.refuted().is_empty());
}

#[test]
fn memory_oracle_passes_core_and_nothing_else() {
    let mut store = StoreHandle::Memory(MemoryStore::new());
    let manifest = store.manifest();
    let report = run_suite(&mut store, &manifest);
    assert_eq!(
        report.certified().into_iter().collect::<Vec<_>>(),
        vec![ConformanceLevel::Core],
        "{:#?}",
        report.laws
    );
    // It refuses to claim durability or tamper evidence, and the suite agrees.
    assert!(
        !report.verified.contains(&ConformanceLevel::TamperEvident),
        "no descriptor, no tamper evidence"
    );
    // A dishonest manifest is refuted, not accepted.
    let mut dishonest = manifest.clone();
    dishonest.claims.insert(ConformanceLevel::TamperEvident);
    let mut store = StoreHandle::Memory(MemoryStore::new());
    let report = run_suite(&mut store, &dishonest);
    assert!(
        report.refuted().contains(&ConformanceLevel::TamperEvident),
        "{:?}",
        report.laws
    );
}

#[test]
fn integrity_descriptors_rotate_side_by_side_and_unknown_means_unverified() {
    let sha = DigestDescriptor::sha256("cortex/record");
    let future = DigestDescriptor {
        algorithm: "blake3-xof".into(),
        domain: "cortex/record".into(),
        canonicalization: "raw".into(),
    };
    let bytes = b"  ledger commit is the retry fence  ";
    let set = digest_set(&[sha.clone(), future.clone()], bytes);
    assert!(set[&sha.key()].is_some());
    assert_eq!(
        set[&future.key()],
        None,
        "unsupported algorithm yields no digest, never a fake one"
    );
    let good = sha.digest(bytes).unwrap();
    assert_eq!(
        verify_digest(&sha, bytes, &good),
        IntegrityVerdict::Verified
    );
    assert!(matches!(
        verify_digest(&future, bytes, "abcd"),
        IntegrityVerdict::Unverified { .. }
    ));
    // Rotation: an old unsupported digest beside a supported one verifies; a supported mismatch is loud.
    assert_eq!(
        verify_rotation(
            &[(future.clone(), "abcd".into()), (sha.clone(), good.clone())],
            bytes
        ),
        IntegrityVerdict::Verified
    );
    assert!(matches!(
        verify_rotation(&[(future.clone(), "abcd".into())], bytes),
        IntegrityVerdict::Unverified { .. }
    ));
    assert!(matches!(
        verify_rotation(&[(sha.clone(), good.clone())], b"tampered"),
        IntegrityVerdict::Mismatch { .. }
    ));
    // Canonicalization is part of identity: trimmed input verifies under the trim descriptor only.
    assert_eq!(
        verify_digest(&sha, b"ledger commit is the retry fence", &good),
        IntegrityVerdict::Verified
    );
    let raw = DigestDescriptor {
        canonicalization: "raw".into(),
        ..sha.clone()
    };
    assert_ne!(
        raw.digest(bytes),
        raw.digest(b"ledger commit is the retry fence")
    );
    // Domain separation: same bytes, different domain, different digest.
    assert_ne!(
        DigestDescriptor::sha256("cortex/other").digest(bytes),
        Some(good)
    );
}

#[test]
fn unknown_optional_index_falls_back_to_exact_and_unknown_required_scheme_blocks() {
    let sqlite = StoreHandle::Sqlite(SqliteStore::new(open_file_db(&temp_db().1)).expect("store"));
    let manifest = sqlite.manifest();
    assert_eq!(
        resolve_index(&manifest, "fts"),
        IndexResolution::Accelerated("fts".into())
    );
    assert_eq!(
        resolve_index(&manifest, "hnsw"),
        IndexResolution::ExactFallback {
            requested: "hnsw".into()
        }
    );
    assert!(resolve_security(&manifest, &["local-token".into()]).is_ok());
    let blocked = resolve_security(&manifest, &["local-token".into(), "mtls-attested".into()]);
    assert!(blocked.unwrap_err().to_string().contains("mtls-attested"));
}

#[test]
fn address_overlay_rebases_without_touching_records_and_measures_short_address_ambiguity() {
    let (_dir, path) = temp_db();
    let mut store = SqliteStore::new(open_file_db(&path)).expect("store");
    let mut tx = store
        .begin_write(WriteIntent {
            request_id: "addr".into(),
            idempotency_key: None,
            principal: "t".into(),
            expected_heads: Vec::new(),
        })
        .unwrap();
    tx.apply(Op::InsertDecision {
        local_name: "a".into(),
        text: "address overlay law".into(),
        context: None,
        agent: "t".into(),
        owner_id: None,
    })
    .unwrap();
    let receipt = tx.commit(Durability::ProcessCrash).unwrap();
    let conn = store.connection();
    // Legacy rows become authoritative records; addresses reference records, never legacy ids.
    cortex_kernel::db::records::import_legacy(conn).unwrap();
    let legacy_id = receipt.entries["a"].value.parse::<i64>().unwrap();
    let id = cortex_kernel::db::records::record_for_legacy(conn, "decision", legacy_id)
        .unwrap()
        .expect("record for legacy decision");
    conn.execute("INSERT INTO records (record_id, kind, scope_id, retention, created_sequence) SELECT 'rec-other', kind, scope_id, retention, created_sequence FROM records WHERE record_id = ?1", [&id]).unwrap();
    let sha = DigestDescriptor::sha256("cortex/record");
    let d1 = sha.digest(b"address overlay law").unwrap();
    let d2 = format!("{}ffff", &d1[..6]);
    assert_eq!(
        addresses::assign(conn, &id, "segments", "sha256-full", &d1).unwrap(),
        1
    );
    addresses::assign(conn, "rec-other", "segments", "sha256-full", &d2).unwrap();
    // A 6-char prefix is ambiguous on this brain; the floor is data-defined.
    assert!(
        matches!(addresses::resolve_short(conn, "sha256-full", "segments", &d1[..6]).unwrap(), addresses::ShortAddress::Ambiguous(ref ids) if ids.len() == 2)
    );
    assert_eq!(
        addresses::resolve_short(conn, "sha256-full", "segments", &d1[..8]).unwrap(),
        addresses::ShortAddress::Unique(id.clone())
    );
    assert_eq!(
        addresses::minimum_unique_prefix(conn, "sha256-full", "segments").unwrap(),
        7
    );
    assert_eq!(
        addresses::resolve_short(conn, "sha256-full", "segments", "zzz").unwrap(),
        addresses::ShortAddress::Unknown
    );
    // Rebase to a different scheme: locators change, the record row does not.
    let before: (String, i64) = conn
        .query_row(
            "SELECT decision, COALESCE(version_id,0) FROM decisions WHERE id = ?1",
            [receipt.entries["a"].value.parse::<i64>().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        addresses::rebase(
            conn,
            "segments",
            "sha256-full",
            "seg-v2",
            |id, old| format!("v2/{id}/{}", &old[..8])
        )
        .unwrap(),
        2
    );
    let bound = addresses::resolve(conn, &id, "segments").unwrap();
    assert_eq!(bound.len(), 1, "{bound:?}");
    assert_eq!(bound[0].0, "seg-v2");
    assert!(bound[0].1.starts_with(&format!("v2/{id}/")), "{bound:?}");
    assert_eq!(
        addresses::record_for(conn, "seg-v2", "segments", &bound[0].1)
            .unwrap()
            .as_deref(),
        Some(id.as_str())
    );
    assert_eq!(
        addresses::record_for(conn, "sha256-full", "segments", &d1).unwrap(),
        None,
        "old scheme dropped"
    );
    let after: (String, i64) = conn
        .query_row(
            "SELECT decision, COALESCE(version_id,0) FROM decisions WHERE id = ?1",
            [receipt.entries["a"].value.parse::<i64>().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        before, after,
        "changing addresses never mutates record references"
    );
}

#[test]
fn shadowed_collector_reports_disagreements_instead_of_claiming_equivalence() {
    let (_dir, path) = temp_db();
    let mut primary = SqliteStore::new(open_file_db(&path)).expect("store");
    let mut shadow = MemoryStore::new();
    let corpus = [
        "retry after ledger commit is forbidden",
        "ledger reads are cached per epoch",
        "gateway timeout budget is 1800ms",
    ];
    for (i, text) in corpus.iter().enumerate() {
        let mut tx = primary
            .begin_write(WriteIntent {
                request_id: format!("p{i}"),
                idempotency_key: None,
                principal: "t".into(),
                expected_heads: Vec::new(),
            })
            .unwrap();
        tx.apply(Op::InsertDecision {
            local_name: "d".into(),
            text: text.to_string(),
            context: None,
            agent: "t".into(),
            owner_id: None,
        })
        .unwrap();
        tx.commit(Durability::ProcessCrash).unwrap();
        let mut tx = shadow
            .begin_write(WriteIntent {
                request_id: format!("s{i}"),
                idempotency_key: None,
                principal: "t".into(),
                expected_heads: Vec::new(),
            })
            .unwrap();
        tx.apply(Op::InsertDecision {
            local_name: "d".into(),
            text: text.to_string(),
            context: None,
            agent: "t".into(),
            owner_id: None,
        })
        .unwrap();
        tx.commit(Durability::ProcessCrash).unwrap();
    }
    let queries = vec![
        vec!["ledger".to_string()],
        vec!["gateway".to_string(), "1800ms".to_string()],
        vec!["nonexistent".to_string()],
    ];
    let report = shadow_compare(&primary, &shadow, &queries).unwrap();
    assert_eq!(report.agreed, 3, "{report:?}");
    // Divergence is counted, not hidden.
    let mut tx = shadow
        .begin_write(WriteIntent {
            request_id: "extra".into(),
            idempotency_key: None,
            principal: "t".into(),
            expected_heads: Vec::new(),
        })
        .unwrap();
    tx.apply(Op::InsertDecision {
        local_name: "d".into(),
        text: "ledger extra only in shadow".into(),
        context: None,
        agent: "t".into(),
        owner_id: None,
    })
    .unwrap();
    tx.commit(Durability::ProcessCrash).unwrap();
    let report = shadow_compare(&primary, &shadow, &queries).unwrap();
    assert_eq!(report.agreed, 2);
    assert_eq!(report.only_in_shadow.len(), 1, "{report:?}");
    assert!(report.only_in_primary.is_empty());
}
