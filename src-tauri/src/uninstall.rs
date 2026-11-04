//! Explicit, local-only uninstall cleanup. Never follows configured workspace/data paths.
use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(windows)]
mod windows {
    use std::{ffi::c_void, os::windows::ffi::OsStringExt, path::PathBuf};
    #[repr(C)]
    pub(super) struct Guid {
        a: u32,
        b: u16,
        c: u16,
        d: [u8; 8],
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            id: *const Guid,
            flags: u32,
            token: *mut c_void,
            path: *mut *mut u16,
        ) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(value: *mut c_void);
        fn CoCreateGuid(value: *mut Guid) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileInformationByHandle(
            handle: *mut c_void,
            class: i32,
            info: *const c_void,
            size: u32,
        ) -> i32;
    }
    pub(super) fn delete_lock_on_close(file: &std::fs::File) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        let delete: u8 = 1; // FILE_DISPOSITION_INFO.DeleteFile is a Windows BOOLEAN.
        let success = unsafe {
            SetFileInformationByHandle(file.as_raw_handle(), 4, (&delete as *const u8).cast(), 1)
        };
        if success == 0 {
            return Err(format!(
                "Mark idle host.lock for removal: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    pub(super) const LOCAL: Guid = Guid {
        a: 0xf1b32785,
        b: 0x6fba,
        c: 0x4fcf,
        d: [0x9d, 0x55, 0x7b, 0x8e, 0x7f, 0x15, 0x70, 0x91],
    };
    pub(super) const ROAMING: Guid = Guid {
        a: 0x3eb685db,
        b: 0x65f9,
        c: 0x4cf6,
        d: [0xa0, 0x3a, 0xe3, 0xef, 0x65, 0x72, 0x9f, 0x3d],
    };
    pub(super) fn known_folder(id: &Guid) -> Result<PathBuf, String> {
        let mut pointer = std::ptr::null_mut();
        let result = unsafe { SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut pointer) };
        if result < 0 {
            return Err(format!("KnownFolder lookup failed: {result:#x}"));
        }
        if pointer.is_null() {
            return Err("KnownFolder returned an empty path".into());
        }
        let mut length = 0;
        unsafe {
            while *pointer.add(length) != 0 {
                length += 1;
            }
            let value = std::ffi::OsString::from_wide(std::slice::from_raw_parts(pointer, length));
            CoTaskMemFree(pointer.cast());
            Ok(PathBuf::from(value))
        }
    }
    pub(super) fn nonce() -> Result<String, String> {
        let mut id = Guid {
            a: 0,
            b: 0,
            c: 0,
            d: [0; 8],
        };
        let result = unsafe { CoCreateGuid(&mut id) };
        if result < 0 {
            return Err(format!("Cleanup staging GUID failed: {result:#x}"));
        }
        Ok(format!(
            "{:08x}{:04x}{:04x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            id.a,
            id.b,
            id.c,
            id.d[0],
            id.d[1],
            id.d[2],
            id.d[3],
            id.d[4],
            id.d[5],
            id.d[6],
            id.d[7]
        ))
    }
}

/// Purge only this Windows user's three fixed application-owned data roots.
/// Does not consult environment overrides, settings, or project paths.
pub fn purge_local_data() -> Result<(), String> {
    #[cfg(windows)]
    {
        purge_roots(
            &windows::known_folder(&windows::LOCAL)?,
            &windows::known_folder(&windows::ROAMING)?,
        )
    }
    #[cfg(not(windows))]
    {
        Err("Local uninstall cleanup is available only on Windows".into())
    }
}

#[cfg(windows)]
fn reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(windows)]
fn checked_parent(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("KnownFolder path must be absolute".into());
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|e| format!("Inspect KnownFolder {}: {e}", ancestor.display()))?;
        if reparse(&metadata) || !metadata.is_dir() {
            return Err(format!(
                "Refusing redirected KnownFolder: {}",
                ancestor.display()
            ));
        }
    }
    fs::canonicalize(path).map_err(|e| format!("Canonicalize KnownFolder: {e}"))
}

