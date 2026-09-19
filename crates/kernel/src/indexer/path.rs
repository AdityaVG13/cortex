use crate::auth;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(super) fn source_unavailable(err: io::Error) -> String {
    format!("source_unavailable: {err}")
}

pub(super) fn is_symlink_open_error(err: &io::Error) -> bool {
    #[cfg(unix)]
    if err.raw_os_error() == Some(libc::ELOOP) {
        return true;
    }
    err.kind() == io::ErrorKind::InvalidInput
}

pub(super) fn lexical_absolute(path: &Path) -> io::Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    })
}

/// Path of the opened vnode. Intermediate symlinks are already resolved on
/// this fd; comparing it to a trusted root closes check-then-open follows.
pub(super) fn normalize_fd_path(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

pub(super) fn fd_path(file: &fs::File) -> io::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        let mut buf = [0u8; libc::PATH_MAX as usize];
        // SAFETY: F_GETPATH writes a NUL-terminated absolute path into `buf`.
        let rc = unsafe {
            libc::fcntl(
                file.as_raw_fd(),
                libc::F_GETPATH,
                buf.as_mut_ptr() as *mut libc::c_char,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let raw = std::str::from_utf8(&buf[..len])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "path not utf8"))?;
        Ok(normalize_fd_path(PathBuf::from(raw)))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::unix::io::AsRawFd;
        fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map(normalize_fd_path)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::Storage::FileSystem::{GetFinalPathNameByHandleW, VOLUME_NAME_DOS};
        let handle = file.as_raw_handle() as HANDLE;
        let mut buf = vec![0u16; 1024];
        // SAFETY: `handle` is a live `File`; `buf` is writable wide storage.
        let n = unsafe {
            GetFinalPathNameByHandleW(handle, buf.as_mut_ptr(), buf.len() as u32, VOLUME_NAME_DOS)
        };
        if n == 0 || (n as usize) > buf.len() {
            return Err(io::Error::last_os_error());
        }
        let raw = String::from_utf16(&buf[..n as usize])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "path not utf16"))?;
        Ok(normalize_fd_path(PathBuf::from(raw)))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "fd path unavailable",
        ))
    }
}

pub(super) fn relative_has_symlink(root: &Path, path: &Path) -> Result<bool, String> {
    let Ok(rel) = path.strip_prefix(root) else {
        return Ok(false);
    };
    let mut cur = root.to_path_buf();
    for component in rel.components() {
        cur.push(component);
        match fs::symlink_metadata(&cur) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err.to_string()),
            Ok(meta) if meta.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
        }
    }
    Ok(false)
}

pub(super) fn opened_under_root(file: &fs::File, root: &Path) -> Result<bool, String> {
    let root = root.canonicalize().map_err(|err| err.to_string())?;
    // Unknown fd path: fail closed. Returning true here would let a planted
    // intermediate alias (home/.claude -> secret) pass after O_NOFOLLOW
    // follows that directory.
    let Ok(actual) = fd_path(file) else {
        return Ok(false);
    };
    let actual = actual.canonicalize().unwrap_or(actual);
    Ok(actual.starts_with(&root))
}

pub(super) fn capture_key_path(
    file: &fs::File,
    path: &Path,
    follow: bool,
    confine: bool,
) -> Result<PathBuf, String> {
    if follow {
        return path.canonicalize().map_err(source_unavailable);
    }
    let expected = lexical_absolute(path).map_err(source_unavailable)?;
    let actual = fd_path(file).map_err(|_| "source_symlink_requires_explicit_path".to_string())?;
    let actual = actual.canonicalize().unwrap_or(actual);
    let expected = expected.canonicalize().unwrap_or(expected);
    if confine {
        return Ok(actual);
    }
    if actual != expected {
        return Err("source_symlink_requires_explicit_path".into());
    }
    Ok(expected)
}

pub(super) fn open_capture(path: &Path, follow: bool) -> Result<(fs::File, PathBuf), String> {
    match auth::open_nofollow(path) {
        Ok(file) => Ok((file, path.to_path_buf())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            if follow {
                Err(source_unavailable(err))
            } else {
                Err("source_not_found".into())
            }
        }
        Err(err) if is_symlink_open_error(&err) => {
            if !follow {
                // Last-component alias: skip automatic intake instead of
                // aborting index_all with source_unavailable.
                return Err("source_symlink_requires_explicit_path".into());
            }
            let target = fs::read_link(path).map_err(source_unavailable)?;
            let resolved = if target.is_absolute() {
                target
            } else {
                path.parent().unwrap_or_else(|| Path::new(".")).join(target)
            };
            let file = auth::open_nofollow(&resolved).map_err(source_unavailable)?;
            Ok((file, resolved))
        }
        Err(err) => Err(source_unavailable(err)),
    }
}
