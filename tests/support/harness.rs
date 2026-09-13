use std::path::PathBuf;

pub fn unique_temp_dir(prefix: &str) -> PathBuf {
    cortex_tests::support::unique_temp_dir(prefix)
}
