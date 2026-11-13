//! `harness` 命令行入口：JSON-RPC 宿主与 Agent 任务驱动。
use clap::{Parser, Subcommand};
use harness_cli::{chat, manage, run};
use harness_contracts::{AppError, AppResult};
use std::path::PathBuf;

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
    /// ACP（Agent Client Protocol）v1 agent 端，供编辑器客户端经 stdio 接入
    Acp {
        /// 模型 ID（缺省用 cli.default_model）
        #[arg(long)]
        model: Option<String>,
        /// 能力预设（显式 --model 优先于预设字段）
        #[arg(long)]
        preset: Option<String>,
    },
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
    /// 数据库维护
    Db(manage::DbArgs),
    /// 审阅工作副本改动
    Diff(manage::DiffArgs),
    /// 审阅后同步到原项目
    Apply(manage::ApplyArgs),
    /// 模型管理
    Models(manage::ModelsArgs),
    /// 预设管理
    Presets(manage::PresetsArgs),
    /// 技能管理
    Skills(manage::SkillsArgs),
    /// ChatGPT 账号登录（浏览器 OAuth）
    Login(manage::LoginArgs),
    /// 登出 ChatGPT 账号并清除本地凭据
    Logout,
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
    let _browser = harness_browser::install(runtime.clone())?;
    match cli.command {
        Commands::Rpc => harness_cli::serve(runtime).await,
        Commands::Acp { model, preset } => {
            harness_cli::acp::serve(runtime, harness_cli::acp::AcpOptions { model, preset }).await
        }
        Commands::Call { method, params } => {
            let parsed: serde_json::Value =
                serde_json::from_str(&params).map_err(|e| AppError::new("JSON", e.to_string()))?;
            println!("{}", harness_cli::dispatch(runtime, &method, parsed).await?);
            Ok(())
        }
        Commands::Run(args) => {
            let code = run::execute(runtime, args).await?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Commands::Chat(args) => chat::execute(runtime, args).await,
        Commands::Sessions(args) => manage::sessions(runtime, args).await,
        Commands::Db(args) => manage::db(runtime, args).await,
        Commands::Diff(args) => manage::diff(runtime, args).await,
        Commands::Apply(args) => manage::apply(runtime, args).await,
        Commands::Models(args) => manage::models(runtime, args).await,
        Commands::Presets(args) => manage::presets(runtime, args).await,
        Commands::Skills(args) => manage::skills(runtime, args).await,
        Commands::Login(args) => manage::login(runtime, args).await,
        Commands::Logout => manage::logout(runtime).await,
    }
}
