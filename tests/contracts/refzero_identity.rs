//! RefZero identity contracts: intern dedup, loc serials, mint table,
//! envelopes, and identity-slot hygiene. Mirrors ZeroStack
//! `tests/refzero/{store,mint_bind,formatter}.rs` against the vendored
//! `cortex-refzero` port (byte-identical semantics, rusqlite driver).

use cortex_kernel::refzero::{
    ByteSpan, Error, LocParseError, MAX_SAFE_INTEGER, RecallEnvelope, Session, Store, format_loc,
    identity_slot_hits, parse_loc,
};
use serde_json::json;

fn open_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::Builder::new()
        .prefix("cortex-refzero-")
        .tempdir()
        .expect("tempdir");
    let store = Store::open(&dir.path().join("refzero.sqlite")).expect("open store");
    (dir, store)
}

#[test]
fn intern_same_bytes_one_object() {
    let (_dir, store) = open_store();
    let a = store.intern(b"parser.rs\n").unwrap();
    let b = store.intern(b"parser.rs\n").unwrap();
    assert_eq!(a.oid, b.oid);
    assert_eq!(a.byte_len, 10);
    let other = store.intern(b"other.rs\n").unwrap();
    assert_ne!(a.oid, other.oid);
}

#[test]
fn first_loc_is_one() {
    let (_dir, store) = open_store();
    let interned = store.intern(b"x").unwrap();
    let session = store.open_session().unwrap();
    let no = store
        .expose(
            session,
            interned.oid,
            ByteSpan::whole(interned.byte_len),
            "file",
            None,
        )
        .unwrap();
    assert_eq!(no, 1);
    assert!(store.resolve(session, 0).is_err());
    // Exact row mapping, including NULL source path on the expose path.
    let loc = store.resolve(session, 1).unwrap();
    assert_eq!(loc.oid, interned.oid);
    assert_eq!(loc.span, ByteSpan::whole(1));
    assert_eq!(loc.role, "file");
    assert_eq!(loc.source_path, None);
    assert_eq!(loc.origin, "");
    assert_eq!(loc.rev, 0);
    assert_eq!(loc.granted_by.as_deref(), Some("read_seed"));
    assert_eq!(loc.kind, "file");
}

#[test]
fn frozen_sessions_mint_nothing_and_resume_keeps_numbering() {
    let (_dir, store) = open_store();
    let interned = store.intern(b"fz").unwrap();
    let span = ByteSpan::whole(interned.byte_len);
    let session = store.open_session().unwrap();
    assert_eq!(
        store
            .expose(session, interned.oid, span, "file", None)
            .unwrap(),
        1
    );
    store.freeze_session(session).unwrap();
    assert_eq!(
        store.session_status(session).unwrap().as_deref(),
        Some("frozen")
    );
    assert!(matches!(
        store.expose(session, interned.oid, span, "file", None),
        Err(Error::SessionInactive)
    ));
    // Resume returns the live session, and numbering continues past history.
    let resumed = store.resume_or_open_session().unwrap();
    assert_eq!(resumed, session);
    let next = store.open_session().unwrap();
    assert_eq!(
        store
            .expose(next, interned.oid, span, "file", None)
            .unwrap(),
        2
    );
}

#[test]
fn loc_serial_survives_abort() {
    let (_dir, store) = open_store();
    let interned = store.intern(b"staged").unwrap();
    let session = store.open_session().unwrap();
    let first = store
        .expose(
            session,
            interned.oid,
            ByteSpan::whole(interned.byte_len),
            "file",
            Some("src/a.rs"),
        )
        .unwrap();
    assert_eq!(first, 1);
    store
        .__test_bump_next_no_then_rollback(session)
        .expect("rollback must not recycle");
    let second = store
        .expose(
            session,
            interned.oid,
            ByteSpan::whole(interned.byte_len),
            "file",
            Some("src/a.rs"),
        )
        .unwrap();
    assert_eq!(second, 2, "aborted next_no bump must not reuse loc 1");
    let resolved = store.resolve(session, 1).unwrap();
    assert_eq!(resolved.oid, interned.oid);
}

#[test]
fn high_water_is_monotonic_across_sessions() {
    let (_dir, store) = open_store();
    let interned = store.intern(b"hw").unwrap();
    let a = store.open_session().unwrap();
    let span = ByteSpan::whole(interned.byte_len);
    assert_eq!(
        store.expose(a, interned.oid, span, "file", None).unwrap(),
        1
    );
    assert_eq!(
        store.expose(a, interned.oid, span, "file", None).unwrap(),
        2
    );
    let b = store.open_session().unwrap();
    assert_eq!(
        store.expose(b, interned.oid, span, "file", None).unwrap(),
        3,
        "a new session never restarts numbering at @1"
    );
}

