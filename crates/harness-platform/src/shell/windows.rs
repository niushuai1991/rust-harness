use super::*;
use crate::{WorkspaceManager, io};
use std::{
    ffi::c_void,
    fs::{self, File},
    io::Read,
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::PathBuf,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::{JobObjects::*, Pipes::*, Threading::*},
};
const SE_GROUP_LOGON_ID: u32 = 0xc0000000;
const SE_GROUP_INTEGRITY: u32 = 0x20;
fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
fn error(api: &str) -> AppError {
    AppError::new(
        "SANDBOX",
        format!("{api}: {}", std::io::Error::last_os_error()),
    )
}
struct Handle(HANDLE);
// Kernel handles are thread safe; this type owns and closes exactly one handle.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                CloseHandle(self.0);
            }
        }
    }
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                LocalFree(self.0);
            }
        }
    }
}
fn sid(value: &str) -> AppResult<Local> {
    unsafe {
        let mut ptr = null_mut();
        if ConvertStringSidToSidW(wide(value).as_ptr(), &mut ptr) == 0 {
            return Err(error("ConvertStringSidToSidW"));
        }
        Ok(Local(ptr))
    }
}
fn entry(sid: PSID, mode: ACCESS_MODE, mask: u32, inheritance: u32) -> EXPLICIT_ACCESS_W {
    EXPLICIT_ACCESS_W {
        grfAccessPermissions: mask,
        grfAccessMode: mode,
        grfInheritance: inheritance,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.cast(),
        },
    }
}
fn grant(root: &Path, capability: PSID, world: PSID) -> AppResult<()> {
    unsafe {
        // Every existing object is independently checked before inherited ACL propagation.
        fn inspect(path: &Path) -> AppResult<()> {
            crate::paths::check(path)?;
            if path.is_dir() {
                for item in fs::read_dir(path).map_err(io)? {
                    inspect(&item.map_err(io)?.path())?;
                }
            }
            Ok(())
        }
        inspect(root)?;
        let mut old = null_mut();
        let mut descriptor = null_mut();
        let name = wide(root);
        let result = GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut old,
            null_mut(),
            &mut descriptor,
        );
        if result != 0 {
            return Err(AppError::new(
                "SANDBOX",
                format!("GetNamedSecurityInfoW: {result}"),
            ));
        }
        let _descriptor = Local(descriptor);
        // A non-elevated owner can edit its DACL but does not implicitly have
        // WRITE_OWNER, which is required to lower a mandatory label. Grant the
        // host user FullControl only on this newly-created managed tree first.
        let mut host_token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut host_token) == 0 {
            return Err(error("OpenProcessToken owner"));
        }
        let host_token = Handle(host_token);
        let mut owner_size = 0;
        GetTokenInformation(host_token.0, TokenUser, null_mut(), 0, &mut owner_size);
        let mut owner = vec![0u64; (owner_size as usize).div_ceil(8)];
        if GetTokenInformation(
            host_token.0,
            TokenUser,
            owner.as_mut_ptr().cast(),
            owner_size,
            &mut owner_size,
        ) == 0
        {
            return Err(error("GetTokenInformation owner"));
        }
        let owner_sid = (*(owner.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        let entries = [
            entry(
                owner_sid,
                GRANT_ACCESS,
                FILE_ALL_ACCESS,
                SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            ),
            entry(world, DENY_ACCESS, FILE_DELETE_CHILD, CONTAINER_INHERIT_ACE),
            entry(
                capability,
                GRANT_ACCESS,
                FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE,
                SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            ),
            entry(
                capability,
                GRANT_ACCESS,
                DELETE,
                SUB_CONTAINERS_AND_OBJECTS_INHERIT | INHERIT_ONLY_ACE,
            ),
        ];
        let mut acl = null_mut();
        let result = SetEntriesInAclW(entries.len() as u32, entries.as_ptr(), old, &mut acl);
        if result != 0 {
            return Err(AppError::new(
                "SANDBOX",
                format!("SetEntriesInAclW: {result}"),
            ));
        }
        let _acl = Local(acl.cast());
        let result = SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        );
        if result != 0 {
            return Err(AppError::new(
                "SANDBOX",
                format!("SetNamedSecurityInfoW host owner: {result}"),
            ));
        }
        let mut low_descriptor = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide("S:(ML;OICI;NW;;;LW)").as_ptr(),
            1,
            &mut low_descriptor,
            null_mut(),
        ) == 0
        {
            return Err(error("ConvertStringSecurityDescriptor"));
        }
        let _low_descriptor = Local(low_descriptor);
        let mut sacl = null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        if GetSecurityDescriptorSacl(low_descriptor, &mut present, &mut sacl, &mut defaulted) == 0 {
            return Err(error("GetSecurityDescriptorSacl"));
        }
        let result = SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            sacl,
        );
        if result != 0 {
            return Err(AppError::new(
                "SANDBOX",
                format!("SetNamedSecurityInfoW: {result}"),
            ));
        }
        Ok(())
    }
}
fn token(capability: PSID, temp_capability: PSID, world: PSID) -> AppResult<Handle> {
    unsafe {
        let mut original = null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT,
            &mut original,
        ) == 0
        {
            return Err(error("OpenProcessToken"));
        }
        let original = Handle(original);
        let mut needed = 0;
        GetTokenInformation(original.0, TokenGroups, null_mut(), 0, &mut needed);
        if needed == 0 {
            return Err(error("GetTokenInformation(size)"));
        }
        let mut groups = vec![0u64; (needed as usize).div_ceil(8)];
        if GetTokenInformation(
            original.0,
            TokenGroups,
            groups.as_mut_ptr().cast(),
            needed,
            &mut needed,
        ) == 0
        {
            return Err(error("GetTokenInformation"));
        }
        let group_info = &*(groups.as_ptr().cast::<TOKEN_GROUPS>());
        let group_slice =
            std::slice::from_raw_parts(group_info.Groups.as_ptr(), group_info.GroupCount as usize);
        let logon = group_slice
            .iter()
            .find(|g| g.Attributes & SE_GROUP_LOGON_ID == SE_GROUP_LOGON_ID)
            .ok_or_else(|| AppError::new("SANDBOX", "No logon SID"))?;
        let restricting = [
            SID_AND_ATTRIBUTES {
                Sid: logon.Sid,
                Attributes: 0,
            },
            SID_AND_ATTRIBUTES {
                Sid: world,
                Attributes: 0,
            },
            SID_AND_ATTRIBUTES {
                Sid: capability,
                Attributes: 0,
            },
            SID_AND_ATTRIBUTES {
                Sid: temp_capability,
                Attributes: 0,
            },
        ];
        let mut restricted = null_mut();
        if CreateRestrictedToken(
            original.0,
            DISABLE_MAX_PRIVILEGE | WRITE_RESTRICTED,
            0,
            null(),
            0,
            null(),
            restricting.len() as u32,
            restricting.as_ptr(),
            &mut restricted,
        ) == 0
        {
            return Err(error("CreateRestrictedToken"));
        }
        let restricted = Handle(restricted);
        let low = sid("S-1-16-4096")?;
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: low.0,
                Attributes: SE_GROUP_INTEGRITY,
            },
        };
        if SetTokenInformation(
            restricted.0,
            TokenIntegrityLevel,
            (&label as *const TOKEN_MANDATORY_LABEL).cast(),
            (std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32) + GetLengthSid(low.0),
        ) == 0
        {
            return Err(error("SetTokenInformation Low IL"));
        }
        let mut size = 0;
        GetTokenInformation(restricted.0, TokenDefaultDacl, null_mut(), 0, &mut size);
        if size == 0 {
            return Err(error("TokenDefaultDacl size"));
        }
        let mut dacl_info = vec![0u64; (size as usize).div_ceil(8)];
        if GetTokenInformation(
            restricted.0,
            TokenDefaultDacl,
            dacl_info.as_mut_ptr().cast(),
            size,
            &mut size,
        ) == 0
        {
            return Err(error("TokenDefaultDacl"));
        }
        let old = (*(dacl_info.as_ptr().cast::<TOKEN_DEFAULT_DACL>())).DefaultDacl;
        if old.is_null() {
            return Err(AppError::new("SANDBOX", "Null default DACL"));
        }
        let mut acl = null_mut();
        let grant = entry(capability, GRANT_ACCESS, 0x1fffff, 0);
        let result = SetEntriesInAclW(1, &grant, old, &mut acl);
        if result != 0 {
            return Err(AppError::new(
                "SANDBOX",
                format!("Token default DACL: {result}"),
            ));
        }
        let _acl = Local(acl.cast());
        let dacl = TOKEN_DEFAULT_DACL { DefaultDacl: acl };
        if SetTokenInformation(
            restricted.0,
            TokenDefaultDacl,
            (&dacl as *const TOKEN_DEFAULT_DACL).cast(),
            std::mem::size_of::<TOKEN_DEFAULT_DACL>() as u32,
        ) == 0
        {
            return Err(error("SetTokenInformation DACL"));
        }
        Ok(restricted)
    }
}
fn pipe() -> AppResult<(File, Handle)> {
    unsafe {
        let mut read = null_mut();
        let mut write = null_mut();
        let security = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        if CreatePipe(&mut read, &mut write, &security, 0) == 0 {
            return Err(error("CreatePipe"));
        }
        let write = Handle(write);
        let read = File::from_raw_handle(read);
        use std::os::windows::io::AsRawHandle;
        if SetHandleInformation(read.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) == 0 {
            return Err(error("SetHandleInformation"));
        }
        Ok((read, write))
    }
}
fn read_output(mut file: File) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    let mut truncated = false;
    while let Ok(n) = file.read(&mut chunk) {
        if n == 0 {
            break;
        }
        let take = n.min((16 * 1024 * 1024usize).saturating_sub(bytes.len()));
        bytes.extend_from_slice(&chunk[..take]);
        truncated |= take < n;
    }
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str("\n[output truncated at 16MiB]");
    }
    text
}
struct Startup {
    info: STARTUPINFOEXW,
    _attributes: Vec<usize>,
    _handles: Vec<HANDLE>,
}
impl Drop for Startup {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.info.lpAttributeList);
        }
    }
}
fn startup(input: HANDLE, output: HANDLE, error_handle: HANDLE) -> AppResult<Startup> {
    unsafe {
        let mut size = 0;
        InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size);
        if size == 0 {
            return Err(error("InitializeProcThreadAttributeList size"));
        }
        let mut attributes = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        let list = attributes.as_mut_ptr().cast();
        if InitializeProcThreadAttributeList(list, 1, 0, &mut size) == 0 {
            return Err(error("InitializeProcThreadAttributeList"));
        }
        let mut info: STARTUPINFOEXW = std::mem::zeroed();
        info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        info.StartupInfo.hStdInput = input;
        info.StartupInfo.hStdOutput = output;
        info.StartupInfo.hStdError = error_handle;
        info.lpAttributeList = list;
        let handles = vec![input, output, error_handle];
        let startup = Startup {
            info,
            _attributes: attributes,
            _handles: handles,
        };
        if UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            startup._handles.as_ptr().cast(),
            startup._handles.len() * std::mem::size_of::<HANDLE>(),
            null_mut(),
            null(),
        ) == 0
        {
            return Err(error("UpdateProcThreadAttribute handle whitelist"));
        }
        Ok(startup)
    }
}
fn capability(id: &str) -> AppResult<Local> {
    let uuid = uuid::Uuid::parse_str(id).map_err(io)?;
    let b = uuid.as_bytes();
    sid(&format!(
        "S-1-4-{}-{}-{}-{}",
        u32::from_le_bytes(b[0..4].try_into().unwrap()),
        u32::from_le_bytes(b[4..8].try_into().unwrap()),
        u32::from_le_bytes(b[8..12].try_into().unwrap()),
        u32::from_le_bytes(b[12..16].try_into().unwrap())
    ))
}
pub(crate) fn prepare(root: &Path, id: &str) -> AppResult<()> {
    let capability = capability(id)?;
    let world = sid("S-1-1-0")?;
    grant(root, capability.0, world.0)
}
struct StdioLifetime {
    job: std::sync::Arc<Handle>,
    process: Handle,
    done: CancellationToken,
    _lease: crate::WorkspaceLease,
    _temp: tempfile::TempDir,
}
impl Drop for StdioLifetime {
    fn drop(&mut self) {
        unsafe {
            TerminateJobObject(self.job.0, 1);
            WaitForSingleObject(self.process.0, 5000);
        }
        self.done.cancel();
    }
}
fn quoted(value: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
        } else {
            out.extend(std::iter::repeat_n('\\', slashes));
        }
        slashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}
