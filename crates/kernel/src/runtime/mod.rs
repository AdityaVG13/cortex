//! CortexRuntime: the library entry point every adapter (HTTP, MCP, CLI,
//! hooks, SDK subprocess) composes. It owns the memory semantics; adapters own
//! transport, authentication and presentation.
//!
//! Invariants: no HTTP, process supervision or provider clients are imported
//! here; a handle caches connections but the durable brain and request
//! identity live outside it; multiple handles may open the same backend.
//! Determinism class: deterministic given the database state and inputs.

mod deposit;
pub mod observation;
pub mod cycle;
pub mod inventory;
pub mod host_capture;
pub mod associations;
pub mod assembly;
mod open;

pub use deposit::{DepositInput, DepositOutcome, ack_profile_label_pub, deposit_decision};
pub use open::{BootInput, CortexError, CortexRuntime, LensInput};
