use super::{BudgetConfigError, MAX_BUDGET_FILE_BYTES};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

pub(super) fn open_nofollow(path: &Path) -> io::Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }
    #[cfg(windows)]
    {
        open_windows_rejecting_name_surrogate(path, |opts| {
            opts.read(true);
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        fs::File::open(path)
    }
}

#[cfg(windows)]
fn open_windows_rejecting_name_surrogate(
    path: &Path,
    configure: impl Fn(&mut fs::OpenOptions),
) -> io::Result<fs::File> {
    use std::os::windows::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let mut inspect = fs::OpenOptions::new();
    configure(&mut inspect);
    let file = inspect
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let meta = file.metadata()?;
    let file_type = meta.file_type();
    if file_type.is_symlink() || file_type.is_symlink_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to follow a name-surrogate reparse point",
        ));
    }
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
        return Ok(file);
    }
    drop(file);
    let mut follow = fs::OpenOptions::new();
    configure(&mut follow);
    follow.open(path)
}

pub(super) fn io_error(error: std::io::Error) -> BudgetConfigError {
    BudgetConfigError::new(
        "io_error",
        format!("failed to read budgets.toml: {error}"),
        None,
        None,
    )
}

pub(super) fn read_budget_file(path: &Path) -> Result<Option<String>, BudgetConfigError> {
    let file = match open_nofollow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    let mut contents = String::new();
    file.take(MAX_BUDGET_FILE_BYTES + 1)
        .read_to_string(&mut contents)
        .map_err(io_error)?;
    if contents.len() as u64 > MAX_BUDGET_FILE_BYTES {
        return Err(BudgetConfigError::new(
            "too_large",
            format!("budgets.toml exceeds {MAX_BUDGET_FILE_BYTES} bytes"),
            None,
            None,
        ));
    }
    Ok(Some(contents))
}
