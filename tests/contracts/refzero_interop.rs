//! RefZero interop contracts: BLAKE3 importer, the shared `z://blob/`
//! fixture vectors, cross-store envelope transfer through the kernel
//! wire-in, migration 025, and cold dual-read. The fixtures file is vendored
//! verbatim from the reference; every vector here must pass there too.

use cortex_kernel::refzero::zeroref::{
    LineEndPolicy, SpanRef, ZeroFragment, ZeroRef, ZeroRefErrorClass,
};
use cortex_kernel::refzero::{
    Algorithm, ByteSpan, FrozenFragment, ImportKey, ImportRequest, Producer, RecallEnvelope, Store,
    digest, import,
};
use serde_json::{Value, json};

fn open_store(prefix: &str) -> (tempfile::TempDir, Store) {
    let dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("tempdir");
    let store = Store::open(&dir.path().join("refzero.sqlite")).expect("open store");
    (dir, store)
}

fn hex_decode(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0, "odd hex length");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex byte"))
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    digest::hex_lower(bytes)
}

// --- Importer (mirrors ZeroStack tests/refzero/importer.rs) ---

fn key(producer: Producer, bytes: &[u8]) -> ImportKey {
    ImportKey {
        producer,
        algorithm: Algorithm::Blake3,
        locator: blake3_digest_hex(bytes),
    }
}

fn blake3_digest_hex(bytes: &[u8]) -> String {
    digest::digest_hex(bytes)
}

#[test]
fn importer_two_payloads_and_reimport() {
    let (_dir, store) = open_store("cortex-refzero-imp-");
    let blake_bytes = b"kernel-payload-aaaa";
    let other_bytes = b"portable-payload-bbbb";
    let a_key = key(Producer::KernelHandle, blake_bytes);
    let b_key = key(Producer::PortableZeroRef, other_bytes);
    assert_eq!(a_key.locator.len(), 64);
    let a = import(
        &store,
        ImportRequest {
            key: a_key.clone(),
            bytes: blake_bytes.to_vec(),
            fragment: None,
        },
    )
    .unwrap();
    let b = import(
        &store,
        ImportRequest {
            key: b_key,
            bytes: other_bytes.to_vec(),
            fragment: None,
        },
    )
    .unwrap();
    assert_ne!(a.oid, b.oid);
    assert_eq!(store.payload(a.oid).unwrap(), blake_bytes);
    assert_eq!(store.payload(b.oid).unwrap(), other_bytes);
    let again = import(
        &store,
        ImportRequest {
            key: a_key,
            bytes: blake_bytes.to_vec(),
            fragment: None,
        },
    )
    .unwrap();
    assert_eq!(again.oid, a.oid);
}

#[test]
fn importer_conflict_same_locator_different_bytes() {
    let (_dir, store) = open_store("cortex-refzero-imp-");
    let bytes = b"first-payload-ok!!";
    let k = key(Producer::KernelHandle, bytes);
    import(
        &store,
        ImportRequest {
            key: k.clone(),
            bytes: bytes.to_vec(),
            fragment: None,
        },
    )
    .unwrap();
    let err = import(
        &store,
        ImportRequest {
            key: k,
            bytes: b"other-payload-nope".to_vec(),
            fragment: None,
        },
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            cortex_kernel::refzero::Error::DigestMismatch { .. }
                | cortex_kernel::refzero::Error::ImportConflict
        ),
        "{err:?}"
    );
}

#[test]
fn importer_fragments_on_frozen_bytes() {
    let (_dir, store) = open_store("cortex-refzero-imp-");
    let bytes = b"aaa\nbbb\nccc\n";
    let imported = import(
        &store,
        ImportRequest {
            key: key(Producer::PortableZeroRef, bytes),
            bytes: bytes.to_vec(),
            fragment: Some(FrozenFragment::Lines { start: 2, end: 2 }),
        },
    )
    .unwrap();
    let slice = &bytes[imported.selection.start as usize..imported.selection.end as usize];
    assert_eq!(slice, b"bbb\n");
    let byte = import(
        &store,
        ImportRequest {
            key: key(Producer::KernelHandle, bytes),
            bytes: bytes.to_vec(),
            fragment: Some(FrozenFragment::Bytes { start: 0, end: 3 }),
        },
    )
    .unwrap();
    assert_eq!(
        &bytes[byte.selection.start as usize..byte.selection.end as usize],
        b"aaa"
    );
}

