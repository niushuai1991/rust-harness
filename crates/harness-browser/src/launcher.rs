//! 定位并启动系统 Edge/Chrome headless 实例；轮询 `DevToolsActivePort` 等待就绪。
//!
//! 不自动下载浏览器：显式指定 > 环境变量 > 标准安装路径（对齐 ZCode executable.ts 策略）。
//! Edge/Chrome 在 Windows 上会由 launcher 进程转交真正的浏览器进程后自行退出，
//! 因此 stderr 的 `DevTools listening` 不可依赖：就绪检测改读 user-data-dir 下的
//! `DevToolsActivePort`，进程树清理改用 Job Object（KILL_ON_JOB_CLOSE）。
use harness_contracts::{AppError, AppResult};
use std::path::{Path, PathBuf};

/// 按优先级返回候选浏览器路径：Edge（各标准目录）→ Chrome（各标准目录）。
pub fn candidate_roots(env: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for executable in [
        r"Microsoft\Edge\Application\msedge.exe",
        r"Google\Chrome\Application\chrome.exe",
    ] {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
            let Some(base) = env(var) else { continue };
            roots.push(PathBuf::from(base).join(executable));
        }
    }
    roots
}

/// 显式路径 > 环境变量 > 系统候选；返回第一个存在的可执行文件。
pub fn find_executable_with(
    explicit: Option<String>,
    env: impl Fn(&str) -> Option<String>,
) -> AppResult<PathBuf> {
    if let Some(path) = explicit {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(AppError::new(
            "BROWSER_NOT_FOUND",
            format!("指定的浏览器可执行文件不存在：{}", path.display()),
        ));
    }
    if let Some(path) = env("RUST_HARNESS_BROWSER_EXECUTABLE")
        && !path.is_empty()
    {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }
    for candidate in candidate_roots(env) {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(AppError::new(
        "BROWSER_NOT_FOUND",
        "未在标准安装路径找到 Microsoft Edge 或 Google Chrome；可通过环境变量 RUST_HARNESS_BROWSER_EXECUTABLE 指定浏览器路径",
    ))
}

/// 生产包装：从进程环境读取变量。
pub fn find_executable(explicit: Option<String>) -> AppResult<PathBuf> {
    find_executable_with(explicit, |key| std::env::var(key).ok())
}

/// Edge/Chrome 安装根目录的 msedge.exe/chrome.exe 是版本 stub：启动真身后立即退出，
/// 导致 stderr 管道提前关闭且无法管理真正的浏览器进程；优先改用版本子目录中的真身。
pub fn resolve_real_executable(executable: &Path) -> PathBuf {
    let Some(parent) = executable.parent() else {
        return executable.to_path_buf();
    };
    let Some(file_name) = executable.file_name() else {
        return executable.to_path_buf();
    };
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    let Ok(entries) = std::fs::read_dir(parent) else {
        return executable.to_path_buf();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(version) = version_key(name) else {
            continue;
        };
        let candidate = path.join(file_name);
        if candidate.is_file() && best.as_ref().is_none_or(|(current, _)| version > *current) {
            best = Some((version, candidate));
        }
    }
    best.map(|(_, path)| path)
        .unwrap_or_else(|| executable.to_path_buf())
}

/// 目录名形如 `156.0.4314.58` 时返回可比较的版本段。
fn version_key(name: &str) -> Option<Vec<u32>> {
    let parts: Vec<u32> = name
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    (!parts.is_empty()).then_some(parts)
}

/// 浏览器进程树守护：Windows 上是 Job Object 句柄（Drop 时终止进程树）。
#[cfg(windows)]
struct JobGuard {
    job: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
unsafe impl Send for JobGuard {}

/// 句柄只经 `terminate`（互斥地由所有者调用）触达，Close/Terminate 本身线程安全。
#[cfg(windows)]
unsafe impl Sync for JobGuard {}

#[cfg(windows)]
impl JobGuard {
    /// 终止整个进程树并关闭 Job 句柄。
    fn terminate(&mut self) {
        unsafe {
            if !self.job.is_null() {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 0);
                windows_sys::Win32::Foundation::CloseHandle(self.job);
                self.job = std::ptr::null_mut();
            }
        }
    }
}

#[cfg(windows)]
impl Drop for JobGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(not(windows))]
struct JobGuard;

