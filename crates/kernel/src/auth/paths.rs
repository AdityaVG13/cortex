use crate::protocol::nonempty_trimmed;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::open_windows_rejecting_name_surrogate;
#[cfg(windows)]
pub use windows::restrict_file_to_owner;
pub const CORTEX_DIR_NAME: &str = ".cortex";
pub const CORTEX_GLOBAL_LOCK_NAME: &str = "cortex.global.lock";
pub const CORTEX_GLOBAL_LOCK_HOME_ENV: &str = "CORTEX_GLOBAL_LOCK_HOME";
pub const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// Token / secret files are UUID-sized. Match the Control Center cap so a
/// replaced huge file cannot OOM `read_token_from` at runtime open.
pub const MAX_SECRET_FILE_BYTES: u64 = 8 * 1024;
#[derive(Debug, Clone)]
pub struct CortexPaths {
    pub home: PathBuf,
    pub db: PathBuf,
    pub token: PathBuf,
    pub pid: PathBuf,
    pub lock: PathBuf,
    #[allow(dead_code)]
    pub write_buffer: PathBuf,
}
static PROCESS_PATHS: OnceLock<CortexPaths> = OnceLock::new();
impl CortexPaths {
    /// Pin `--home` / `--db` (or the env resolve) for later `resolve()` callers.
    /// Does not mutate the process environment.
    pub fn install_process_paths(paths: &Self) {
        let _ = PROCESS_PATHS.set(paths.clone());
    }
    pub(crate) fn process_paths() -> Option<&'static Self> {
        PROCESS_PATHS.get()
    }
    /// After [`Self::install_process_paths`], returns that cell. Otherwise
    /// reads operator `CORTEX_HOME` / `CORTEX_DB`.
    pub fn resolve() -> Self {
        if let Some(paths) = PROCESS_PATHS.get() {
            return paths.clone();
        }
        Self::resolve_with_overrides(None, None)
    }
    /// Set `CORTEX_HOME` / `CORTEX_DB` on a child only (safe `Command::env`).
    pub fn apply_to_command(&self, command: &mut std::process::Command) {
        command.env("CORTEX_HOME", &self.home);
        command.env("CORTEX_DB", &self.db);
    }
    pub fn resolve_with_overrides(home_override: Option<&str>, db_override: Option<&str>) -> Self {
        let home = home_override
            .map(PathBuf::from)
            .or_else(|| nonempty_env_path("CORTEX_HOME"))
            .unwrap_or_else(|| default_home_root().join(CORTEX_DIR_NAME));
        let db = db_override
            .map(PathBuf::from)
            .or_else(|| nonempty_env_path("CORTEX_DB"))
            .unwrap_or_else(|| home.join("cortex.db"));
        Self {
            token: home.join("cortex.token"),
            pid: home.join("cortex.pid"),
            lock: home.join("cortex.lock"),
            write_buffer: home.join("write_buffer.jsonl"),
            home,
            db,
        }
    }
    /// `--home --db /tmp/x` must not treat `--db` as the home path.
    /// `--home -- --odd` keeps a home whose name starts with `--`.
    pub fn resolve_from_args(args: &[String]) -> Self {
        let home = parse_flag_value(args, "--home");
        let db = parse_flag_value(args, "--db");
        Self::resolve_with_overrides(home.as_deref(), db.as_deref())
    }
    pub fn to_json(&self) -> String {
        serde_json::json!({"home":self.home.display().to_string(),"db":self.db.display().to_string(),"token":self.token.display().to_string(),"pid":self.pid.display().to_string()}).to_string()
    }
    /// Operator-owned live capture sidecar. `CORTEX_CAPTURE` overrides this file.
    pub fn capture_sidecar(&self) -> PathBuf {
        self.home.join("capture.json")
    }
}
pub fn is_flag_token(value: &str) -> bool {
    value.starts_with("--")
}

/// Value after a flag. `--` quotes the next token so a path or name may start with `--`.
/// Returns `(value, index_after_consumed_tokens)`.
pub fn take_flag_value(args: &[String], flag_index: usize) -> Option<(String, usize)> {
    let value = args.get(flag_index + 1)?;
    if value == "--" {
        let explicit = args.get(flag_index + 2).filter(|v| !v.trim().is_empty())?;
        return Some((explicit.clone(), flag_index + 3));
    }
    (!is_flag_token(value) && !value.trim().is_empty()).then(|| (value.clone(), flag_index + 2))
}

