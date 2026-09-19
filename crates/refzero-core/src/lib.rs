//! Cortex RefZero identity store.
//!
//! Port of ZeroStack `refzero-core` (pinned `2e220e1` + `522a814` + `39ae0fd`)
//! with byte-identical semantics: interned byte objects (UUIDv4 oid, hidden
//! BLAKE3 seal + byte length), session-scoped `@N` locs (commit-before-reveal
//! serials, first loc = 1, monotonic high-water, quota), grants, import map,
//! and recall envelopes. The SQLite schema is identical to the reference; only
//! the driver differs (rusqlite here, fsqlite there) so Cortex keeps one
//! sqlite stack. See `CONTRACT.md` for the port deltas.
//!
//! Crate name is `cortex-refzero` (not `refzero-core`) so a harness that
//! builds both products in one process never sees two same-named crates.

#![forbid(unsafe_code)]

pub mod digest;
mod error;
mod identity;
mod ids;
mod importer;
mod loc;
mod mint;
mod recall;
mod store;
pub mod zeroref;

pub use error::{BindError, Error, Failure, NextCall};
pub use identity::identity_slot_hits;
pub use ids::{ObjectId, SessionId};
pub use importer::{
    Algorithm, FrozenFragment, ImportKey, ImportRequest, Imported, Producer, import,
    required_algorithm,
};
pub use loc::{LocParseError, MAX_SAFE_INTEGER, format_loc, is_digest_spelling, parse_loc};
pub use mint::{BoundEdit, Seed, Session};
pub use recall::RecallEnvelope;
pub use store::{ByteSpan, DEFAULT_SEAL_SUITE, Interned, Loc, Store};

/// Guest loc numbers are `@[1-9][0-9]*`. The store issues `u64` starting at 1.
pub type LocNo = u64;
