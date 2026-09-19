//! RefZero wire-in: the session-owned identity sidecar next to the brain DB.
//!
//! The sidecar (`<brain>.refzero.sqlite`) holds interned objects and locs with
//! byte-identical semantics to ZeroStack's identity store, so a harness
//! running both products can pass [`RecallEnvelope`]s and `z://blob/` refs
//! between them. It is daemonless: a plain SQLite file beside the brain, never
//! a machine-wide service.
//!
//! Intake hooks ([`intern_store_bytes_for_conn`], [`intern_store_bytes_for_db`])
//! are best-effort like `traces::record_store_write`: a sidecar failure warns
//! and never fails the store. Recall envelopes carry identity only, never an
//! Edit grant; importing an envelope mints a fresh loc on the receiving
//! session. Digest spellings in identity slots fail closed as `not_a_loc`.

pub use cortex_refzero::{
    Algorithm, BoundEdit, ByteSpan, Error, FrozenFragment, ImportKey, ImportRequest, Imported,
    Interned, Loc, LocNo, LocParseError, MAX_SAFE_INTEGER, ObjectId, Producer, RecallEnvelope,
    Seed, Session, SessionId, Store, format_loc, identity_slot_hits, import, is_digest_spelling,
    parse_loc,
};
pub use cortex_refzero::{digest, zeroref};

use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Sidecar path for a brain DB: `<stem>.refzero.sqlite` beside it. `None`
/// for `:memory:`/unnamed databases, which have no sibling directory.
pub fn sidecar_path_for_db(db_path: &Path) -> Option<PathBuf> {
    let name = db_path.file_name()?.to_str()?;
    if name.is_empty() || name == ":memory:" {
        return None;
    }
    let stem = name.split('.').next().unwrap_or(name);
    let stem = if stem.is_empty() { "cortex" } else { stem };
    Some(
        db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{stem}.refzero.sqlite")),
    )
}

/// Resolve the sidecar from a live connection's `main` database file.
pub fn sidecar_path_for_conn(conn: &Connection) -> Option<PathBuf> {
    let file: Option<String> = conn
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name = 'main'",
            [],
            |r| r.get(0),
        )
        .ok()?;
    let file = file.filter(|f| !f.is_empty())?;
    sidecar_path_for_db(Path::new(&file))
}

/// Open (creating parent dirs) the sidecar for a brain DB path.
pub fn open_sidecar(db_path: &Path) -> Result<Store, String> {
    let path = sidecar_path_for_db(db_path)
        .ok_or_else(|| "refzero sidecar needs a file-backed brain DB".to_string())?;
    Store::open(&path).map_err(|e| e.to_string())
}

/// Best-effort intern of stored bytes. Warns, never fails the store.
pub fn intern_store_bytes_for_db(db_path: &Path, bytes: &[u8]) -> Option<Interned> {
    sidecar_path_for_db(db_path).and_then(|path| cached_intern(&path, bytes))
}

/// Best-effort intern of stored bytes via a live connection.
pub fn intern_store_bytes_for_conn(conn: &Connection, bytes: &[u8]) -> Option<Interned> {
    sidecar_path_for_conn(conn).and_then(|path| cached_intern(&path, bytes))
}

/// Process-wide cache of sidecar handles for the intake hooks, so deposits
/// pay one SQLite open per sidecar path instead of one per deposit. Every
/// cached use is a complete committed transaction, so entries can be dropped
/// at any time; the hooks stay correct (just slower) on every path,
/// including lock poisoning. `open_sidecar` stays uncached: harness-owned
/// handles must never be shared behind the caller's back.
fn sidecar_cache() -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, Store>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<PathBuf, Store>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

const SIDECAR_CACHE_CAP: usize = 32;

fn cached_intern(path: &Path, bytes: &[u8]) -> Option<Interned> {
    let mut map = sidecar_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if map.len() >= SIDECAR_CACHE_CAP {
        map.clear();
    }
    let store = match map.entry(path.to_path_buf()) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::hash_map::Entry::Vacant(vacant) => match Store::open(path) {
            Ok(store) => vacant.insert(store),
            Err(err) => {
                eprintln!("[refzero] Warning: failed to open identity sidecar: {err}");
                return None;
            }
        },
    };
    match store.intern(bytes) {
        Ok(interned) => Some(interned),
        Err(err) => {
            eprintln!("[refzero] Warning: failed to intern stored bytes: {err}");
            // Drop a handle that errored: the next call reopens fresh rather
            // than pinning a broken connection.
            map.remove(path);
            None
        }
    }
}

/// Test helper: number of cached sidecar handles.
#[doc(hidden)]
pub fn __test_cache_len() -> usize {
    sidecar_cache().lock().map(|map| map.len()).unwrap_or(0)
}

/// Export a loc as a portable recall envelope (`{oid_hex, start, end}`).
pub fn export_envelope(
    store: &Store,
    session: SessionId,
    no: LocNo,
) -> Result<RecallEnvelope, String> {
    let loc = store.resolve(session, no).map_err(|e| e.to_string())?;
    Ok(RecallEnvelope::from_oid(loc.oid, loc.span))
}

/// Import an envelope plus its bytes, minting a fresh loc on `session` with
/// **no** Edit grant. The sender's oid is adopted so the identity survives
/// the hop; a span outside the object fails closed.
pub fn import_envelope(
    store: &Store,
    session: SessionId,
    envelope: &RecallEnvelope,
    bytes: &[u8],
) -> Result<LocNo, String> {
    let oid = envelope.oid().map_err(|e| e.to_string())?;
    let interned = store.adopt_object(oid, bytes).map_err(|e| e.to_string())?;
    let span = envelope.span();
    if span.start > span.end || span.end > interned.byte_len {
        return Err(Error::InvalidSpan {
            start: span.start,
            end: span.end,
            len: interned.byte_len,
        }
        .to_string());
    }
    store
        .expose_imported(session, oid, span)
        .map_err(|e| e.to_string())
}

/// Fail closed when any identity slot still holds a digest spelling.
/// The error names JSON pointers, never the digest values.
pub fn require_no_digest_in_identity(value: &serde_json::Value) -> Result<(), String> {
    let hits = identity_slot_hits(value);
    if hits.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "not_a_loc: digest spelling in identity slot(s): {}",
            hits.join(", ")
        ))
    }
}

/// Parse a guest identity string. Anything that is not `@[1-9][0-9]*` —
/// including digest spellings — fails closed as `not_a_loc` without echoing
/// the digest.
pub fn resolve_loc_no(name: &str) -> Result<LocNo, String> {
    match parse_loc(name) {
        Ok(no) => Ok(no),
        Err(LocParseError::NotALoc) if is_digest_spelling(name) => Err("not_a_loc".to_string()),
        Err(LocParseError::NotALoc) => Err(format!("not_a_loc: {name} is not a loc")),
    }
}

/// Portable blob ref for complete bytes: `z://blob/<blake3>`.
pub fn blob_ref_for_bytes(bytes: &[u8]) -> String {
    format!("z://blob/{}", digest::digest_hex(bytes))
}

/// Verify a blob ref against complete bytes, then select its fragment.
/// Digest mismatch precedes selection; `#B` is zero-based half-open, `#L` is
/// one-based inclusive with canonical clamp-end policy.
pub fn select_blob_ref(ref_text: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let parsed = zeroref::ZeroRef::parse(ref_text).map_err(|e| e.to_string())?;
    parsed
        .verify_and_select(bytes)
        .map(|s| s.to_vec())
        .map_err(|e| e.to_string())
}
