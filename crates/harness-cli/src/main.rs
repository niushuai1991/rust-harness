#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
async fn run() -> harness_contracts::AppResult<()> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        println!(
            "Rust Harness\n  harness [--data-dir PATH] rpc\n  harness [--data-dir PATH] call METHOD JSON\n\nUse workspace.list/open/diff/apply, session.create/send/events/cancel, model.save, approval.resolve.\nSource synchronization requires the complete reviewed FileChange snapshot.\nThe data directory is exclusively locked; close the desktop first or choose a separate directory."
        );
        return Ok(());
    }
    let data = if args.first().is_some_and(|v| v == "--data-dir") {
        if args.len() < 3 {
            return Err(harness_contracts::AppError::new("ARGS", "--data-dir PATH"));
        }
        let path = std::path::PathBuf::from(args.remove(1));
        args.remove(0);
        path
    } else {
        std::env::var_os("RUST_HARNESS_DATA_DIR")
            .map(Into::into)
            .unwrap_or(
                dirs::data_local_dir()
                    .ok_or_else(|| {
                        harness_contracts::AppError::new("DATA_DIR", "无法定位数据目录")
                    })?
                    .join("RustHarness"),
            )
    };
    let runtime = harness_runtime::Runtime::open(data).await?;
    let _owner = harness_orchestration::install(runtime.clone())?;
    match args.first().map(String::as_str) {
        Some("rpc") => harness_cli::serve(runtime).await,
        Some("call") => {
            let method = args
                .get(1)
                .ok_or_else(|| harness_contracts::AppError::new("ARGS", "缺少方法"))?;
            let params = serde_json::from_str(args.get(2).map(String::as_str).unwrap_or("{}"))
                .map_err(|e| harness_contracts::AppError::new("JSON", e.to_string()))?;
            println!("{}", harness_cli::dispatch(runtime, method, params).await?);
            Ok(())
        }
        _ => Err(harness_contracts::AppError::new(
            "ARGS",
            "使用 harness --help",
        )),
    }
}
