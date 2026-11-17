# Agent 执行速度基准

`agent_bench.py` 用固定项目 fixture、独立工作区和数据目录重复执行命令，按轮次轮换配置顺序。`short.example.json` 对照 PR 97 的三个开关；`long.example.json` 使用固定的 30 轮合成历史。均使用 GLM 5.3 Flash、智谱 Coding Plan 的 Anthropic 协议和 high effort。每个样本均是新进程，进程内的后续请求仍可复用连接。

与实际安装版 ZCode 进行对照时，使用官方 GUI；安装包中的 CLI 不应视为受支持的独立入口。`zcode.example.json` 和 `prepare_zcode.py` 只保留供具备兼容 CLI 的构建实验，不能代表安装版 GUI 的表现。将 CLI、内置供应商配置和辅助脚本路径改为绝对路径；辅助脚本会把本机凭据文件和供应商配置复制到临时数据目录，并在副本中设定模型，不修改用户配置。运行器从 stream-json 事件核实实际供应商、模型和档位，观察不到或不一致时样本失败。临时凭据副本在运行结束后删除；异常中断后若操作系统无法立即释放文件，需检查系统临时目录。示例中的 ZCode plan 模式与 Harness never 模式并不完全相同。当前长会话 fixture 仅适用于 Harness，不能声称两边长会话上下文等价。

2026-11-18 GUI 对照的具体环境、逐步复现方法、数据来源与缺失项见 [GUI 复现资料](GUI-REPRODUCTION.md)；逐次脱敏数据和汇总见[结果记录](RESULTS.md)。

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

## 桌面端请求级诊断

默认不写请求诊断。完全退出桌面程序后，在 PowerShell 中设置独立的绝对文件路径，再从同一环境启动待测版本；只重新打开已有单实例窗口不会改变其环境变量。例如将以下可执行文件路径改成实际待测程序：

```powershell
$env:RUST_HARNESS_REQUEST_TRACE_FILE = 'D:\bench\requests.jsonl'
& 'D:\path\to\rust-harness.exe'
Remove-Item Env:RUST_HARNESS_REQUEST_TRACE_FILE
```

父目录必须预先存在。文件以追加方式写入；为每次实验使用新文件，完成后检查文件是否存在及内容是否完整。另行记录被测程序版本、源码提交及可执行文件 SHA-256：诊断文件本身不包含版本信息，仅凭版本号不能区分同版本的不同构建。写失败会向标准错误输出提示，但不影响请求。退出应用并在未设置变量的环境中重新启动即可关闭采集。文件含会话标识、时间及 token 用量，分享前仍需检查并保护。

每个模型调用结束时输出一行 JSON：`source` 分别为 `main_turn`、`session_title` 或 `compact`；`request_id` 唯一标识该次调用，`session_id` 用于关联会话；`started_unix_ms` 与 `finished_unix_ms` 用于观察并发，`duration_ms` 和 `first_delta_ms` 使用单调时钟，后者只计首个非空文本或推理增量，并可能为 null。计时从调用 provider 前开始，到其返回后结束，包含该调用内的编码、网络和流处理，不包括调用前的历史投影和标题模型构建。诊断文件同步追加会带来额外开销；原有 `model_request` 基准阶段在写入前结束，同时开启两种诊断时，新请求耗时包含旧基准记录写入标准错误输出的开销。墙钟若被系统调整，不能据此精确计算耗时或断言重叠。`status` 为 `completed`、`error`、`cancelled` 或 `dropped`；最后一项表示 future 被丢弃，可能是标题超时、任务退出等，不能确定具体原因。`usage` 只保留已识别的非负整数 token 字段，缺失或零都不能单独证明服务端是否命中缓存。进程被强制终止时可能没有结束记录。本诊断没有 HTTP 状态、首字节或每次传输重试的计时，也不捕获原始流事件。

参考实现的口径有所不同：本地 ZCode 3.18.2 源码 `fec975905a2a` 在主模型步骤较早位置用 `Date.now()` 开始，完成或失败时按逻辑请求记录；首 token 是首次非空文本或推理增量，记录到 `model_usage`，还包含压缩和标题等来源。Codex 本地源码 `c90354172c` 用 `Instant` 记录回合级首 token，除文本和推理外，工具调用等输出项也可触发；它还单独按每次 HTTP 传输尝试记录请求时间，流式 HTTP 的请求时间不是完整流的生成时间。两个项目的源码版本不自动证明与本机运行二进制完全相同；对比必须核对实际口径。

长会话历史是可重复的合成消息，作为模型输入和历史投影负载，不代表真实历史的任务质量或 token 分布。长会话在计时前创建工作区和会话，短任务则在计时内创建，不应直接比较两者的绝对总耗时。合成历史不包含虚构的模型用量，默认不会单靠这些历史触发自动压缩。要衡量自动压缩，应另行准备真实、等价的长会话及近阈值用量，核实 `compaction` 记录、摘要质量与压缩请求用量；不要用未触发压缩的结果评价压缩开关。

实际模型有网络、服务负载和生成轨迹波动；应增加样本、检查原始分布、失败率、答案和调用轨迹，不能从少量样本承诺固定加速比例。桌面 UI 渲染性能不在本工具测量范围内。
