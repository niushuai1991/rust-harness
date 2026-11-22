//! 控制台宿主 ConPTY 附着矩阵（对照）实验（诊断工具）：自包含 mini-ConPTY，扫 shell × flag 组合。
//!
//! 目的：把「附着失败」的变量切开——powershell 5.1 特有 vs ConPTY 通用 vs CreatePseudoConsole
//! flag 差异。每组：spawn → 写 echo 标记 → 5 秒轮询输出/退出 → 汇总写系统临时目录
//! `pty_attach_matrix.txt`。判定：输出含标记 = 附着成功（输入到达且回流）。
// 控制台子系统对照版：与 attach_matrix.rs 同款矩阵，验证宿主子系统差异。

#[cfg(windows)]
use windows_sys::Win32::Foundation::HANDLE;
#[cfg(windows)]
fn main() {
    // 标注宿主子系统：console 版。
    let results = run_matrix();
    let path = std::env::temp_dir().join("pty_attach_matrix.txt");
    std::fs::write(&path, results).expect("写结果");
}

#[cfg(windows)]
fn run_matrix() -> String {
    let mut report = String::from("=== 控制台宿主 ConPTY 附着矩阵（对照） ===\n");
    let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"));
    let shells: Vec<(&str, std::path::PathBuf, &str)> = vec![
        (
            "powershell5.1",
            system.join("System32/WindowsPowerShell/v1.0/powershell.exe"),
            "-NoLogo -NoProfile",
        ),
        (
            "cmd.exe",
            std::path::PathBuf::from(r"C:\Windows\System32\cmd.exe"),
            "",
        ),
    ];
    let pwsh7 = std::path::PathBuf::from(r"C:\Program Files\PowerShell\7\pwsh.exe");
    let shells = if pwsh7.is_file() {
        let mut v = vec![("pwsh7", pwsh7, "-NoLogo -NoProfile")];
        v.extend(shells);
        v
    } else {
        report.push_str("(pwsh7 未安装，跳过)\n");
        shells
    };
    for (name, exe, args) in &shells {
        for inherit_cursor in [false, true] {
            let no_suspend = std::env::var("GUI_PTY_NO_SUSPEND").is_ok();
            let label = format!("{name} INHERIT_CURSOR={inherit_cursor} no_suspend={no_suspend}");
            let outcome = std::panic::catch_unwind(|| probe_one(exe, args, inherit_cursor));
            match outcome {
                Ok(line) => report.push_str(&format!("{label}: {line}\n")),
                Err(_) => report.push_str(&format!("{label}: PANIC\n")),
            }
        }
    }
    report
}

