use asupersync::{
    Cx,
    sync::{LockError, Mutex, MutexGuard},
};
use std::ffi::OsStr;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn lock() -> MutexGuard<'static, ()> {
    crate::support::run_with_cx(
        |cx| async move { lock_async(&cx).await.expect("environment lock") },
    )
}

pub async fn lock_async(cx: &Cx) -> Result<MutexGuard<'static, ()>, LockError> {
    ENV_LOCK.get_or_init(|| Mutex::new(())).lock(cx).await
}

/// Kill and reap a child if the caller panics or times out before taking it.
struct KillChildOnDrop(Option<std::process::Child>);

impl Drop for KillChildOnDrop {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Re-run one contract in a child whose environment is fixed before startup.
/// Returns true only inside that child; the parent verifies its exit status.
/// No process-global environment mutation occurs in either process.
pub fn in_subprocess(test: &str, variables: &[(&str, Option<&OsStr>)]) -> bool {
    const CHILD: &str = "CORTEX_TEST_ENV_CHILD";
    const WAIT: Duration = Duration::from_secs(30);
    if std::env::var(CHILD).ok().as_deref() == Some(test) {
        return true;
    }
    let mut cmd = Command::new(std::env::current_exe().expect("test executable"));
    cmd.args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD, test)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in variables {
        match value {
            Some(value) => {
                cmd.env(key, value);
            }
            None => {
                cmd.env_remove(key);
            }
        }
    }
    let mut child = KillChildOnDrop(Some(cmd.spawn().expect("run isolated contract")));
    let deadline = Instant::now() + WAIT;
    loop {
        let proc = child.0.as_mut().expect("isolated contract child");
        match proc.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let mut proc = child.0.take().expect("isolated contract child");
                    let _ = proc.kill();
                    let output = proc.wait_with_output().expect("reap isolated contract");
                    panic!(
                        "isolated contract {test} timed out ({}):\n{}\n{}",
                        output.status,
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(err) => panic!("isolated contract {test} wait failed: {err}"),
        }
    }
    let output = child
        .0
        .take()
        .expect("isolated contract child")
        .wait_with_output()
        .expect("collect isolated contract");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "isolated contract {test} failed ({}):\n{stdout}\n{stderr}",
        output.status
    );
    // libtest exits 0 for `--exact` with zero matches; that would hide the contract.
    assert!(
        stdout.lines().any(|line| line.trim() == "running 1 test"),
        "isolated contract {test} did not run (a 0-test child would hide the contract):\n{stdout}\n{stderr}"
    );
    false
}
