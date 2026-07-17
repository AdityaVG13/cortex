use std::ffi::{OsStr, OsString};
use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
pub fn lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.get_or_init(|| Mutex::new(())).blocking_lock()
}
pub async fn lock_async() -> MutexGuard<'static, ()> {
    ENV_LOCK.get_or_init(|| Mutex::new(())).lock().await
}
pub struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}
impl ScopedEnvVar {
    pub fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
    pub fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, previous }
    }
}
impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}
