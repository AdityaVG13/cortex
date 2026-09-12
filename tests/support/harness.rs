use std::path::PathBuf;

/// Exclusive directory under the process temp root.
///
/// Clock-only names can collide under parallel tests and reuse a leftover
/// SQLite/WAL from a previous run (`create_dir_all` on a dirty path). Callers
/// that `remove_dir_all` still clean up; Drop of this PathBuf does not.
pub fn unique_temp_dir(prefix: &str) -> PathBuf {
    tempfile::Builder::new()
        .prefix(&format!("cortex-{prefix}-"))
        .tempdir()
        .expect("unique temp dir")
        .keep()
}
