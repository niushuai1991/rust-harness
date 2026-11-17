# Agent 执行速度基准

`agent_bench.py` 用固定项目 fixture、独立工作区和数据目录重复执行命令，按轮次轮换配置顺序。`short.example.json` 对照 PR 97 的三个开关；`long.example.json` 使用固定的 30 轮合成历史。均使用 GLM 5.3 Flash、智谱 Coding Plan 的 Anthropic 协议和 high effort。每个样本均是新进程，进程内的后续请求仍可复用连接。

`zcode.example.json` 演示 ZCode 安装包 CLI 的单独测量。将其中的 CLI、内置供应商配置和 `prepare_zcode.py` 路径改为绝对路径；辅助脚本只把本机已有的凭据文件和供应商配置复制到临时数据目录，并在副本中设定模型，不修改用户配置。它要求本机已登录相应的智谱账号。运行器从 ZCode 的 stream-json 事件核实实际供应商、模型和档位，观察不到或不一致时样本失败，避免静默回退。临时凭据副本在运行结束后删除；异常中断后若操作系统无法立即释放文件，需检查系统临时目录。ZCode 的 plan 模式与 Harness 的 never 模式并非完全相同，系统提示词和工具能力也可能不同；对比时必须记录这些差异。当前长会话 fixture 仅适用于 Harness，不能声称两边长会话上下文等价。

ZCode 样本必须使用 `--output-format stream-json`：采集器要求最终结果对应的 `turn.completed` 事件为 `success`，无法确认、取消或超预算都记为失败。`fingerprint_files` 用来额外登记 `zcode.cjs` 和内置配置等文件的摘要，仅对 `node` 可执行文件取摘要不能标识 ZCode 版本。

## 准备与运行

1. 在当前 worktree 执行 `cargo build --release --quiet -p harness-cli`。准备已配置好供应商和凭据引用的 Rust Harness 数据库。基准只读取其中的 `models`、`providers` 配置项，凭据仍由原凭据存储提供，不复制会话或修改该数据库。
2. 复制所需的示例 JSON 到本地配置文件，将 `harness_seed_database` 改为自己的 `harness.db` 的绝对路径，将 `command` 的首项改为刚构建的 `target/release/harness.exe` 的绝对路径。所有变体都要修改。保持模型、供应商端点、协议、effort、任务和权限一致。
3. 在仓库根目录执行 `python benchmarks/agent_bench.py <配置文件> <不存在的输出目录>`。例如：

   ```powershell
   python benchmarks/agent_bench.py target/short.json target/bench-short-01
   python benchmarks/agent_bench.py target/long.json target/bench-long-01
   ```

4. 查看 `summary.json` 和 `samples.jsonl`。每个样本的 stdout 与 stderr 单独保存，便于核对结果和异常。它们可能包含提示词、模型回答、文件内容及错误信息；不要在未经检查时提交或分享。

示例使用 `never` 只读权限和三份小文件，答案需包含六个指定词才计入成功。该校验仅用于确认任务没有被跳过，不能证明所有回答都正确。可以替换 fixture、提示词和 `required_markers`，也可以增加其他可执行程序；命令以参数数组执行，不经 shell，`{workspace}`、`{data_dir}`、`{run_dir}`、`{prompt}`、`{repeat}` 和长会话的 `{session_id}` 在运行时替换。对其他 Agent 可不设置 `harness_seed_database` 和 `result_format`；此时以退出码和输出关键词判定成功，必须自行确认对应命令及权限等价。若没有关键词，`quality_ok` 为 null，质量未验证。

`setup` 可指定若干命令数组，在计时开始前依次执行；每个变体和样本都在新的临时目录中运行。样本超时后终止进程树并记录失败，不计入成功样本中位数。不要并发运行其他任务或构建，且应记录版本、供应商、配置、机器和网络环境；`metadata` 是运行者填写的信息，输出还记录 fixture、命令、可执行文件及冻结后的供应商配置的 SHA-256。命令摘要不能证明程序实际采用了某个模型；仍需结合工具输出和供应商配置核实。输出目录必须不存在，防止覆盖旧结果。包含符号链接的 fixture 会被拒绝。

代码修改任务可在变体中增加 `"verify": ["验证程序", "参数"]`，验证在计时结束后执行并单独记录结果及日志。默认在临时工作区运行；使用 Rust Harness 副本模式时设置 `"verify_cwd": "replica"`，运行器通过 `workspace.list` 找到该次实际副本。`verification_timeout_seconds` 可设置验证超时（默认 300 秒）。`{stdout_path}` 指向已保存的样本输出，`{sample_output}` 是运行者可用于导出产物的路径。设置 `"preserve_on_failure": true` 会在失败时把验证工作区复制到输出目录，复制的文件可能包含敏感数据。若未配置验证，`verification_ok` 为 null，不能把退出成功视为代码任务质量已验证。

## 指标与解释

- `elapsed_ms` 从启动 CLI 到退出，包含进程启动、工作区初始化、模型和工具，排除 fixture 复制、历史准备与 `setup`。CLI 等待子代理收尾，因此其耗时也包含在内。
- `RUST_HARNESS_BENCHMARK_TRACE=1` 时，stderr 以 `BENCHMARK ` 输出不含正文的 JSON 记录：`history_read`、`projection`、`model_request`、`model_first_delta`、`tool_batch` 和 `compaction`。后两项只在实际发生时出现。`model_first_delta` 是首次文本或推理增量，不是 HTTP 首字节；没有增量就没有该项。`tool_batch` 包含准备、执行和结算，也可能包含审批等待。发生错误并提前退出的阶段可能没有记录。
- `summary.json` 包含成功和失败次数、成功样本总耗时中位数，以及各阶段每个样本内求和后的中位数；缺失的阶段不按零计。多会话并行时，不同会话的阶段时间会相互重叠，不能当作墙钟关键路径。压缩阶段内部可能包含模型请求，不能简单把所有阶段求和。
- `samples.jsonl` 的 `harness.model_usages` 保留各次主模型响应的原始用量；供应商字段和缓存口径不同，压缩请求用量不包含在这里。`tool_calls` 包含作用域内子代理的调用；不能把模型请求次数或工具调用次数差异归因于调度实现。

长会话历史是可重复的合成消息，作为模型输入和历史投影负载，不代表真实历史的任务质量或 token 分布。长会话在计时前创建工作区和会话，短任务则在计时内创建，不应直接比较两者的绝对总耗时。合成历史不包含虚构的模型用量，默认不会单靠这些历史触发自动压缩。要衡量自动压缩，应另行准备真实、等价的长会话及近阈值用量，核实 `compaction` 记录、摘要质量与压缩请求用量；不要用未触发压缩的结果评价压缩开关。

实际模型有网络、服务负载和生成轨迹波动；应增加样本、检查原始分布、失败率、答案和调用轨迹，不能从少量样本承诺固定加速比例。桌面 UI 渲染性能不在本工具测量范围内。
