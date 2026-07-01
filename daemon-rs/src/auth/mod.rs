// SPDX-License-Identifier: MIT
mod paths;
mod migration;
mod locks;
mod keys;
mod runtime;

#[cfg(test)]
mod tests;

pub use paths::CortexPaths;
pub use migration::{legacy_db_path, migrate_legacy_db};
pub use locks::{acquire_daemon_lock, acquire_global_daemon_lock};
pub use keys::{
    cortex_dir, generate_ctx_api_key, generate_ephemeral_token, hash_api_key_argon2id, read_token,
    read_token_from, try_generate_token, try_generate_token_for, try_write_token_for,
    verify_api_key_argon2id, verify_ctx_api_key_checksum,
};
pub use runtime::{cleanup_stale_pid_lock, db_path, stale_pid_candidate, write_pid};

pub(crate) use paths::{restrict_file_to_owner, write_secret_file};
