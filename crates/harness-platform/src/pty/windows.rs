//! ConPTY-backed sandboxed terminal spawn on Windows.
use super::{OutputBuffer, ResizeFn, SandboxedPty};
use crate::shell::windows::{
    Handle, capability, environment_block, error, grant, sandbox_lease, sid, token, wide,
};
use crate::{WorkspaceLease, io};
use harness_contracts::{AppError, AppResult};
use std::{
    ffi::c_void,
    fs::{self, File},
    io::Read,
    os::windows::io::FromRawHandle,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;
use windows_sys::Win32::{
    Security::SECURITY_ATTRIBUTES,
    System::{
        Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole},
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Pipes::CreatePipe,
        Threading::*,
    },
};
const TERMINAL_COLS: i16 = 120;
const TERMINAL_ROWS: i16 = 30;
/// Owns one HPCON; closed after the process tree is torn down (guard drop order).
struct PcHandle(HPCON);
unsafe impl Send for PcHandle {}
unsafe impl Sync for PcHandle {}
impl Drop for PcHandle {
    fn drop(&mut self) {
        unsafe {
            if self.0 != 0 {
                ClosePseudoConsole(self.0);
            }
        }
    }
}
struct PtyStartup {
    info: STARTUPINFOEXW,
    _attributes: Vec<usize>,
}
impl Drop for PtyStartup {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.info.lpAttributeList);
        }
    }
}
/// Terminal lifetime: kill-on-close job, exit wait handle, private temp and lease.
struct PtyLifetime {
    job: Arc<Handle>,
    process: Arc<Handle>,
    done: CancellationToken,
    _lease: WorkspaceLease,
    _temp: tempfile::TempDir,
    /// Ends donated to CreatePseudoConsole; kept open for the session lifetime
    /// (portable-pty/codex retain them on Windows — closing early can break the
    /// console output path on some builds).
    _pty_input_read: Handle,
    _pty_output_write: Handle,
    // Declared last so the console is closed after the process tree is gone.
    _hpc: Arc<PcHandle>,
}
impl Drop for PtyLifetime {
    fn drop(&mut self) {
        unsafe {
            TerminateJobObject(self.job.0, 1);
            WaitForSingleObject(self.process.0, 5000);
        }
        self.done.cancel();
    }
}
/// `hpc_value` must outlive the subsequent CreateProcess call: the attribute
/// list stores a pointer to it rather than copying the handle.
fn conpty_startup(hpc_value: &HPCON) -> AppResult<PtyStartup> {
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
        info.lpAttributeList = list;
        let startup = PtyStartup {
            info,
            _attributes: attributes,
        };
        if UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            (hpc_value as *const HPCON).cast::<c_void>(),
            std::mem::size_of::<HPCON>(),
            null_mut(),
            null(),
        ) == 0
        {
            return Err(error("UpdateProcThreadAttribute pseudoconsole"));
        }
        Ok(startup)
    }
}
pub(super) fn spawn(root: &Path, cancel: CancellationToken) -> AppResult<SandboxedPty> {
    if cancel.is_cancelled() {
        return Err(AppError::new("CANCELLED", "Terminal cancelled"));
    }
    let (root, id, _manager, lease) = sandbox_lease(root)?;
    let workspace = root
        .parent()
        .ok_or_else(|| AppError::new("INVALID_WORKSPACE", "Missing workspace"))?
        .to_path_buf();
    fs::create_dir_all(workspace.join("temp")).map_err(io)?;
    let temp = tempfile::tempdir_in(workspace.join("temp")).map_err(io)?;
    let cap = capability(&id)?;
    let temp_cap = capability(&uuid::Uuid::new_v4().to_string())?;
    let world = sid("S-1-1-0")?;
    grant(temp.path(), temp_cap.0, world.0)?;
    let restricted = token(cap.0, temp_cap.0, world.0)?;
    unsafe {
        let job = Arc::new(Handle(CreateJobObjectW(null(), null())));
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
        // ConPTY pipe pairs: we keep in_write/out_read; the console gets the ends it reads/writes.
        let security = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        let mut in_read = null_mut();
        let mut in_write = null_mut();
        if CreatePipe(&mut in_read, &mut in_write, &security, 0) == 0 {
            return Err(error("CreatePipe pty input"));
        }
        let in_read = Handle(in_read);
        let in_write = File::from_raw_handle(in_write);
        let mut out_read = null_mut();
        let mut out_write = null_mut();
        if CreatePipe(&mut out_read, &mut out_write, &security, 0) == 0 {
            return Err(error("CreatePipe pty output"));
        }
        let out_write = Handle(out_write);
        let out_read = File::from_raw_handle(out_read);
        let mut hpc: HPCON = 0;
        if CreatePseudoConsole(
            COORD {
                X: TERMINAL_COLS,
                Y: TERMINAL_ROWS,
            },
            in_read.0,
            out_write.0,
            0,
            &mut hpc,
        ) < 0
        {
            return Err(error("CreatePseudoConsole"));
        }
        let hpc = Arc::new(PcHandle(hpc));
        // 存活于本作用域的副本：属性列表仅存指针，不能指向已返回的栈帧。
        let hpc_value = hpc.0;
        let startup = conpty_startup(&hpc_value)?;
        // Shell selection matches ShellService::run: pwsh 7 first, then Windows PowerShell.
        let system = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .and_then(|p| {
                let p = p.join("System32/WindowsPowerShell/v1.0/powershell.exe");
                p.is_file().then_some(p)
            })
            .ok_or_else(|| AppError::new("SANDBOX", "SystemRoot missing"))?;
        let executable = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:/Program Files"))
            .join("PowerShell/7/pwsh.exe");
        let executable = if executable.is_file() {
            executable
        } else {
            system
        };
        let mut command_line = wide(format!("\"{}\" -NoLogo -NoProfile", executable.display()));
        let env = environment_block(temp.path());
        let mut process: PROCESS_INFORMATION = std::mem::zeroed();
        if CreateProcessAsUserW(
            restricted.0,
            wide(&executable).as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            env.as_ptr().cast(),
            wide(&root).as_ptr(),
            &startup.info.StartupInfo,
            &mut process,
        ) == 0
        {
            return Err(error("CreateProcessAsUserW"));
        }
        let process_handle = Arc::new(Handle(process.hProcess));
        let thread = Handle(process.hThread);
        if AssignProcessToJobObject(job.0, process_handle.0) == 0 {
            TerminateProcess(process_handle.0, 1);
            return Err(error("AssignProcessToJobObject"));
        }
        if ResumeThread(thread.0) == u32::MAX {
            TerminateJobObject(job.0, 1);
            return Err(error("ResumeThread"));
        }
        // Output pump: boundary-safe UTF-8 decode straight into the shared buffer.
        let buffer = Arc::new(std::sync::Mutex::new(OutputBuffer::new()));
        let pump_buffer = buffer.clone();
        std::thread::spawn(move || pump(out_read, pump_buffer));
        // Exit waiter: surface the exit code through a oneshot.
        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<i32>();
        let wait_handle = process_handle.clone();
        std::thread::spawn(move || {
            WaitForSingleObject(wait_handle.0, INFINITE);
            let mut code: u32 = 1;
            GetExitCodeProcess(wait_handle.0, &mut code);
            let _ = exit_tx.send(code as i32);
        });
        let done = CancellationToken::new();
        let guard = PtyLifetime {
            job: job.clone(),
            process: process_handle,
            done: done.clone(),
            _lease: lease,
            _temp: temp,
            _pty_input_read: in_read,
            _pty_output_write: out_write,
            _hpc: hpc.clone(),
        };
        let cancel_task = cancel.clone();
        let cancel_job = job.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = cancel_task.cancelled() => { TerminateJobObject(cancel_job.0, 1); },
                _ = done.cancelled() => {},
            }
        });
        let resize: ResizeFn = {
            let hpc = hpc.clone();
            Box::new(move |cols, rows| {
                if ResizePseudoConsole(
                    hpc.0,
                    COORD {
                        X: cols as i16,
                        Y: rows as i16,
                    },
                ) < 0
                {
                    Err(error("ResizePseudoConsole"))
                } else {
                    Ok(())
                }
            })
        };
        Ok(SandboxedPty {
            input: tokio::fs::File::from_std(in_write),
            buffer,
            exit: exit_rx,
            cancel,
            resize,
            _guard: Box::new(guard),
        })
    }
}
/// Blocking reader: decode complete UTF-8 sequences (carrying split boundaries)
/// and push decoded text into the shared output buffer. Never blocks on a consumer.
fn pump(mut file: File, buffer: Arc<std::sync::Mutex<OutputBuffer>>) {
    let mut rest: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 16384];
    loop {
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                rest.extend_from_slice(&chunk[..n]);
                let valid = match std::str::from_utf8(&rest) {
                    Ok(_) => rest.len(),
                    Err(e) => e.valid_up_to(),
                };
                if rest.len() - valid > 3 {
                    // Unrecoverable sequence: flush lossy so the pump never stalls.
                    let text = String::from_utf8_lossy(&rest).into_owned();
                    rest.clear();
                    buffer.lock().unwrap().push(text.as_bytes());
                    continue;
                }
                if valid > 0 {
                    let text = String::from_utf8_lossy(&rest[..valid]).into_owned();
                    rest.drain(..valid);
                    buffer.lock().unwrap().push(text.as_bytes());
                }
            }
            Err(_) => break,
        }
    }
}