#[cfg(windows)]
fn probe_one(exe: &std::path::Path, args: &str, inherit_cursor: bool) -> String {
    use std::time::{Duration, Instant};
    use windows_sys::Win32::System::{
        Console::{
            COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, PSEUDOCONSOLE_INHERIT_CURSOR,
        },
        Threading::*,
    };
    unsafe {
        let (in_read, in_write) = pipe_pair();
        let (out_read, out_write) = pipe_pair();
        let mut hpc: HPCON = 0;
        let flags = if inherit_cursor {
            PSEUDOCONSOLE_INHERIT_CURSOR
        } else {
            0
        };
        let hr = CreatePseudoConsole(COORD { X: 120, Y: 30 }, in_read, out_write, flags, &mut hpc);
        if hr < 0 {
            return format!("CreatePseudoConsole 失败 hr={hr:#x}");
        }
        // attribute list
        let mut size = 0usize;
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size);
        let mut buffer = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        let list = buffer.as_mut_ptr().cast();
        if InitializeProcThreadAttributeList(list, 1, 0, &mut size) == 0 {
            return "InitializeProcThreadAttributeList 失败".into();
        }
        if UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            (&hpc as *const HPCON).cast::<std::ffi::c_void>(),
            std::mem::size_of::<HPCON>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        ) == 0
        {
            return "UpdateProcThreadAttribute 失败".into();
        }
        let mut startup: STARTUPINFOEXW = std::mem::zeroed();
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.lpAttributeList = list;
        let mut command_line = wide(&format!("\"{}\" {args}", exe.display()));
        let mut process: PROCESS_INFORMATION = std::mem::zeroed();
        let no_suspend = std::env::var("GUI_PTY_NO_SUSPEND").is_ok();
        let mut flags =
            CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW;
        if !no_suspend {
            flags |= CREATE_SUSPENDED;
        }
        let ok = CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            flags,
            std::ptr::null(),
            std::ptr::null(),
            &startup.StartupInfo,
            &mut process,
        );
        if ok == 0 {
            return "CreateProcessW 失败".into();
        }
        if !no_suspend {
            ResumeThread(process.hThread);
        }
        // 写标记命令（延迟 500ms 等壳就绪），读输出 5 秒。
        let mut marker_seen = false;
        std::thread::sleep(Duration::from_millis(500));
        let cmd = if exe.file_stem().unwrap() == "cmd" {
            "echo ATTACH_MATRIX_MARKER\r\nexit\r\n"
        } else {
            "Write-Output ATTACH_MATRIX_MARKER\r\n"
        };
        write_pipe(in_write, cmd);
        let mut seen = String::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut code: Option<u32> = None;
        while Instant::now() < deadline {
            seen.push_str(&read_pipe(out_read, 100));
            if seen.contains("ATTACH_MATRIX_MARKER") {
                marker_seen = true;
            }
            let mut exit: u32 = 0;
            if WaitForSingleObject(process.hProcess, 0) == 0 {
                GetExitCodeProcess(process.hProcess, &mut exit);
                code = Some(exit);
                break;
            }
        }
        terminate_job_or_process(process.hProcess);
        ClosePseudoConsole(hpc);
        format!(
            "marker={marker_seen} exit={code:?} output_len={}",
            seen.len()
        )
    }
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn pipe_pair() -> (HANDLE, HANDLE) {
    use windows_sys::Win32::{Security::SECURITY_ATTRIBUTES, System::Pipes::CreatePipe};
    unsafe {
        let mut read = std::ptr::null_mut();
        let mut write = std::ptr::null_mut();
        let security = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        assert_ne!(CreatePipe(&mut read, &mut write, &security, 0), 0);
        (read, write)
    }
}

#[cfg(windows)]
fn write_pipe(handle: HANDLE, text: &str) {
    use std::{fs::File, io::Write, os::windows::io::FromRawHandle};
    unsafe {
        let mut file = File::from_raw_handle(handle);
        let _ = file.write_all(text.as_bytes());
        let _ = file.flush();
        std::mem::forget(file); // 句柄归实验所有，不关闭
    }
}

#[cfg(windows)]
fn read_pipe(handle: HANDLE, ms: u32) -> String {
    use std::{fs::File, io::Read, os::windows::io::FromRawHandle};
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;
    unsafe {
        // PeekNamedPipe 探可用字节数，无数据立即返回空（File::read 在匿名管道无数据时会阻塞）。
        let mut available: u32 = 0;
        if PeekNamedPipe(
            handle,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        ) == 0
            || available == 0
        {
            let _ = ms;
            return String::new();
        }
        let mut file = File::from_raw_handle(handle);
        let mut chunk = vec![0u8; available as usize];
        let text = match file.read(&mut chunk) {
            Ok(n) => String::from_utf8_lossy(&chunk[..n]).into_owned(),
            Err(_) => String::new(),
        };
        std::mem::forget(file);
        text
    }
}

#[cfg(windows)]
fn terminate_job_or_process(handle: HANDLE) {
    use windows_sys::Win32::System::Threading::TerminateProcess;
    unsafe {
        TerminateProcess(handle, 1);
    }
}

#[cfg(not(windows))]
fn main() {
    println!("Windows only");
}
