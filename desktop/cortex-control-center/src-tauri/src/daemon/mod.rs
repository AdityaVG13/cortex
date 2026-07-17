pub mod paths;
pub mod process;
pub mod shutdown;
pub mod spawn;
pub mod state;
pub mod supervisor;

pub use shutdown::shutdown_daemon;
pub use state::{AppInstanceGuard, DaemonState, LifecycleState};

#[cfg(test)]
mod tests;
