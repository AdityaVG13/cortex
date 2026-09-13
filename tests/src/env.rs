use asupersync::{
    Cx,
    sync::{LockError, Mutex, MutexGuard},
};
use std::ffi::OsStr;
use std::io::Read;
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
/// Drain-pipe threads are joined only after the child is reaped: joining first
/// deadlocks when the child is hung (pipe still open).
struct KillChildOnDrop {
    child: Option<std::process::Child>,
    stdout: Option<thread::JoinHandle<Vec<u8>>>,
    stderr: Option<thread::JoinHandle<Vec<u8>>>,
}

impl KillChildOnDrop {
    fn take_pipes(&mut self) -> (Vec<u8>, Vec<u8>) {
        let stdout = self
            .stdout
            .take()
            .map(|handle| handle.join().unwrap_or_else(|_| Vec::new()))
            .unwrap_or_default();
        let stderr = self
            .stderr
            .take()
            .map(|handle| handle.join().unwrap_or_else(|_| Vec::new()))
            .unwrap_or_default();
        (stdout, stderr)
    }
}

impl Drop for KillChildOnDrop {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = self.stdout.take().map(|handle| handle.join());
        let _ = self.stderr.take().map(|handle| handle.join());
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
    cmd.args([
        "--exact",
        test,
        "--nocapture",
        "--test-threads=1",
        "--color=never",
    ])
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
    let mut child = KillChildOnDrop {
        child: Some(cmd.spawn().expect("run isolated contract")),
        stdout: None,
        stderr: None,
    };
    {
        let proc = child.child.as_mut().expect("isolated contract child");
        let mut stdout_pipe = proc.stdout.take().expect("stdout pipe");
        let mut stderr_pipe = proc.stderr.take().expect("stderr pipe");
        child.stdout = Some(thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout_pipe.read_to_end(&mut buf);
            buf
        }));
        child.stderr = Some(thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr_pipe.read_to_end(&mut buf);
            buf
        }));
    }
    let deadline = Instant::now() + WAIT;
    loop {
        let proc = child.child.as_mut().expect("isolated contract child");
        match proc.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let mut proc = child.child.take().expect("isolated contract child");
                    let _ = proc.kill();
                    let status = proc.wait().expect("reap isolated contract");
                    let (stdout_bytes, stderr_bytes) = child.take_pipes();
                    let stdout = String::from_utf8_lossy(&stdout_bytes);
                    let stderr = String::from_utf8_lossy(&stderr_bytes);
                    panic!(
                        "isolated contract {test} timed out ({status}):\n{stdout}\n{stderr}"
                    );
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(err) => panic!("isolated contract {test} wait failed: {err}"),
        }
    }
    let status = child
        .child
        .take()
        .expect("isolated contract child")
        .wait()
        .expect("collect isolated contract");
    let (stdout_bytes, stderr_bytes) = child.take_pipes();
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let stderr = String::from_utf8_lossy(&stderr_bytes);
    assert!(
        status.success(),
        "isolated contract {test} failed ({status}):\n{stdout}\n{stderr}"
    );
    // libtest exits 0 for `--exact` with zero matches; that would hide the contract.
    assert!(
        stdout.lines().any(|line| line.trim() == "running 1 test"),
        "isolated contract {test} did not run (a 0-test child would hide the contract):\n{stdout}\n{stderr}"
    );
    // `--exact` ignored would still print "running 1 test" when the binary has one test.
    assert!(
        stdout.lines().any(|line| {
            let line = line.trim();
            line.starts_with(&format!("test {test} ")) || line.starts_with(&format!("test {test}..."))
        }),
        "isolated contract {test} ran a different test (wrong filter would hide the contract):\n{stdout}\n{stderr}"
    );
    false
}