#[test]
fn loc_quota_fails_closed() {
    let (_dir, store) = open_store();
    store.set_loc_quota(1).unwrap();
    let interned = store.intern(b"q").unwrap();
    let session = store.open_session().unwrap();
    let span = ByteSpan::whole(interned.byte_len);
    assert_eq!(
        store
            .expose(session, interned.oid, span, "file", None)
            .unwrap(),
        1
    );
    assert!(matches!(
        store.expose(session, interned.oid, span, "file", None),
        Err(Error::Quota)
    ));
}

#[test]
fn integrity_collision_fails_closed() {
    let (_dir, store) = open_store();
    let original = b"hello!!";
    let interned = store.intern(original).unwrap();
    store
        .__test_overwrite_payload_keep_seal(interned.oid, b"world!!")
        .unwrap();
    match store.intern(original) {
        Err(Error::IntegrityCollision) => {}
        other => panic!("expected IntegrityCollision, got {other:?}"),
    }
}

#[test]
fn persist_across_reopen() {
    let dir = tempfile::Builder::new()
        .prefix("cortex-refzero-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("refzero.sqlite");
    let oid = {
        let store = Store::open(&path).unwrap();
        store.intern(b"durable").unwrap().oid
    };
    let store = Store::open(&path).unwrap();
    let again = store.intern(b"durable").unwrap();
    assert_eq!(again.oid, oid);
    assert_eq!(store.payload(oid).unwrap(), b"durable");
}

#[test]
fn makeup_checksum_suite_loc_oid_stable() {
    let (_dir, store) = open_store();
    let session = Session::open(&store).unwrap();
    let interned = store.intern(b"stable-bytes").unwrap();
    let no = store
        .expose(
            session.id(),
            interned.oid,
            ByteSpan::whole(interned.byte_len),
            "file",
            Some("stable.txt"),
        )
        .unwrap();
    assert_eq!(no, 1);
    let oid = interned.oid;
    let again = store
        .intern_with_suite(b"stable-bytes", "blake3/2")
        .unwrap();
    assert_eq!(store.payload(oid).unwrap(), b"stable-bytes");
    let loc = store.resolve(session.id(), no).unwrap();
    assert_eq!(
        loc.oid, oid,
        "changing hidden suite must not rewrite loc oid"
    );
    assert_eq!(loc.no, 1);
    assert_ne!(again.oid, oid, "new suite is a new intern, not a rewrite");
}

#[test]
fn mint_table_reuse_paths_and_revisions() {
    let (_dir, store) = open_store();
    let s = Session::open(&store).unwrap();
    let a = b"AAAA";
    let b = b"BBBB";
    let span = ByteSpan::whole(4);

    let r1 = s.seed_read("p.rs", a, span, 1, "1:4", false).unwrap();
    let r1b = s.seed_read("p.rs", a, span, 1, "1:4", false).unwrap();
    assert!(r1b.reused);
    assert_eq!(r1.loc.no, r1b.loc.no);

    let q = s.seed_read("q.rs", a, span, 2, "1:4", false).unwrap();
    assert_ne!(q.loc.no, r1.loc.no);
    assert_eq!(q.loc.oid, r1.loc.oid);

    let r1c = s.seed_read("p.rs", a, span, 3, "1:4", false).unwrap();
    assert!(r1c.reused);

    let r2 = s.seed_read("p.rs", b, span, 4, "2:4", false).unwrap();
    assert!(!r2.reused);
    assert_ne!(r2.loc.no, r1.loc.no);
    assert_eq!(store.resolve(s.id(), r1.loc.no).unwrap().oid, r1.loc.oid);

    let r3 = s.seed_read("p.rs", a, span, 5, "3:4", false).unwrap();
    assert!(!r3.reused);
    assert_ne!(r3.loc.no, r1.loc.no);

    let staged = s.write_path("staged.rs", a, "0:4", true).unwrap();
    assert!(staged.granted_by.is_none());
}

#[test]
fn two_paths_same_bytes_no_cross_edit() {
    let (_dir, store) = open_store();
    let s = Session::open(&store).unwrap();
    let span = ByteSpan::whole(4);
    let a = s.seed_read("a.rs", b"SAME", span, 1, "1:4", false).unwrap();
    let b = s.seed_read("b.rs", b"SAME", span, 1, "1:4", false).unwrap();
    assert_eq!(a.loc.oid, b.loc.oid);
    assert_ne!(a.loc.no, b.loc.no);
    let bound = s
        .bind_edit(Some(&format!("@{}", a.loc.no)), 9, Some(b"SAME"))
        .unwrap();
    assert_eq!(bound.loc.origin, "a.rs");
}

#[test]
fn content_a_b_a_grant_death() {
    let (_dir, store) = open_store();
    let s = Session::open(&store).unwrap();
    let span = ByteSpan::whole(4);
    let l1 = s.seed_read("p.rs", b"AAAA", span, 1, "1:4", false).unwrap();
    let _l2 = s.write_path("p.rs", b"BBBB", "2:4", false).unwrap();
    let l3 = s.write_path("p.rs", b"AAAA", "3:4", false).unwrap();
    assert_ne!(l1.loc.no, l3.no);
    let err = s
        .bind_edit(Some(&format!("@{}", l1.loc.no)), 2, Some(b"AAAA"))
        .unwrap_err();
    assert_eq!(err.failure().unwrap().code, "bind_stale");
}

#[test]
fn g1_through_g5_next() {
    let (_dir, store) = open_store();
    let s = Session::open(&store).unwrap();
    let g1 = s.bind_edit(None, 1, None).unwrap_err();
    assert_eq!(g1.failure().unwrap().code, "unbound");
    assert_eq!(g1.failure().unwrap().next.tool, "Read");

    let hex = format!("z://blob/{}", "aa".repeat(32));
    let g2 = s.exact_read(&hex).unwrap_err();
    assert_eq!(g2.failure().unwrap().code, "not_a_loc");

    let g2b = s.exact_read("@readme.md").unwrap_err();
    assert_eq!(g2b.failure().unwrap().code, "not_a_loc");

    let span = ByteSpan::whole(4);
    let _ = s.seed_read("p.rs", b"AAAA", span, 1, "1:4", false).unwrap();
    let q = s.seed_read("q.rs", b"BBBB", span, 1, "1:4", false).unwrap();
    let amb = s.bind_edit(None, 1, Some(b"BBBB")).unwrap_err();
    assert_eq!(amb.failure().unwrap().code, "bind_ambiguous");
    assert_eq!(amb.failure().unwrap().next.tool, "Edit");

    let staged = s.write_path("s.rs", b"AAAA", "0:4", true).unwrap();
    let g5 = s
        .bind_edit(Some(&format!("@{}", staged.no)), 3, Some(b"AAAA"))
        .unwrap_err();
    assert_eq!(g5.failure().unwrap().code, "capsule_not_authority");

    let _ = q;
}

#[test]
fn loc_grammar_rejects_malformed_and_digests() {
    assert_eq!(parse_loc("@4"), Ok(4));
    assert_eq!(parse_loc("@1"), Ok(1));
    for s in ["@0", "@04", "", "@", "@s1/2", "@readme.md", "@4a"] {
        assert_eq!(parse_loc(s), Err(LocParseError::NotALoc), "{s}");
    }
    let hex = "ab".repeat(32);
    assert_eq!(
        parse_loc(&format!("z://blob/{hex}")),
        Err(LocParseError::NotALoc)
    );
    assert_eq!(parse_loc(&hex), Err(LocParseError::NotALoc));
    assert_eq!(format_loc(1).unwrap(), "@1");
    assert_eq!(format_loc(0), Err(LocParseError::NotALoc));
    assert_eq!(
        format_loc(MAX_SAFE_INTEGER + 1),
        Err(LocParseError::NotALoc)
    );
    assert_eq!(parse_loc(&format_loc(12).unwrap()).unwrap(), 12);
}

#[test]
fn identity_slot_true_pos_true_neg() {
    let hex = "cd".repeat(32);
    let uri = format!("z://blob/{hex}");
    let pos = json!({
        "handle": uri,
        "loc": "@4",
        "nested": { "refetch": hex }
    });
    let hits = identity_slot_hits(&pos);
    assert!(hits.iter().any(|p| p == "/handle"), "{hits:?}");
    assert!(hits.iter().any(|p| p == "/nested/refetch"), "{hits:?}");
    assert!(!hits.iter().any(|p| p == "/loc"), "loc @4 is not a digest");

    let payload = format!("see digest {hex} in the README");
    let neg = json!({
        "path": "README.md",
        "text": payload.clone(),
        "inline_utf8": payload,
        "content": payload,
    });
    assert!(
        identity_slot_hits(&neg).is_empty(),
        "payload hex must not be flagged or redacted: {:?}",
        identity_slot_hits(&neg)
    );
    assert!(neg["text"].as_str().unwrap().contains(&hex));
}

#[test]
fn recall_envelope_shape_round_trip() {
    let (_dir, store) = open_store();
    let interned = store.intern(b"envelope-bytes").unwrap();
    let env = RecallEnvelope::from_oid(interned.oid, ByteSpan::whole(interned.byte_len));
    assert_eq!(env.oid_hex.len(), 32);
    assert!(
        env.oid_hex
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
    assert_eq!((env.start, env.end), (0, 14));
    let json = serde_json::to_value(&env).unwrap();
    assert_eq!(
        json,
        json!({"oid_hex": env.oid_hex, "start": 0, "end": 14}),
        "exact cross-product envelope shape"
    );
    let back: RecallEnvelope = serde_json::from_value(json).unwrap();
    assert_eq!(back.oid().unwrap(), interned.oid);
    assert_eq!(back.span(), ByteSpan::whole(14));
}