#[test]
fn import_locator_accepts_bare_and_uri_spellings() {
    let hex = "ab".repeat(32);
    assert_eq!(ImportKey::parse_locator(&hex).unwrap(), hex);
    assert_eq!(
        ImportKey::parse_locator(&format!("z://blob/{hex}")).unwrap(),
        hex
    );
    assert!(ImportKey::parse_locator("z://blob/short").is_err());
    assert!(ImportKey::parse_locator("not-hex-at-all").is_err());
    assert!(ImportKey::parse_locator(&"AB".repeat(32)).is_err());
}

// --- Shared z://blob/ fixture vectors ---

fn fixtures() -> Value {
    let path = format!(
        "{}/fixtures/zeroref-fixtures.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(&path).expect("read zeroref fixtures");
    serde_json::from_str(&raw).expect("parse zeroref fixtures")
}

fn blob_bytes(fx: &Value, name: &str) -> Vec<u8> {
    let blob = &fx["blobs"][name];
    let bytes = hex_decode(blob["bytes_hex"].as_str().expect("bytes_hex"));
    assert_eq!(
        digest::digest_hex(&bytes),
        blob["blake3"].as_str().expect("blake3"),
        "fixture blob {name} must hash to its pinned BLAKE3"
    );
    bytes
}

fn error_class(name: &str) -> ZeroRefErrorClass {
    match name {
        "malformed" => ZeroRefErrorClass::Malformed,
        "unsupported" => ZeroRefErrorClass::Unsupported,
        "range_out_of_bounds" => ZeroRefErrorClass::RangeOutOfBounds,
        "not_utf8" => ZeroRefErrorClass::NotUtf8,
        "missing" => ZeroRefErrorClass::Missing,
        "io" => ZeroRefErrorClass::Io,
        "digest_mismatch" => ZeroRefErrorClass::DigestMismatch,
        "policy_denied" => ZeroRefErrorClass::PolicyDenied,
        other => panic!("unknown fixture error class {other}"),
    }
}

#[test]
fn zeroref_error_classes_match_fixtures_verbatim() {
    let fx = fixtures();
    let expected: Vec<&str> = fx["error_classes"]
        .as_array()
        .expect("error_classes")
        .iter()
        .map(|v| v.as_str().expect("class str"))
        .collect();
    let actual: Vec<&str> = ZeroRefErrorClass::ALL.iter().map(|c| c.as_str()).collect();
    assert_eq!(actual, expected);
    assert_eq!(cortex_kernel::refzero::zeroref::HASH_ALGORITHM, "blake3");
    assert_eq!(cortex_kernel::refzero::zeroref::HASH_CASE, "lower");
}

#[test]
fn zeroref_parse_and_select_vectors() {
    let fx = fixtures();
    for vector in fx["vectors"].as_array().expect("vectors") {
        let name = vector["name"].as_str().expect("name");
        let input = vector["input"].as_str().expect("input");
        match vector["parse"].as_str().expect("parse") {
            "ok" => {
                let parsed =
                    ZeroRef::parse(input).unwrap_or_else(|e| panic!("{name}: parse failed: {e}"));
                assert_eq!(
                    parsed.to_string(),
                    vector["canonical"].as_str().expect("canonical"),
                    "{name}"
                );
                let bytes = blob_bytes(&fx, vector["blob"].as_str().expect("blob"));
                let strict =
                    vector.get("line_end_policy").and_then(Value::as_str) == Some("strict");
                let policy = if strict {
                    LineEndPolicy::Strict
                } else {
                    LineEndPolicy::ClampEnd
                };
                let selected = parsed.verify_and_select_with_policy(&bytes, policy);
                if let Some(expected) = vector.get("selection_error").and_then(Value::as_str) {
                    match selected {
                        Ok(_) => panic!("{name}: expected {expected}, selected ok"),
                        Err(err) => assert_eq!(err.class, error_class(expected), "{name}"),
                    }
                } else {
                    let selected =
                        selected.unwrap_or_else(|e| panic!("{name}: selection failed: {e}"));
                    assert_eq!(
                        hex_encode(selected),
                        vector["selected_hex"].as_str().expect("selected_hex"),
                        "{name}"
                    );
                }
            }
            "error" => match ZeroRef::parse(input) {
                Ok(parsed) => panic!("{name}: expected parse error, got {parsed:?}"),
                Err(err) => assert_eq!(
                    err.class,
                    error_class(vector["error_class"].as_str().expect("error_class")),
                    "{name}"
                ),
            },
            other => panic!("{name}: unknown parse expectation {other}"),
        }
    }
}

#[test]
fn zeroref_span_vectors_and_tamper_cases() {
    let fx = fixtures();
    for vector in fx["span_vectors"].as_array().expect("span_vectors") {
        let name = vector["name"].as_str().expect("name");
        let bytes = blob_bytes(&fx, vector["blob"].as_str().expect("blob"));
        let frag = &vector["fragment"];
        let fragment = match frag["kind"].as_str().expect("kind") {
            "bytes" => ZeroFragment::Bytes {
                start: frag["start"].as_u64().expect("start"),
                end: frag["end"].as_u64().expect("end"),
            },
            "lines" => ZeroFragment::Lines {
                start: frag["start"].as_u64().expect("start"),
                end: frag["end"].as_u64().expect("end"),
            },
            other => panic!("{name}: unknown fragment kind {other}"),
        };
        let (span, selected) = SpanRef::from_fragment(&bytes, &fragment, name).expect("span");
        assert_eq!(
            span.byte_start,
            vector["byte_start"].as_u64().expect("byte_start"),
            "{name}"
        );
        assert_eq!(
            span.byte_len,
            vector["byte_len"].as_u64().expect("byte_len"),
            "{name}"
        );
        assert_eq!(
            hex_encode(selected),
            vector["selected_hex"].as_str().expect("selected_hex"),
            "{name}"
        );
        assert_eq!(
            hex_encode(&span.span_digest),
            vector["span_digest"].as_str().expect("span_digest"),
            "{name}"
        );
        span.verify_span(selected).expect("verify_span");
    }
    // Tamper cases all derive from valid_span, with the exact classes the
    // fixtures pin.
    use cortex_kernel::refzero::zeroref::SpanRefError;
    let bytes = blob_bytes(&fx, "alpha");
    let (base, selected) = SpanRef::from_fragment(
        &bytes,
        &ZeroFragment::Bytes { start: 0, end: 5 },
        "valid_span",
    )
    .expect("base span");
    assert_eq!(selected, b"alpha");
    let mut tampered = selected.to_vec();
    tampered[0] ^= 0xff;
    assert_eq!(
        base.verify_span(&tampered),
        Err(SpanRefError::SpanDigestMismatch),
        "tampered_payload"
    );
    let ranged = SpanRef {
        byte_len: bytes.len() as u64 + 1,
        ..base.clone()
    };
    assert_eq!(
        ranged.verify_and_select(&bytes),
        Err(SpanRefError::RangeOutOfBounds),
        "tampered_range"
    );
    let mut digest_tampered = base.clone();
    digest_tampered.span_digest[0] ^= 0xff;
    assert_eq!(
        digest_tampered.verify_span(selected),
        Err(SpanRefError::SpanDigestMismatch),
        "tampered_digest"
    );
}

// --- Cross-store envelope transfer through the kernel wire-in ---

#[test]
fn envelope_transfer_mints_fresh_loc_with_adopted_oid() {
    let (_a_dir, a) = open_store("cortex-refzero-xa-");
    let (_b_dir, b) = open_store("cortex-refzero-xb-");
    let session_a = a.open_session().unwrap();
    let session_b = b.open_session().unwrap();
    let bytes = b"shared recall bytes\nline two\n";
    let interned = a.intern(bytes).unwrap();
    let span = ByteSpan { start: 0, end: 19 };
    let no_a = a
        .expose(session_a, interned.oid, span, "file", Some("note.md"))
        .unwrap();
    assert_eq!(no_a, 1);

    let env = cortex_kernel::refzero::export_envelope(&a, session_a, no_a).unwrap();
    assert_eq!(env.oid().unwrap(), interned.oid);
    assert_eq!(env.span(), span);
    // Exact cross-product JSON shape, through a string round-trip.
    let json_text = serde_json::to_string(&env).unwrap();
    let back: RecallEnvelope = serde_json::from_str(&json_text).unwrap();
    assert_eq!(back, env);

    let no_b = cortex_kernel::refzero::import_envelope(&b, session_b, &back, bytes).unwrap();
    assert_eq!(no_b, 1, "receiving session mints its own fresh loc");
    let loc_b = b.resolve(session_b, no_b).unwrap();
    assert_eq!(loc_b.oid, interned.oid, "sender oid is adopted");
    assert_eq!(loc_b.span, span);
    assert_eq!(b.payload(loc_b.oid).unwrap(), bytes);
    assert!(loc_b.granted_by.is_none(), "import carries no Edit grant");

    // A span outside the adopted object fails closed, consuming no loc.
    let bad = RecallEnvelope {
        start: 0,
        end: bytes.len() as u64 + 1,
        ..back.clone()
    };
    assert!(cortex_kernel::refzero::import_envelope(&b, session_b, &bad, bytes).is_err());
    let next = b
        .expose(
            session_b,
            loc_b.oid,
            ByteSpan::whole(bytes.len() as u64),
            "file",
            None,
        )
        .unwrap();
    assert_eq!(next, 2);
}

// --- Kernel identity-slot hygiene ---

#[test]
fn kernel_identity_slots_refuse_digests() {
    let hex = "ee".repeat(32);
    let uri = format!("z://blob/{hex}");
    let value = json!({"target": uri, "loc": "@7"});
    let err = cortex_kernel::refzero::require_no_digest_in_identity(&value).unwrap_err();
    assert!(err.contains("not_a_loc"), "{err}");
    assert!(err.contains("/target"), "{err}");
    assert!(!err.contains(&hex), "must not echo the digest: {err}");
    assert!(cortex_kernel::refzero::require_no_digest_in_identity(&json!({"loc": "@7"})).is_ok());

    assert_eq!(cortex_kernel::refzero::resolve_loc_no("@12").unwrap(), 12);
    assert_eq!(
        cortex_kernel::refzero::resolve_loc_no(&uri).unwrap_err(),
        "not_a_loc"
    );
    assert_eq!(
        cortex_kernel::refzero::resolve_loc_no(&hex).unwrap_err(),
        "not_a_loc"
    );
    let bad = cortex_kernel::refzero::resolve_loc_no("@readme.md").unwrap_err();
    assert!(bad.contains("not_a_loc"), "{bad}");
}

#[test]
fn kernel_blob_ref_round_trip() {
    let bytes = b"alpha\nbeta\ngamma\n";
    let reference = cortex_kernel::refzero::blob_ref_for_bytes(bytes);
    assert_eq!(
        reference, "z://blob/ed8b7a779a4acecd8265c7b2b70a2e74e7c63b9d39e5050601c6c80af194e5c0",
        "must match the shared fixture blob alpha"
    );
    let selected =
        cortex_kernel::refzero::select_blob_ref(&format!("{reference}#B6-10"), bytes).unwrap();
    assert_eq!(selected, b"beta");
    let err = cortex_kernel::refzero::select_blob_ref(&format!("{reference}#B6-10"), b"other")
        .unwrap_err();
    assert!(err.contains("digest_mismatch"), "{err}");
}

#[test]
fn intake_hooks_cache_one_sidecar_handle_per_path() {
    // The cache is process-wide and other tests in this binary also intern,
    // so assert relative deltas on fresh tempdir paths, never absolutes.
    let before = cortex_kernel::refzero::__test_cache_len();
    let dir = tempfile::Builder::new()
        .prefix("cortex-refzero-cache-")
        .tempdir()
        .expect("tempdir");
    let db = dir.path().join("cortex.db");
    let first =
        cortex_kernel::refzero::intern_store_bytes_for_db(&db, b"cached-bytes").expect("intern");
    assert_eq!(cortex_kernel::refzero::__test_cache_len(), before + 1);
    let second =
        cortex_kernel::refzero::intern_store_bytes_for_db(&db, b"cached-bytes").expect("intern");
    assert_eq!(first, second, "same bytes dedup to one object");
    assert_eq!(
        cortex_kernel::refzero::__test_cache_len(),
        before + 1,
        "second hook reuses the cached handle, no reopen"
    );
    let other_db = dir.path().join("other.db");
    cortex_kernel::refzero::intern_store_bytes_for_db(&other_db, b"cached-bytes").expect("intern");
    assert_eq!(
        cortex_kernel::refzero::__test_cache_len(),
        before + 2,
        "a distinct sidecar path gets its own handle"
    );
    // The cached bytes are really there under a fresh handle.
    let sidecar = cortex_kernel::refzero::sidecar_path_for_db(&db).expect("sidecar");
    let fresh = cortex_kernel::refzero::Store::open(&sidecar).expect("reopen");
    assert_eq!(fresh.payload(first.oid).expect("payload"), b"cached-bytes");
}

// --- Migration 025: sha256 column rename ---

#[test]
fn migration_025_renames_sha256_marker_column() {
    use cortex_kernel::db;
    use rusqlite::Connection;

    let dir = tempfile::Builder::new()
        .prefix("cortex-mig025-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("cortex.db");
    let mut conn = Connection::open(&path).expect("open");
    db::configure(&conn).expect("configure");
    db::initialize_schema(&conn).expect("schema");
    // Pre-cutover host-capture table with the old column name.
    conn.execute_batch(
        "CREATE TABLE host_capture_metadata (principal TEXT NOT NULL, grant_key TEXT NOT NULL, generation TEXT NOT NULL, byte_offset INTEGER NOT NULL, kind TEXT NOT NULL, byte_length INTEGER NOT NULL, sha256 TEXT NOT NULL, PRIMARY KEY(principal,grant_key,generation,byte_offset));",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO host_capture_metadata VALUES ('p','g','gen',0,'tool',4,'deadbeef')",
        [],
    )
    .unwrap();
    let applied = db::run_pending_migrations(&mut conn);
    assert!(applied >= 1, "025 must apply");
    assert!(db::pending_migration_versions(&conn).unwrap().is_empty());
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('host_capture_metadata')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(cols.contains(&"digest".to_string()), "{cols:?}");
    assert!(!cols.contains(&"sha256".to_string()), "{cols:?}");
    let kept: String = conn
        .query_row("SELECT digest FROM host_capture_metadata", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        kept, "deadbeef",
        "legacy marker bytes preserved, never reinterpreted"
    );
}

// --- Cold dual-read across the hash epoch ---

#[test]
fn cold_new_seals_verify_and_legacy_reseals() {
    use cortex_kernel::db;
    use cortex_tests::support::open_file_db;

    let dir = tempfile::Builder::new()
        .prefix("cortex-cold-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("cortex.db");
    let conn = open_file_db(&path);
    conn.execute(
        "INSERT INTO decisions (decision, type, source_agent, status) VALUES ('cold bytes here', 'decision', 't', 'active')",
        [],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    db::cold::move_to_cold(&conn, "decision", id).unwrap();
    let (text, _ctx, intact) = db::cold::hydrate(&conn, "decision", id)
        .unwrap()
        .expect("row");
    assert_eq!(text, "cold bytes here");
    assert!(intact, "fresh BLAKE3 seals verify");

    // Simulate a pre-cutover SipHash-epoch seal: unverifiable by
    // construction, so the first read reports !intact and re-seals.
    conn.execute("UPDATE cold_sources SET digest = '0123456789abcdef'", [])
        .unwrap();
    let (text, _ctx, intact) = db::cold::hydrate(&conn, "decision", id)
        .unwrap()
        .expect("row");
    assert_eq!(text, "cold bytes here");
    assert!(!intact, "legacy seals cannot attest integrity");
    let (_text, _ctx, intact) = db::cold::hydrate(&conn, "decision", id)
        .unwrap()
        .expect("row");
    assert!(intact, "row re-sealed to BLAKE3 on the previous read");

    // A legacy seal with a mismatched length is not re-sealed: the next
    // read must not attest bytes that are not what was archived.
    conn.execute(
        "UPDATE cold_sources SET digest = '0123456789abcdef', byte_length = 9999",
        [],
    )
    .unwrap();
    let (_text, _ctx, intact) = db::cold::hydrate(&conn, "decision", id)
        .unwrap()
        .expect("row");
    assert!(!intact);
    let kept: String = conn
        .query_row("SELECT digest FROM cold_sources", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        kept, "0123456789abcdef",
        "length-mismatched rows are never re-sealed"
    );
}