pub fn parse_flag_values(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut i = 0usize;
    while i < args.len() {
        if args[i] == flag {
            if let Some((value, next)) = take_flag_value(args, i) {
                values.push(value);
                i = next;
                continue;
            }
        }
        i += 1;
    }
    values
}

pub fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    parse_flag_values(args, flag).into_iter().next()
}

/// Blank / whitespace `CORTEX_HOME` / `CORTEX_DB` must not become cwd-relative
/// `cortex.token` paths that disagree with `cortex_dir()` and Control Center.
pub(crate) fn nonempty_env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .and_then(nonempty_trimmed)
        .map(PathBuf::from)
}

pub fn default_home_root() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
#[cfg(unix)]
pub fn restrict_file_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}
#[cfg(not(any(unix, windows)))]
pub fn restrict_file_to_owner(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
/// Open a home-local automatic file without following a planted symlink
/// (Unix `O_NOFOLLOW`) or Windows name-surrogate reparse point. Operator
/// `--file` paths may still follow; this is for files Cortex opens on its own.
pub fn open_nofollow(path: &Path) -> io::Result<fs::File> {
    open_configured_nofollow(path, |opts| {
        opts.read(true);
    })
}

/// Append to a home-local ledger without following a planted symlink.
pub fn open_append_nofollow(path: &Path) -> io::Result<fs::File> {
    open_configured_nofollow(path, |opts| {
        opts.create(true).append(true);
    })
}

/// Open `path` with caller flags, without following a planted symlink.
///
/// Windows OneDrive/dedup files are reparse points but not name surrogates;
/// refusing every reparse attribute made those homes unreadable. Inspect
/// with `FILE_FLAG_OPEN_REPARSE_POINT`, reject symlink/junction tags, then
/// reopen without the flag so cloud/dedup content is the file bytes.
pub(crate) fn open_configured_nofollow(
    path: &Path,
    configure: impl Fn(&mut fs::OpenOptions),
) -> io::Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = fs::OpenOptions::new();
        configure(&mut opts);
        opts.custom_flags(libc::O_NOFOLLOW).open(path)
    }
    #[cfg(windows)]
    {
        open_windows_rejecting_name_surrogate(path, configure)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let mut opts = fs::OpenOptions::new();
        configure(&mut opts);
        opts.open(path)
    }
}

/// Read a secret without following a planted symlink/reparse and without
/// slurping an attacker-replaced huge file. Write already uses O_NOFOLLOW.
pub fn read_secret_file(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = open_nofollow(path)?;
    read_secret_file_bounded(&mut file)
}

fn read_secret_file_bounded(file: &mut fs::File) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    file.take(MAX_SECRET_FILE_BYTES + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > MAX_SECRET_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "secret file exceeds maximum size",
        ));
    }
    Ok(buf)
}

#[cfg(unix)]
fn secret_staging_path(path: &Path) -> PathBuf {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("secret");
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    parent.join(format!(".{stem}.{}.{n}.tmp", std::process::id()))
}

pub fn write_secret_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // Stage a new 0o600 inode, then rename over the destination so a
        // concurrent Tauri `read_auth_token` never observes the empty window
        // of in-place truncate. Path chmod after write followed a planted
        // symlink; fchmod the staging fd instead.
        let tmp = secret_staging_path(path);
        let staged = (|| {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&tmp)?;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(contents)?;
            file.flush()?;
            fs::rename(&tmp, path)
        })();
        if staged.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        staged
    }
    #[cfg(windows)]
    {
        use std::io::Write as _;
        // Name-surrogate reparse is the O_NOFOLLOW analog. Apply the owner
        // DACL before truncate so a SetNamedSecurityInfo failure cannot wipe
        // an existing token (Unix fchmod-before-set_len).
        let mut file = open_configured_nofollow(path, |opts| {
            opts.create(true).write(true).truncate(false);
        })?;
        restrict_file_to_owner(path)?;
        file.set_len(0)?;
        file.write_all(contents)?;
        file.flush()?;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        use std::io::Write as _;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        restrict_file_to_owner(path)
            .and_then(|_| file.write_all(contents))
            .and_then(|_| file.flush())
    }
}
