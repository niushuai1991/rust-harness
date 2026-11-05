//! `harness` 命令行入口：JSON-RPC 宿主与 Agent 任务驱动。
mod chat;
mod manage;
mod run;

use clap::{Parser, Subcommand, ValueEnum};
use harness_contracts::{AppError, AppResult};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Approval {
    /// 自动允许作用域内的审批请求
    Auto,
    /// 终端 y/n 确认（需 TTY）
    Ask,
    /// 一律拒绝需审批工具
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Parser)]
#[command(
    name = "harness",
    version,
    about = "Rust Harness：本地开发协作 Agent 命令行"
)]
#[command(subcommand_required = true, arg_required_else_help = true)]
struct Cli {
    /// 数据目录（默认 RUST_HARNESS_DATA_DIR 或 %LOCALAPPDATA%\\RustHarness）
    #[arg(long, global = true, value_name = "PATH")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// stdio JSON-RPC 长连接宿主（推送 session.event）
    Rpc,
    /// 单次 JSON-RPC 调用并打印结果
    Call {
        /// 方法名，如 workspace.list
        method: String,
        /// JSON 参数
        #[arg(default_value = "{}")]
        params: String,
    },
    /// 一次性执行 Agent 任务，流式输出后退出
    Run(run::Args),
    /// 交互式终端对话
    Chat(chat::Args),
    /// 会话查看
    Sessions(manage::SessionsArgs),
    /// 审阅工作副本改动
    Diff(manage::DiffArgs),
    /// 审阅后同步到原项目
    Apply(manage::ApplyArgs),
    /// 模型管理
    Models(manage::ModelsArgs),
}

fn data_dir(cli: &Cli) -> AppResult<PathBuf> {
    match &cli.data_dir {
        Some(path) => Ok(path.clone()),
        None => match std::env::var_os("RUST_HARNESS_DATA_DIR") {
            Some(value) => Ok(value.into()),
            None => Ok(dirs::data_local_dir()
                .ok_or_else(|| AppError::new("DATA_DIR", "无法定位数据目录"))?
                .join("RustHarness")),
        },
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    let cli = Cli::parse();
    let runtime = harness_runtime::Runtime::open(data_dir(&cli)?).await?;
    let _owner = harness_orchestration::install(runtime.clone())?;
    match cli.command {
        Commands::Rpc => harness_cli::serve(runtime).await,
        Commands::Call { method, params } => {
            let parsed: serde_json::Value =
                serde_json::from_str(&params).map_err(|e| AppError::new("JSON", e.to_string()))?;
            println!("{}", harness_cli::dispatch(runtime, &method, parsed).await?);
            Ok(())
        }
        Commands::Run(args) => run::execute(runtime, args).await,
        Commands::Chat(args) => chat::execute(runtime, args).await,
        Commands::Sessions(args) => manage::sessions(runtime, args).await,
        Commands::Diff(args) => manage::diff(runtime, args).await,
        Commands::Apply(args) => manage::apply(runtime, args).await,
        Commands::Models(args) => manage::models(runtime, args).await,
    }
}
