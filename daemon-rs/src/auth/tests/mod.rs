// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use crate::auth::paths::CORTEX_GLOBAL_LOCK_HOME_ENV;
    use crate::auth::{acquire_global_daemon_lock, generate_ctx_api_key, verify_ctx_api_key_checksum};
    use crate::test_env::{lock, ScopedEnvVar};
    fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        lock()
    }
    #[test]
    fn verify_ctx_api_key_checksum_accepts_generated_keys() {
        let key = generate_ctx_api_key();
        assert!(verify_ctx_api_key_checksum(&key));
    }
    #[test]
    fn acquire_global_daemon_lock_rejects_duplicate_instances() {
        let _guard = env_guard();
        let lock_home = std::env::temp_dir().join(format!(
            "cortex-global-lock-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
        ));
        let _home_var = ScopedEnvVar::set(CORTEX_GLOBAL_LOCK_HOME_ENV, &lock_home);
        let _first = acquire_global_daemon_lock().expect("first lock");
        let second = acquire_global_daemon_lock();
        assert!(second.is_err());
    }
}
