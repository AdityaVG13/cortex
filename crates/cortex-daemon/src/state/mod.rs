mod init;
mod read_pool;
mod runtime;

mod types;
pub use init::initialize;
pub use runtime::RuntimeState;
pub use types::*;
/// TEST-API: re-exported so the extracted test-support crate can name the
/// trait object used by `RuntimeState::db_read` without `cfg(test)`.
pub use read_pool::ReadConnectionProvider;
