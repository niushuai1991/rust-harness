use crate::manifest::fail;
use harness_contracts::AppResult;
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
const MAX_FILE: u64 = 32 << 20;
const MAX_TOTAL: u64 = 128 << 20;
const MAX_ENTRIES: usize = 2048;
/// Cross-platform validation also rejects names dangerous on Windows when installed on Unix.
pub(crate) fn relative(name: &str) -> AppResult<PathBuf> {
    if name.is_empty()
        || name.contains('\\')
        || name.contains(':')
        || name.starts_with('/')
        || name.contains('\0')
    {
        return Err(fail("unsafe package path"));
    }
    for part in name.trim_end_matches('/').split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with([' ', '.'])
            || part.chars().any(|c| c.is_control())
            || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(fail("unsafe package component"));
        }
    }
    Ok(PathBuf::from(name.trim_end_matches('/')))
}
#[cfg(windows)]
fn linked(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn linked(m: &fs::Metadata) -> bool {
    m.file_type().is_symlink()
}
pub(crate) fn unpack(source: &Path, dest: &Path) -> AppResult<()> {
    let meta = fs::symlink_metadata(source).map_err(fail)?;
    if linked(&meta) {
        return Err(fail("linked package rejected"));
    }
    let mut names = HashSet::new();
    let mut total = 0;
    if meta.is_dir() {
        copy_dir(source, source, dest, &mut names, &mut total)?;
    } else {
        let mut archive =
            zip::ZipArchive::new(fs::File::open(source).map_err(fail)?).map_err(fail)?;
        if archive.len() > MAX_ENTRIES {
            return Err(fail("too many archive entries"));
        }
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(fail)?;
            let rel = relative(entry.name())?;
            if !names.insert(rel.to_string_lossy().to_lowercase()) {
                return Err(fail("case duplicate in package"));
            }
            if let Some(mode) = entry.unix_mode() {
                let kind = mode & 0o170000;
                if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                    return Err(fail("archive special file rejected"));
                }
            }
            let size = entry.size();
            total = total
                .checked_add(size)
                .ok_or_else(|| fail("archive size overflow"))?;
            if size > MAX_FILE
                || total > MAX_TOTAL
                || (size > 1 << 20 && size / entry.compressed_size().max(1) > 200)
            {
                return Err(fail("archive size/ratio exceeded"));
            }
            let out = dest.join(rel);
            if entry.is_dir() {
                fs::create_dir_all(&out).map_err(fail)?;
            } else {
                fs::create_dir_all(out.parent().unwrap()).map_err(fail)?;
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(out)
                    .map_err(fail)?;
                let n =
                    std::io::copy(&mut entry.by_ref().take(MAX_FILE + 1), &mut f).map_err(fail)?;
                if n != size || n > MAX_FILE {
                    return Err(fail("archive size mismatch"));
                }
                f.flush().map_err(fail)?;
            }
        }
    }
    Ok(())
}
fn copy_dir(
    base: &Path,
    src: &Path,
    dest: &Path,
    names: &mut HashSet<String>,
    total: &mut u64,
) -> AppResult<()> {
    for item in fs::read_dir(src).map_err(fail)? {
        let p = item.map_err(fail)?.path();
        let m = fs::symlink_metadata(&p).map_err(fail)?;
        if linked(&m) {
            return Err(fail("package links rejected"));
        }
        let name = p
            .strip_prefix(base)
            .map_err(fail)?
            .to_str()
            .ok_or_else(|| fail("non UTF8 path"))?
            .replace('\\', "/");
        let rel = relative(&name)?;
        if !names.insert(name.to_lowercase()) || names.len() > MAX_ENTRIES {
            return Err(fail("package duplicate/entry limit"));
        }
        if m.is_dir() {
            fs::create_dir_all(dest.join(&rel)).map_err(fail)?;
            copy_dir(base, &p, dest, names, total)?;
        } else if m.is_file() {
            *total = total
                .checked_add(m.len())
                .ok_or_else(|| fail("package size overflow"))?;
            if m.len() > MAX_FILE || *total > MAX_TOTAL {
                return Err(fail("package size limit"));
            }
            fs::create_dir_all(dest.join(&rel).parent().unwrap()).map_err(fail)?;
            let mut input = fs::File::open(&p).map_err(fail)?;
            let mut out = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dest.join(rel))
                .map_err(fail)?;
            let copied = std::io::copy(&mut Read::by_ref(&mut input).take(MAX_FILE + 1), &mut out)
                .map_err(fail)?;
            if copied != m.len() {
                return Err(fail("package changed during installation"));
            }
        } else {
            return Err(fail("special file rejected"));
        }
    }
    Ok(())
}