#[cfg(not(windows))]
impl JobGuard {
    fn terminate(&mut self) {}
}

/// 一次 headless 浏览器启动；持有 Job 守护与独立临时 user-data-dir。
pub struct LaunchedBrowser {
    child: tokio::process::Child,
    job: JobGuard,
    /// 实际调试端口（来自 `DevToolsActivePort` 文件第一行）。
    pub debug_port: u16,
    _user_data: tempfile::TempDir,
}

impl LaunchedBrowser {
    /// 启动 headless 实例并等待调试端口就绪（最长 30 秒）。
    pub async fn launch(executable: &Path) -> AppResult<Self> {
        let user_data = tempfile::tempdir()
            .map_err(|e| AppError::new("BROWSER_TEMP", format!("创建临时目录失败：{e}")))?;
        let executable = resolve_real_executable(executable);
        let mut child = tokio::process::Command::new(&executable)
            .arg("--headless=new")
            .arg("--remote-debugging-port=0")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-extensions")
            .arg("--disable-background-networking")
            .arg("--disable-component-update")
            .arg("--disable-sync")
            .arg("--mute-audio")
            .arg("--window-size=1280,900")
            .arg(format!("--user-data-dir={}", user_data.path().display()))
            .arg("about:blank")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| AppError::new("BROWSER_SPAWN", format!("启动浏览器失败：{e}")))?;
        let mut job = attach_job(&child)?;
        let port = wait_devtools_port(user_data.path(), &mut child, &mut job).await?;
        Ok(Self {
            child,
            job,
            debug_port: port,
            _user_data: user_data,
        })
    }

    /// 终止浏览器进程树并等待 launcher 进程退出。
    pub async fn shutdown(&mut self) {
        self.job.terminate();
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

/// 把 launcher 进程挂入 KILL_ON_JOB_CLOSE 的 Job；launcher 转交的真身进程会继承 Job。
#[cfg(windows)]
fn attach_job(child: &tokio::process::Child) -> AppResult<JobGuard> {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(AppError::new("BROWSER_JOB", "创建 Job Object 失败"));
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
        {
            windows_sys::Win32::Foundation::CloseHandle(job);
            return Err(AppError::new("BROWSER_JOB", "配置 Job Object 失败"));
        }
        let Some(process) = child.raw_handle() else {
            windows_sys::Win32::Foundation::CloseHandle(job);
            return Err(AppError::new("BROWSER_JOB", "无法获取浏览器进程句柄"));
        };
        if AssignProcessToJobObject(job, process as _) == 0 {
            windows_sys::Win32::Foundation::CloseHandle(job);
            return Err(AppError::new("BROWSER_JOB", "挂载进程到 Job 失败"));
        }
        Ok(JobGuard { job })
    }
}

#[cfg(not(windows))]
fn attach_job(_child: &tokio::process::Child) -> AppResult<JobGuard> {
    Ok(JobGuard)
}

/// 轮询 `DevToolsActivePort`（浏览器就绪后写入：第一行为端口）。
async fn wait_devtools_port(
    user_data: &Path,
    child: &mut tokio::process::Child,
    job: &mut JobGuard,
) -> AppResult<u16> {
    let port_file = user_data.join("DevToolsActivePort");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Ok(text) = tokio::fs::read_to_string(&port_file).await
            && let Some(port) = text.lines().next().and_then(|l| l.parse().ok())
        {
            return Ok(port);
        }
        if tokio::time::Instant::now() >= deadline {
            job.terminate();
            let _ = child.kill().await;
            return Err(AppError::new(
                "BROWSER_TIMEOUT",
                "等待浏览器调试端口超时（30 秒）",
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
