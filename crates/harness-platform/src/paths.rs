use crate::io;
use harness_contracts::{AppError, AppResult};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(crate) fn validate(relative: &str) -> AppResult<()> {
    if relative.is_empty()
        || relative.starts_with(['/', '\\'])
        || relative.contains([':', '\0', '*', '?', '"', '<', '>', '|'])
        || relative.chars().any(|c| c.is_control())
    {
        return Err(AppError::new(
            "INVALID_PATH",
            "Only relative paths without streams are allowed",
        ));
    }
    for part in relative.split(['/', '\\']) {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part.eq_ignore_ascii_case(".git")
            || ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(AppError::new("INVALID_PATH", relative));
        }
    }
    Ok(())
}
pub(crate) fn check(path: &Path) -> AppResult<()> {
    let meta = fs::symlink_metadata(path).map_err(io)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(AppError::new("UNSAFE_LINK", path.display().to_string()));
        }
    }
    if meta.file_type().is_symlink() {
        return Err(AppError::new("UNSAFE_LINK", path.display().to_string()));
    }
    if !meta.is_dir() && !meta.is_file() {
        return Err(AppError::new("UNSAFE_FILE", path.display().to_string()));
    }
    if meta.is_file() {
        let _ = open_read(path)?;
    }
    Ok(())
}
pub(crate) fn open_read(path: &Path) -> AppResult<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path).map_err(io)?;
    #[cfg(windows)]
    unsafe {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut info = std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>();
        if GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0 {
            return Err(io(std::io::Error::last_os_error()));
        }
        if info.dwFileAttributes & 0x400 != 0 || info.nNumberOfLinks > 1 {
            return Err(AppError::new("UNSAFE_LINK", path.display().to_string()));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata().map_err(io)?.nlink() > 1 {
            return Err(AppError::new("UNSAFE_LINK", path.display().to_string()));
        }
    }
    Ok(file)
}
/// Holds ancestor directories without FILE_SHARE_DELETE on Windows.
pub(crate) struct PathGuard {
    pub path: PathBuf,
    _dirs: Vec<File>,
}
pub(crate) fn guarded(root: &Path, relative: &str, must_exist: bool) -> AppResult<PathGuard> {
    validate(relative)?;
    check(root)?;
    let root = dunce::canonicalize(root).map_err(io)?;
    check(&root)?;
    let mut path = root.clone();
    let mut dirs = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        dirs.push(
            OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02200000)
                .open(&root)
                .map_err(io)?,
        );
        check(&root)?;
    }
    for part in relative.split(['/', '\\']) {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                check(&path)?;
                #[cfg(windows)]
                if meta.is_dir() {
                    use std::os::windows::fs::OpenOptionsExt;
                    dirs.push(
                        OpenOptions::new()
                            .read(true)
                            .share_mode(3)
                            .custom_flags(0x02200000)
                            .open(&path)
                            .map_err(io)?,
                    );
                    check(&path)?;
                }
                let real = dunce::canonicalize(&path).map_err(io)?;
                if !real.starts_with(&root) {
                    return Err(AppError::new("INVALID_PATH", "Path escapes root"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !must_exist => {}
            Err(e) => return Err(io(e)),
        }
    }
    Ok(PathGuard { path, _dirs: dirs })
}
pub fn resolve_path(root: &Path, relative: &str, must_exist: bool) -> AppResult<PathBuf> {
    Ok(guarded(root, relative, must_exist)?.path)
}
pub fn read_file(root: &Path, relative: &str) -> AppResult<String> {
    let guard = guarded(root, relative, true)?;
    let mut text = String::new();
    open_read(&guard.path)?
        .read_to_string(&mut text)
        .map_err(io)?;
    Ok(text)
}
/// Bounded human preview using the same guarded handle as full replica reads.
pub fn read_file_bounded(root: &Path, relative: &str, max_bytes: usize) -> AppResult<String> {
    let (text, _, _) = read_file_bounded_sized(root, relative, max_bytes)?;
    Ok(text)
}
/// 有界读取并附带文件完整字节数与实际读取字节数（用于截断判定）。
pub fn read_file_bounded_sized(
    root: &Path,
    relative: &str,
    max_bytes: usize,
) -> AppResult<(String, u64, u64)> {
    let guard = guarded(root, relative, true)?;
    let size = fs::metadata(&guard.path).map_err(io)?.len();
    let mut bytes = Vec::new();
    let read = open_read(&guard.path)?
        .take(max_bytes.min(1 << 20) as u64)
        .read_to_end(&mut bytes)
        .map_err(io)? as u64;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), size, read))
}
pub(crate) fn bytes(root: &Path, relative: &str) -> AppResult<Vec<u8>> {
    let guard = guarded(root, relative, true)?;
    let mut data = Vec::new();
    open_read(&guard.path)?.read_to_end(&mut data).map_err(io)?;
    Ok(data)
}
pub(crate) fn replace(root: &Path, relative: &str, data: &[u8]) -> AppResult<()> {
    validate(relative)?;
    let parent_rel = relative.rsplit_once(['/', '\\']).map(|x| x.0);
    if let Some(parent) = parent_rel {
        let mut current = String::new();
        for part in parent.split(['/', '\\']) {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(part);
            let guard = guarded(root, &current, false)?;
            if !guard.path.exists() {
                fs::create_dir(&guard.path).map_err(io)?;
            }
        }
    }
    let guard = guarded(root, relative, false)?;
    let parent = guard
        .path
        .parent()
        .ok_or_else(|| AppError::new("INVALID_PATH", relative))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(io)?;
    temp.write_all(data).map_err(io)?;
    temp.as_file().sync_all().map_err(io)?;
    temp.persist(&guard.path).map_err(io)?;
    Ok(())
}
pub fn write_file(root: &Path, relative: &str, text: &str) -> AppResult<()> {
    replace(root, relative, text.as_bytes())
}
pub fn list_files(root: &Path) -> AppResult<Vec<String>> {
    check(root)?;
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    for item in walker {
        let entry = item.map_err(io)?;
        if entry.depth() == 0 {
            continue;
        }
        check(entry.path())?;
        if entry.file_type().is_some_and(|t| t.is_file()) {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(io)?
                .to_str()
                .ok_or_else(|| AppError::new("INVALID_PATH", "Non UTF8 path"))?
                .replace('\\', "/");
            validate(&relative)?;
            out.push(relative);
        }
    }
    out.sort();
    Ok(out)
}
