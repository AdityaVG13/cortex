pub const DEFAULT_CORTEX_PORT: u16 = 7437;

pub use cortex_kernel::aging;
pub use cortex_kernel::auth;
pub mod cli;
pub use cortex_kernel::compaction;
pub use cortex_kernel::compiler;
pub use cortex_kernel::crystallize;

pub use cortex_kernel::db;
pub use cortex_kernel::export_data;
pub use cortex_kernel::focus;
pub mod handlers;
pub mod hook_boot;
pub use cortex_kernel::hook_event;
pub use cortex_kernel::indexer;
pub mod mcp_native;
pub mod prompt_inject;
pub use cortex_kernel::reflex;
pub use cortex_kernel::runtime;

pub mod setup;
pub use cortex_kernel::state;
pub use cortex_kernel::store_spi;
pub use cortex_kernel::workspace;

pub use cortex_logic::{adapter, api_types, budgets, capture, clockwork, conflict, eval, graph, lens, presence, protocol, rate_limit, recipe, traces};

pub use cli::run_daemon;
pub use cortex_kernel::{CortexError, CortexRuntime};

use chrono::Utc;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) fn install_daemon_panic_hook(paths: &auth::CortexPaths) {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let panic_log_path = paths.home.join("panic.log");
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "<unknown location>".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture();
        let entry = format!("[{ts}] PANIC at {location}: {message}\n{backtrace}\n", ts = Utc::now().to_rfc3339(),);
        eprintln!("[cortex] {entry}");
        if let Some(parent) = panic_log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&panic_log_path) {
            let _ = file.write_all(entry.as_bytes());
        }
        previous(info);
    }));
}
