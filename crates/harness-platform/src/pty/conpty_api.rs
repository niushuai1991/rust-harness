//! ConPTY API 来源双路径：优先侧载 `conpty.dll`（Microsoft.Windows.Console.ConPTY，
//! 内部启动自带的 OpenConsole.exe 而非系统内置 conhost——绕开部分 Windows 版本上
//! 「GUI 宿主 × inbox conhost」附着失效的平台回归），失败回退系统内置静态绑定。
//!
//! 三个导出函数（`CreatePseudoConsole`/`ResizePseudoConsole`/`ClosePseudoConsole`）
//! 的签名与 kernel32 同名 API ABI 一致（conpty.dll 即 winconpty 的产物），
//! 因此同一组函数指针可指向任一来源。
//!
//! 搜索顺序（alacritty 模式）：当前 exe 同目录 → 当前工作目录 → PATH（LoadLibraryW
//! 默认搜索顺序兜底）。任何一步失败都安全回退 inbox，保证 CI 与正常机器零影响。
#![cfg(windows)]
use std::sync::OnceLock;
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::{
        Console::{COORD, HPCON},
        LibraryLoader::{GetProcAddress, LoadLibraryW},
    },
};

pub type CreatePseudoConsoleFn = unsafe extern "system" fn(
    size: COORD,
    input: HANDLE,
    output: HANDLE,
    flags: u32,
    hpc: *mut HPCON,
) -> i32;
pub type ResizePseudoConsoleFn = unsafe extern "system" fn(hpc: HPCON, size: COORD) -> i32;
pub type ClosePseudoConsoleFn = unsafe extern "system" fn(hpc: HPCON);

/// 一组可调用的 ConPTY 实现（侧载 dll 或系统内置）。
pub struct ConptyApi {
    pub create: CreatePseudoConsoleFn,
    pub resize: ResizePseudoConsoleFn,
    pub close: ClosePseudoConsoleFn,
    /// 实际来源："conpty.dll" 或 "inbox"（诊断与测试断言用）。
    pub source: &'static str,
}

/// 系统内置静态绑定（kernel32 直链）。
fn inbox_api() -> ConptyApi {
    ConptyApi {
        create: windows_sys::Win32::System::Console::CreatePseudoConsole,
        resize: windows_sys::Win32::System::Console::ResizePseudoConsole,
        close: windows_sys::Win32::System::Console::ClosePseudoConsole,
        source: "inbox",
    }
}

/// 按候选目录依次尝试侧载 conpty.dll；全部失败返回系统内置。
/// 候选顺序由调用方给定（exe 目录、当前工作目录等），LoadLibraryW 的
/// 默认搜索顺序（含 PATH）作为最终兜底。
pub fn load_from(candidate_dirs: &[std::path::PathBuf]) -> ConptyApi {
    for dir in candidate_dirs {
        let dll = dir.join("conpty.dll");
        if let Some(api) = try_load_dll(&dll) {
            return api;
        }
    }
    if let Some(api) = try_load_dll(std::path::Path::new("conpty.dll")) {
        return api;
    }
    inbox_api()
}

fn try_load_dll(path: &std::path::Path) -> Option<ConptyApi> {
    let wide: Vec<u16> = path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let module = LoadLibraryW(wide.as_ptr());
        if module.is_null() {
            return None;
        }
        let create = GetProcAddress(module, c"CreatePseudoConsole".as_ptr().cast())?;
        let resize = GetProcAddress(module, c"ResizePseudoConsole".as_ptr().cast())?;
        let close = GetProcAddress(module, c"ClosePseudoConsole".as_ptr().cast())?;
        Some(ConptyApi {
            create: std::mem::transmute::<usize, CreatePseudoConsoleFn>(create as usize),
            resize: std::mem::transmute::<usize, ResizePseudoConsoleFn>(resize as usize),
            close: std::mem::transmute::<usize, ClosePseudoConsoleFn>(close as usize),
            source: "conpty.dll",
        })
    }
}

/// 全局 API 表（进程生命周期内来源不变）。
static API: OnceLock<ConptyApi> = OnceLock::new();

/// 取进程级 ConPTY API：exe 同目录与当前工作目录优先侧载，回退系统内置。
pub fn get() -> &'static ConptyApi {
    API.get_or_init(|| {
        let mut dirs = Vec::new();
        if let Ok(exe) = std::env::current_exe()
            && let Some(parent) = exe.parent()
        {
            // 桌面端 tauri resources 落安装根的 binaries/conpty 子目录；CLI 与开发态在 exe 同目录。
            dirs.push(parent.join("binaries/conpty"));
            dirs.push(parent.to_path_buf());
        }
        dirs.push(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
        load_from(&dirs)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_candidates_fall_back_to_inbox() {
        let api = load_from(&[]);
        assert_eq!(api.source, "inbox");
    }

    #[test]
    fn garbage_dll_falls_back_to_inbox() {
        // 目录存在但 dll 不是合法 PE：LoadLibrary 失败必须安全回退。
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("conpty.dll"), b"not a pe file").unwrap();
        let api = load_from(&[dir.path().to_path_buf()]);
        assert_eq!(api.source, "inbox");
    }

    #[test]
    fn missing_directory_falls_back_to_inbox() {
        let api = load_from(&[std::path::PathBuf::from("Z:/definitely/not/here")]);
        assert_eq!(api.source, "inbox");
    }
}
