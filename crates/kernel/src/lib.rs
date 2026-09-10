//! cortex-kernel — the embeddable Cortex brain.
//!
//! In-process store / recall / boot against the SQLite brain. No server, no
//! port, no HTTP types in the public surface. `cortex-daemon` is a transport
//! adapter over this crate; a Rust host (Outfit, tests, scripts) opens a
//! [`CortexRuntime`] directly:
//!
//! ```ignore
//! let rt = cortex_kernel::CortexRuntime::open_db(&db_path)?;
//! // cx: &asupersync::Cx is supplied by the host's current task.
//! rt.deposit(cx, "req-1", "PAY-12 ledger writes are idempotent", "outfit", None).await?;
//! let view = rt.lens(cx, LensInput { query: "PAY-12".into(), agent: "outfit".into(), ..Default::default() }).await?;
//! ```

pub const DEFAULT_CORTEX_PORT: u16 = 7437;

pub mod aging;
pub mod auth;
pub mod compaction;
pub mod compiler;
pub mod crystallize;
pub mod db;
pub mod export_data;
pub mod focus;
pub mod handlers;
pub mod hook_event;
pub mod hybrid_search;
pub mod indexer;
pub mod reflex;
pub mod runtime;
pub mod state;
pub mod store_spi;
pub mod workspace;

pub use cortex_logic::{
    adapter, api_types, budgets, capture, clockwork, conflict, eval, graph, lens, presence,
    protocol, rate_limit, recipe, traces,
};
pub use runtime::{BootInput, CortexError, CortexRuntime, DepositOutcome, LensInput};