pub(super) fn spawn_stdio(
    root: &Path,
    program: &str,
    args: &[String],
    cancel: CancellationToken,
) -> AppResult<SandboxedStdio> {
    if cancel.is_cancelled() {
        return Err(AppError::new("CANCELLED", "Process cancelled"));
    }
    if program.contains('\0') || args.iter().any(|a| a.contains('\0')) {
        return Err(AppError::new("INVALID_ARGUMENT", "NUL in command"));
    }
    crate::paths::check(root)?;
    let root = dunce::canonicalize(root).map_err(io)?;
    let workspace = root
        .parent()
        .ok_or_else(|| AppError::new("INVALID_WORKSPACE", "Missing workspace"))?;
    let id = workspace
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| AppError::new("INVALID_WORKSPACE", "Missing id"))?;
    let data = workspace
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| AppError::new("INVALID_WORKSPACE", "Missing host directory"))?;
    let manager = WorkspaceManager::new(data.to_path_buf())?;
    let lease = manager.acquire(id)?;
    if Path::new(&lease.info.replica_path) != root {
        return Err(AppError::new("INVALID_WORKSPACE", "Root mismatch"));
    }
    let executable = if Path::new(program).is_absolute() {
        PathBuf::from(program)
    } else if !program.contains(['/', '\\']) {
        let filename = if program.to_ascii_lowercase().ends_with(".exe") {
            program.to_owned()
        } else {
            format!("{program}.exe")
        };
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .map(|p| p.join(&filename))
            .find(|p| p.is_file())
            .ok_or_else(|| AppError::new("PROCESS_NOT_FOUND", program))?
    } else {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "Use an absolute executable path or a PATH executable name",
        ));
    };
    // System executables commonly have WinSxS hardlinks. They are only read and
    // executed here: no ACL is applied and no bytes are written to this path.
    let executable = dunce::canonicalize(&executable).map_err(io)?;
    if !executable.is_file() {
        return Err(AppError::new("PROCESS_NOT_FOUND", program));
    }
    if executable
        .extension()
        .and_then(|x| x.to_str())
        .is_none_or(|x| !x.eq_ignore_ascii_case("exe"))
    {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "stdio requires an executable (.exe)",
        ));
    }
    fs::create_dir_all(workspace.join("temp")).map_err(io)?;
    let temp = tempfile::tempdir_in(workspace.join("temp")).map_err(io)?;
    let capability = capability(id)?;
    let temp_capability = self::capability(&uuid::Uuid::new_v4().to_string())?;
    let world = sid("S-1-1-0")?;
    grant(temp.path(), temp_capability.0, world.0)?;
    let token = token(capability.0, temp_capability.0, world.0)?;
    unsafe {
        let job = std::sync::Arc::new(Handle(CreateJobObjectW(null(), null())));
        if job.0.is_null() {
            return Err(error("CreateJobObjectW"));
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        ) == 0
        {
            return Err(error("SetInformationJobObject"));
        }
        let (stdout, out_write) = pipe()?;
        let (stderr, err_write) = pipe()?;
        let mut in_read = null_mut();
        let mut in_write = null_mut();
        let security = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        if CreatePipe(&mut in_read, &mut in_write, &security, 0) == 0 {
            return Err(error("CreatePipe stdin"));
        }
        let in_read = Handle(in_read);
        let in_write = File::from_raw_handle(in_write);
        use std::os::windows::io::AsRawHandle;
        if SetHandleInformation(in_write.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) == 0 {
            return Err(error("SetHandleInformation stdin"));
        }
        let startup = startup(in_read.0, out_write.0, err_write.0)?;
        let mut command = quoted(&executable.to_string_lossy());
        for argument in args {
            command.push(' ');
            command.push_str(&quoted(argument));
        }
        let mut command = wide(command);
        let mut environment: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| {
                !["TEMP", "TMP", "TMPDIR"]
                    .iter()
                    .any(|v| k.eq_ignore_ascii_case(v))
            })
            .collect();
        for key in ["TEMP", "TMP", "TMPDIR"] {
            environment.push((key.into(), temp.path().to_string_lossy().into_owned()));
        }
        environment.sort_by_key(|(k, _)| k.to_ascii_uppercase());
        let mut env: Vec<u16> = environment
            .into_iter()
            .flat_map(|(k, v)| wide(format!("{k}={v}")))
            .collect();
        env.push(0);
        let mut process: PROCESS_INFORMATION = std::mem::zeroed();
        if CreateProcessAsUserW(
            token.0,
            wide(executable).as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_SUSPENDED
                | CREATE_NO_WINDOW
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT,
            env.as_ptr().cast(),
            wide(root).as_ptr(),
            &startup.info.StartupInfo,
            &mut process,
        ) == 0
        {
            return Err(error("CreateProcessAsUserW"));
        }
        let process_handle = Handle(process.hProcess);
        let thread = Handle(process.hThread);
        if AssignProcessToJobObject(job.0, process_handle.0) == 0 {
            TerminateProcess(process_handle.0, 1);
            return Err(error("AssignProcessToJobObject"));
        }
        if ResumeThread(thread.0) == u32::MAX {
            TerminateJobObject(job.0, 1);
            return Err(error("ResumeThread"));
        }
        drop(in_read);
        drop(out_write);
        drop(err_write);
        let stderr_buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let buffer = stderr_buffer.clone();
        std::thread::spawn(move || {
            let mut stderr = stderr;
            let mut chunk = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                if let Ok(mut bytes) = buffer.lock() {
                    let take = n.min(65536usize.saturating_sub(bytes.len()));
                    bytes.extend_from_slice(&chunk[..take]);
                }
            }
        });
        let done = CancellationToken::new();
        let lifecycle = StdioLifetime {
            job: job.clone(),
            process: process_handle,
            done: done.clone(),
            _lease: lease,
            _temp: temp,
        };
        tokio::spawn(async move {
            tokio::select! { _ = cancel.cancelled() => { TerminateJobObject(job.0, 1); }, _ = done.cancelled() => {} }
        });
        Ok(SandboxedStdio {
            reader: tokio::fs::File::from_std(stdout),
            writer: tokio::fs::File::from_std(in_write),
            stderr: stderr_buffer,
            _guard: Box::new(lifecycle),
        })
    }
}
pub(super) fn run(
    root: &Path,
    command: &str,
    cancel: CancellationToken,
) -> AppResult<ProcessOutput> {
    if cancel.is_cancelled() {
        return Err(AppError::new("CANCELLED", "Shell cancelled"));
    }
    crate::paths::check(root)?;
    let root = dunce::canonicalize(root).map_err(io)?;
    if root.file_name().and_then(|s| s.to_str()) != Some("root") {
        return Err(AppError::new(
            "INVALID_WORKSPACE",
            "Shell requires a managed root",
        ));
    }
    let workspace = root
        .parent()
        .ok_or_else(|| AppError::new("INVALID_WORKSPACE", "Missing workspace"))?;
    let id = workspace
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| AppError::new("INVALID_WORKSPACE", "Missing id"))?;
    let data = workspace
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| AppError::new("INVALID_WORKSPACE", "Missing data directory"))?;
    let manager = WorkspaceManager::new(data.to_path_buf())?;
    let _lease = manager.acquire(id)?;
    if Path::new(&_lease.info.replica_path) != root {
        return Err(AppError::new("INVALID_WORKSPACE", "Root mismatch"));
    }
    fs::create_dir_all(workspace.join("temp")).map_err(io)?;
    let temp = tempfile::tempdir_in(workspace.join("temp")).map_err(io)?;
    let capability = capability(id)?;
    let temp_capability = self::capability(&uuid::Uuid::new_v4().to_string())?;
    let world = sid("S-1-1-0")?;
    grant(temp.path(), temp_capability.0, world.0)?;
    let token = token(capability.0, temp_capability.0, world.0)?;
    unsafe {
        let job = Handle(CreateJobObjectW(null(), null()));
        if job.0.is_null() {
            return Err(error("CreateJobObjectW"));
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        ) == 0
        {
            return Err(error("SetInformationJobObject"));
        }
        let (stdout, out_write) = pipe()?;
        let (stderr, err_write) = pipe()?;
        let system = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .ok_or_else(|| AppError::new("SANDBOX", "SystemRoot missing"))?;
        let pwsh = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:/Program Files"))
            .join("PowerShell/7/pwsh.exe");
        let executable = if pwsh.is_file() {
            pwsh
        } else {
            system.join("System32/WindowsPowerShell/v1.0/powershell.exe")
        };
        // EncodedCommand avoids quoting/injection through the process command line.
        let script_path = temp.path().join("command.ps1");
        fs::write(
            &script_path,
            format!(
                "\u{feff}$ErrorActionPreference='Stop'; Set-Location -LiteralPath '{}';\n{}",
                root.to_string_lossy().replace('\'', "''"),
                command
            ),
        )
        .map_err(io)?;
        let mut command_line = wide(format!(
            "\"{}\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\"",
            executable.display(),
            script_path.display()
        ));
        let mut environment: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| {
                !["TEMP", "TMP", "TMPDIR"]
                    .iter()
                    .any(|v| k.eq_ignore_ascii_case(v))
            })
            .collect();
        for key in ["TEMP", "TMP", "TMPDIR"] {
            environment.push((key.into(), temp.path().to_string_lossy().into_owned()));
        }
        environment.sort_by_key(|(k, _)| k.to_ascii_uppercase());
        let mut env: Vec<u16> = environment
            .into_iter()
            .flat_map(|(k, v)| wide(format!("{k}={v}")))
            .collect();
        env.push(0);
        let mut in_read = null_mut();
        let mut in_write = null_mut();
        let security = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        if CreatePipe(&mut in_read, &mut in_write, &security, 0) == 0 {
            return Err(error("CreatePipe empty stdin"));
        }
        let in_read = Handle(in_read);
        let in_write = Handle(in_write);
        let startup = startup(in_read.0, out_write.0, err_write.0)?;
        let mut process: PROCESS_INFORMATION = std::mem::zeroed();
        if CreateProcessAsUserW(
            token.0,
            wide(&executable).as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_SUSPENDED
                | CREATE_NO_WINDOW
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT,
            env.as_ptr().cast(),
            wide(&root).as_ptr(),
            &startup.info.StartupInfo,
            &mut process,
        ) == 0
        {
            return Err(error("CreateProcessAsUserW"));
        }
        let process_handle = Handle(process.hProcess);
        let thread = Handle(process.hThread);
        if AssignProcessToJobObject(job.0, process_handle.0) == 0 {
            TerminateProcess(process_handle.0, 1);
            return Err(error("AssignProcessToJobObject"));
        }
        if ResumeThread(thread.0) == u32::MAX {
            TerminateJobObject(job.0, 1);
            return Err(error("ResumeThread"));
        }
        drop(out_write);
        drop(err_write);
        drop(in_read);
        drop(in_write);
        let out_thread = std::thread::spawn(move || read_output(stdout));
        let err_thread = std::thread::spawn(move || read_output(stderr));
        let mut cancelled = false;
        loop {
            let wait = WaitForSingleObject(process_handle.0, 25);
            if wait == WAIT_OBJECT_0 {
                break;
            }
            if wait == WAIT_FAILED {
                TerminateJobObject(job.0, 1);
                return Err(error("WaitForSingleObject"));
            }
            if cancel.is_cancelled() {
                cancelled = true;
                TerminateJobObject(job.0, 1);
                WaitForSingleObject(process_handle.0, 5000);
                break;
            }
        }
        let mut exit_code = 0;
        if GetExitCodeProcess(process_handle.0, &mut exit_code) == 0 {
            return Err(error("GetExitCodeProcess"));
        }
        // Kill any descendants retaining pipe handles before joining readers.
        TerminateJobObject(job.0, exit_code);
        drop(job);
        let stdout = out_thread
            .join()
            .map_err(|_| AppError::new("IO", "stdout reader panicked"))?;
        let stderr = err_thread
            .join()
            .map_err(|_| AppError::new("IO", "stderr reader panicked"))?;
        if cancelled {
            return Err(AppError::new("CANCELLED", "Shell cancelled"));
        }
        Ok(ProcessOutput {
            stdout,
            stderr,
            exit_code: exit_code as i32,
        })
    }
}
