pub mod paths;
pub mod process;
pub mod shutdown;
pub mod spawn;
pub mod state;
pub mod supervisor;

pub use shutdown::shutdown_daemon;
pub use state::{AppInstanceGuard, DaemonState, LifecycleState};
pub use supervisor::{join_supervisor, request_supervisor_stop, SupervisorControl};

#[cfg(test)]
#[path = "../../../../../tests/control-center/rust/daemon.rs"]
mod tests;
