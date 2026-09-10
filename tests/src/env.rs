use asupersync::{
    Cx,
    sync::{LockError, Mutex, MutexGuard},
};
use std::ffi::OsStr;
use std::sync::OnceLock;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn lock() -> MutexGuard<'static, ()> {
    crate::support::run_with_cx(
        |cx| async move { lock_async(&cx).await.expect("environment lock") },
    )
}

pub async fn lock_async(cx: &Cx) -> Result<MutexGuard<'static, ()>, LockError> {
    ENV_LOCK.get_or_init(|| Mutex::new(())).lock(cx).await
}

/// Re-run one contract in a child whose environment is fixed before startup.
/// Returns true only inside that child; the parent verifies its exit status.
/// No process-global environment mutation occurs in either process.
pub fn in_subprocess(test: &str, variables: &[(&str, Option<&OsStr>)]) -> bool {
    const CHILD: &str = "CORTEX_TEST_ENV_CHILD";
    if std::env::var(CHILD).ok().as_deref() == Some(test) {
        return true;
    }
    let mut child = std::process::Command::new(std::env::current_exe().expect("test executable"));
    child
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD, test);
    for (key, value) in variables {
        match value {
            Some(value) => {
                child.env(key, value);
            }
            None => {
                child.env_remove(key);
            }
        }
    }
    let output = child.output().expect("run isolated contract");
    assert!(
        output.status.success(),
        "isolated contract {test} failed ({}):\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}
