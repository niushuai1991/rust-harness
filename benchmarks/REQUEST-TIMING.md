# 桌面端请求级诊断

默认不写请求诊断。完全退出桌面程序后，在 PowerShell 中设置独立的绝对文件路径，再从同一环境启动待测版本；只重新打开已有单实例窗口不会改变其环境变量。例如将以下可执行文件路径改成实际待测程序：

```powershell
$env:RUST_HARNESS_REQUEST_TRACE_FILE = 'D:\bench\requests.jsonl'
& 'D:\path\to\rust-harness.exe'
Remove-Item Env:RUST_HARNESS_REQUEST_TRACE_FILE
```

父目录必须预先存在。文件以追加方式写入；为每次实验使用新文件，完成后检查文件是否存在及内容是否完整。另行记录被测程序版本、源码提交及可执行文件 SHA-256：诊断文件本身不包含版本信息，仅凭版本号不能区分同版本的不同构建。写失败会向标准错误输出提示，但不影响请求。退出应用并在未设置变量的环境中重新启动即可关闭采集。文件含会话标识、时间及 token 用量，分享前仍需检查并保护。

每个模型调用结束时输出一行 JSON：`source` 分别为 `main_turn`、`session_title` 或 `compact`；`request_id` 唯一标识该次调用，`session_id` 用于关联会话；`started_unix_ms` 与 `finished_unix_ms` 用于观察并发，`duration_ms` 和 `first_delta_ms` 使用单调时钟，后者只计首个非空文本或推理增量，并可能为 null。计时从调用 provider 前开始，到其返回后结束，包含该调用内的编码、网络和流处理，不包括调用前的历史投影和标题模型构建。诊断文件同步追加会带来额外开销；原有 `model_request` 基准阶段在写入前结束，同时开启两种诊断时，新请求耗时包含旧基准记录写入标准错误输出的开销。墙钟若被系统调整，不能据此精确计算耗时或断言重叠。`status` 为 `completed`、`error`、`cancelled` 或 `dropped`；最后一项表示 future 被丢弃，可能是标题超时、任务退出等，不能确定具体原因。`usage` 只保留已识别的非负整数 token 字段，缺失或零都不能单独证明服务端是否命中缓存。进程被强制终止时可能没有结束记录。本诊断没有 HTTP 状态、首字节或每次传输重试的计时，也不捕获原始流事件。
