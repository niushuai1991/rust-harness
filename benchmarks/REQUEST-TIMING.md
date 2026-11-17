# 桌面端请求级诊断

默认不写请求诊断。完全退出桌面程序后，在 PowerShell 中设置独立的绝对文件路径，再从同一环境启动待测版本；只重新打开已有单实例窗口不会改变其环境变量。例如将以下可执行文件路径改成实际待测程序：

```powershell
$env:RUST_HARNESS_REQUEST_TRACE_FILE = 'D:\bench\requests.jsonl'
& 'D:\path\to\rust-harness.exe'
Remove-Item Env:RUST_HARNESS_REQUEST_TRACE_FILE
```

父目录必须预先存在。文件以追加方式写入；为每次实验使用新文件，完成后检查文件是否存在及内容是否完整。另行记录被测程序版本、源码提交及可执行文件 SHA-256：诊断文件本身不包含版本信息，仅凭版本号不能区分同版本的不同构建。写失败会向标准错误输出提示，但不影响请求。退出应用并在未设置变量的环境中重新启动即可关闭采集。文件含会话标识、时间及 token 用量，分享前仍需检查并保护。

每个模型调用结束时输出一行 JSON：`source` 分别为 `main_turn`、`session_title` 或 `compact`；`request_id` 唯一标识该次调用，`session_id` 用于关联会话；`started_unix_ms` 与 `finished_unix_ms` 用于观察并发，`duration_ms` 和 `first_delta_ms` 使用单调时钟，后者只计首个非空文本或推理增量，并可能为 null。计时从调用 provider 前开始，到其返回后结束，包含该调用内的编码、网络和流处理，不包括调用前的历史投影和标题模型构建。诊断文件同步追加会带来额外开销；原有 `model_request` 基准阶段在写入前结束，同时开启两种诊断时，新请求耗时包含旧基准记录写入标准错误输出的开销。墙钟若被系统调整，不能据此精确计算耗时或断言重叠。`status` 为 `completed`、`error`、`cancelled` 或 `dropped`；最后一项表示 future 被丢弃，可能是标题超时、任务退出等，不能确定具体原因。`usage` 只保留已识别的非负整数 token 字段，缺失或零都不能单独证明服务端是否命中缓存。进程被强制终止时可能没有结束记录。本诊断没有 HTTP 状态、首字节或每次传输重试的计时，也不捕获原始流事件。

## 原始响应流采集

如需核对供应商的原始用量事件，可在启动进程前**额外**设置独立文件路径：

```powershell
$env:RUST_HARNESS_SSE_TRACE_FILE = 'D:\bench\streams.jsonl'
```

该开关默认关闭，可单独使用，也可与上面的计时开关同时使用。使用同一进程时，两类记录的 `session_id`、`request_id` 和 `source` 相同。不同进程应使用不同文件；目录必须预先存在，文件以追加方式写入。退出应用并在未设置变量的环境中重新启动即可关闭采集。

每条 JSONL 记录的 `sse_base64` 是实际送入解码器前的响应字节的 Base64 编码，可无损还原；`received_bytes`、`captured_bytes` 和 `truncated` 表示接收与保存情况，单请求上限为 8 MiB。记录还包含状态、HTTP 状态和已观察到的白名单用量。失败、取消或 future 被丢弃时可保留不完整的响应和部分用量；强制终止进程可能没有记录。非成功 HTTP 状态只记录状态，不捕获错误响应正文。此处不是原始 TCP/HTTP 线路，不保留分块边界、每帧时间或程序停止读取之后的数据。

例如，用 Python 解码文件中的第一条记录：

```powershell
python -c 'import base64,json,pathlib; p=pathlib.Path(r"D:\bench\streams.jsonl"); r=json.loads(p.open(encoding="utf-8").readline()); pathlib.Path(r"D:\bench\response.sse").write_bytes(base64.b64decode(r["sse_base64"]))'
```

**原始响应可能包含模型输出、工具参数或敏感信息**；Base64 不是加密。仅保存在受控本地目录，检查后再分享或删除。记录不包括请求头和请求正文，但不能保证响应内容中没有敏感信息。采集及同步文件写入会影响耗时，比较速度时应使用相同采集设置。