#[cfg(windows)]
fn purge_roots(local: &Path, roaming: &Path) -> Result<(), String> {
    let local = checked_parent(local)?;
    let roaming = checked_parent(roaming)?;
    let roots = [
        local.join("RustHarness"),
        local.join("dev.rustharness.desktop"),
        roaming.join("dev.rustharness.desktop"),
    ];
    let mut existing = Vec::new();
    // Inspect every root before any mutation, including roots that have no active host lock.
    for root in &roots {
        match fs::symlink_metadata(root) {
            Ok(metadata) if reparse(&metadata) || !metadata.is_dir() => {
                return Err(format!(
                    "Refusing reparse or non-directory data root: {}",
                    root.display()
                ));
            }
            Ok(_) => existing.push(root.clone()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Inspect data root {}: {e}", root.display())),
        }
    }
    let host_lock = if existing.contains(&roots[0]) {
        use std::os::windows::fs::OpenOptionsExt;
        let path = roots[0].join("host.lock");
        let exists = match fs::symlink_metadata(&path) {
            Ok(metadata) if reparse(&metadata) || !metadata.is_file() => {
                return Err("Refusing redirected or non-file host.lock".into());
            }
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(format!("Inspect host.lock: {e}")),
        };
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(!exists)
            .share_mode(0x0000_0001 | 0x0000_0002 | 0x0000_0004)
            .access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000) // GENERIC_READ | GENERIC_WRITE | DELETE
            .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT
            .open(&path)
            .map_err(|e| format!("Open host.lock: {e}"))?;
        if reparse(&file.metadata().map_err(|e| e.to_string())?) {
            return Err("Refusing redirected host.lock handle".into());
        }
        file.try_lock().map_err(|e| {
            format!("Application data is busy; close Rust Harness before deleting data: {e}")
        })?;
        Some(file)
    } else {
        None
    };
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut runtime_stage: Option<PathBuf> = None;
    let stage_result = (|| -> Result<(), String> {
        for root in existing {
            // Staging is an unpredictable sibling on the same KnownFolder volume.
            let stage = root
                .parent()
                .ok_or("Missing fixed data parent")?
                .join(format!(".rust-harness-uninstall-{}", windows::nonce()?));
            if fs::symlink_metadata(&stage).is_ok() {
                return Err("Cleanup staging collision".into());
            }
            if root == roots[0] {
                // Windows cannot rename a directory containing an open host.lock handle.
                // Stage its children instead; retain the original lock path throughout removal.
                fs::create_dir(&stage).map_err(|e| format!("Create cleanup staging: {e}"))?;
                runtime_stage = Some(stage.clone());
                for entry in fs::read_dir(&root).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    if entry.file_name() == "host.lock" {
                        continue;
                    }
                    let destination = stage.join(entry.file_name());
                    fs::rename(entry.path(), &destination).map_err(|e| {
                        format!("Stage runtime data {}: {e}", entry.path().display())
                    })?;
                    staged.push((entry.path(), destination));
                }
            } else {
                fs::rename(&root, &stage)
                    .map_err(|e| format!("Stage data {}: {e}", root.display()))?;
                staged.push((root, stage.clone()));
                let metadata = fs::symlink_metadata(&stage).map_err(|e| e.to_string())?;
                if reparse(&metadata) || !metadata.is_dir() {
                    return Err("Data root changed during cleanup staging".into());
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = stage_result {
        let mut rollback_errors = Vec::new();
        for (original, stage) in staged.iter().rev() {
            // Never replace a new root created by another process during staging.
            if fs::symlink_metadata(original).is_ok() {
                rollback_errors.push(format!(
                    "Original root recreated; data retained at {}",
                    stage.display()
                ));
            } else if let Err(e) = fs::rename(stage, original) {
                rollback_errors.push(format!("Data retained at {}: {e}", stage.display()));
            }
        }
        if let Some(stage) = runtime_stage {
            let _ = fs::remove_dir(stage);
        }
        return Err(format!("{error}; rollback: {}", rollback_errors.join("; ")));
    }
    for (_, stage) in &staged {
        // std::fs explicitly does not follow symbolic links; Windows uses handle-based removal.
        let metadata = fs::symlink_metadata(stage).map_err(|e| e.to_string())?;
        use std::os::windows::fs::MetadataExt;
        if reparse(&metadata) && metadata.file_attributes() & 0x10 != 0 {
            fs::remove_dir(stage)
                .map_err(|e| format!("Remove staged directory link {}: {e}", stage.display()))?;
        } else if metadata.is_dir() {
            fs::remove_dir_all(stage)
                .map_err(|e| format!("Remove staged app data {}: {e}", stage.display()))?;
        } else {
            fs::remove_file(stage)
                .map_err(|e| format!("Remove staged app data {}: {e}", stage.display()))?;
        }
    }
    if let Some(stage) = runtime_stage {
        fs::remove_dir(stage).map_err(|e| e.to_string())?;
    }
    // The active runtime has been excluded until every selected old-data entry is removed.
    // Only non-recursive removal follows releasing the original lock: never sweep new data.
    if let Some(lock) = host_lock {
        // Delete through the still-locked original handle, never a re-resolved parent path.
        // Marking delete-pending before close also prevents new opens during finalization.
        windows::delete_lock_on_close(&lock)?;
        drop(lock);
        fs::remove_dir(&roots[0])
            .map_err(|e| format!("Data root changed or is busy; new contents retained: {e}"))?;
    }
    Ok(())
}

#[cfg(all(test, windows))]
pub(crate) fn purge_test_roots(local: &Path, roaming: &Path) -> Result<(), String> {
    purge_roots(local, roaming)
}
