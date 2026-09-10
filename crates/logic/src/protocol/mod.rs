//! Protocol vocabulary shared by every surface (HTTP, MCP, CLI, SDKs).
//!
//! Determinism class: pure data; no clocks, no I/O.
//! Invariants:
//! - identity kinds are distinct Rust types and never interchangeable (`ids`);
//! - every response carries one `ResponseStatus`; transport codes derive from it,
//!   never the reverse (`status`);
//! - the request envelope separates adapter-owned authority from model-supplied
//!   selectors and keeps temporal/cursor/presence fields independent (`envelope`);
//! - a `Receipt` reports durability as a vector, never a boolean (`receipt`).
//! No-claim boundary: these types validate shape and vocabulary, not policy.

pub mod envelope;
pub mod ids;
pub mod receipt;
pub mod status;

pub use envelope::{
    ContextPresence, Envelope, EnvelopeError, Limits, PrincipalContext, Scope, KNOWN_OPERATIONS,
};
pub use ids::{ExactRef, Frontier, Integrity, Locator, LogicalId, Principal};
pub use receipt::{
    AckProfile, CaptureReceipt, CaptureStatus, DurabilityVector, PayloadAvailability, Receipt,
};
pub use status::ResponseStatus;
