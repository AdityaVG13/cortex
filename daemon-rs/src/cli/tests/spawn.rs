// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use crate::cli::daemon::{background_db_lock_max_wait, validate_spawned_owner_runtime_claim, BACKGROUND_DB_LOCK_MAX_WAIT_MS_ENV, SPAWN_PARENT_PID_ENV};
    use crate::cli::tests::support::*;
    use crate::cli::*;
    use crate::*;
    use std::time::Duration;
    #[test]
    fn background_db_lock_wait_env_is_clamped() {
        let _env_guard = env_guard();
        let _small = ScopedEnvVar::set(BACKGROUND_DB_LOCK_MAX_WAIT_MS_ENV, "1");
        assert_eq!(background_db_lock_max_wait(), Duration::from_millis(100));
        drop(_small);
        let _large = ScopedEnvVar::set(BACKGROUND_DB_LOCK_MAX_WAIT_MS_ENV, "70000");
        assert_eq!(background_db_lock_max_wait(), Duration::from_millis(60_000));
    }
    #[test]
    fn spawned_owner_runtime_claim_requires_parent_linkage_for_plugin_owner() {
        let home_dir = temp_test_dir("owner_runtime_parent");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);
        let err = validate_spawned_owner_runtime_claim(&paths, Some("plugin-claude"), None, None, None).unwrap_err();
        assert!(err.contains(SPAWN_PARENT_PID_ENV));
        let _ = std::fs::remove_dir_all(&home_dir);
    }
    #[test]
    fn spawned_owner_runtime_claim_allows_unspawned_control_center_mode() {
        let home_dir = temp_test_dir("owner_runtime_unspawned");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);
        validate_spawned_owner_runtime_claim(&paths, Some("control-center"), None, None, None)
            .expect("direct control-center owner mode should remain compatible");
        let _ = std::fs::remove_dir_all(&home_dir);
    }
}
