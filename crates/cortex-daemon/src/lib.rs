pub const DEFAULT_CORTEX_PORT: u16 = 7437;

pub mod aging;
pub mod api_types;
pub mod auth;
pub mod budgets;
pub mod cli;
pub mod compaction;
pub mod compiler;
pub mod conflict;
pub mod crystallize;
pub mod daemon_lifecycle;
pub mod db;
pub mod embeddings;
pub mod eval;
pub mod export_data;
pub mod focus;
pub mod handlers;
pub mod hook_boot;
pub mod indexer;
pub mod mcp_proxy;
pub mod prompt_inject;
pub mod rate_limit;
pub mod rerank;
pub mod server;
pub mod service;
pub mod setup;
pub mod state;

pub mod tls;
pub mod transport;
pub mod workspace;

pub use cli::run_daemon;

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
        let entry = format!(
            "[{ts}] PANIC at {location}: {message}\n{backtrace}\n",
            ts = Utc::now().to_rfc3339(),
        );
        eprintln!("[cortex] {entry}");
        if let Some(parent) = panic_log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&panic_log_path)
        {
            let _ = file.write_all(entry.as_bytes());
        }
        previous(info);
    }));
}
