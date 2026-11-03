use harness_contracts::{AppError, AppResult};
fn target(reference: &str) -> AppResult<String> {
    if reference.is_empty() || reference.len() > 1024 || reference.contains('\0') {
        return Err(AppError::new(
            "INVALID_CREDENTIAL",
            "Invalid credential reference",
        ));
    }
    Ok(format!("RustHarness/{reference}"))
}
pub fn set_secret(reference: &str, value: &str) -> AppResult<()> {
    let target = target(reference)?;
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Security::Credentials::*;
        let mut name: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
        let mut bytes = value.as_bytes().to_vec();
        if bytes.len() > 2560 {
            return Err(AppError::new(
                "INVALID_CREDENTIAL",
                "Credential is too large",
            ));
        }
        let mut credential: CREDENTIALW = std::mem::zeroed();
        credential.Type = CRED_TYPE_GENERIC;
        credential.TargetName = name.as_mut_ptr();
        credential.CredentialBlob = bytes.as_mut_ptr();
        credential.CredentialBlobSize = bytes.len() as u32;
        credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
        let result = CredWriteW(&credential, 0);
        bytes.fill(0);
        if result == 0 {
            return Err(crate::io(std::io::Error::last_os_error()));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (target, value);
        Err(AppError::new(
            "UNSUPPORTED",
            "Secure credential store unavailable",
        ))
    }
}
pub fn get_secret(reference: &str) -> AppResult<Option<String>> {
    let target = target(reference)?;
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Security::Credentials::*;
        let name: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
        let mut credential = std::ptr::null_mut();
        if CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) == 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(1168) {
                return Ok(None);
            }
            return Err(crate::io(e));
        }
        let bytes = if (*credential).CredentialBlobSize == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(
                (*credential).CredentialBlob,
                (*credential).CredentialBlobSize as usize,
            )
            .to_vec()
        };
        let value = String::from_utf8(bytes).map_err(crate::io);
        CredFree(credential.cast());
        value.map(Some)
    }
    #[cfg(not(windows))]
    {
        let _ = target;
        Err(AppError::new(
            "UNSUPPORTED",
            "Secure credential store unavailable",
        ))
    }
}
pub fn delete_secret(reference: &str) -> AppResult<()> {
    let target = target(reference)?;
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Security::Credentials::*;
        let name: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
        if CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) == 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(1168) {
                return Err(crate::io(e));
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = target;
        Err(AppError::new(
            "UNSUPPORTED",
            "Secure credential store unavailable",
        ))
    }
}
