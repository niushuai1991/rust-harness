# Task 3 扩展实现与验收记录

## 公共接口

`harness_extensions::PluginManager` 提供 `new(PathBuf)`、`list()`、`install(&Path, Vec<String>)`、`set_enabled(&str,bool)`、`remove(&str)`；安装后默认禁用。同一版本不可覆盖，启用中的插件必须先停用才允许升级；仍有启用依赖者时无法停用。卸载注销插件并取消调用，版本文件保留在宿主数据区，避免破坏尚在退出的 Store；本版未提供历史版本垃圾回收。

运行接线为 `tool_definitions(workspace_id)`、异步 `execute_tool(workspace_id,root,name,args,cancel)` 和 `run_hook(workspace_id,root,hook,payload,cancel)`。工具名为 `plugin__<id>__<tool>`。`set_workspace_policy` 设置工作区权限上限；未配置时全部拒绝。权限取 manifest 声明、安装显式授权、工作区上限三者交集。

`run_hook` 只执行已启用且在 `[contributions].hooks` 声明的 `before_request`、`before_tool`、`after_tool`，需要额外 `hooks` 和 `tools` 权限。它调用真实组件的 `execute("hook:<name>", payloadJSON)`，返回按插件 ID 排序的 JSON 结果列表。分别校验 `{context:string}`、`{name:string,arguments:object}`、`{content:string,isError:boolean}`；before_tool 不接受 `id` 字段。无声明返回空列表，失败显式返回错误。最终工具名、参数、审批和目录 guard 必须由 runtime 在 hook 后再次执行。

`contributions()` 返回启用插件声明的 settings/tools/commands/themes。它是声明数据来源；commands/themes/settings 是否展示由上层 UI 决定，不能将声明数据误计成所有 UI 入口已接线。

`HostPolicy` 的 read_file/write_file/http_get 必须由宿主实现目录边界、审批和有界请求；默认实现全部拒绝。`set_host_policy(Arc<dyn HostPolicy>)` 注入。没有直接绕过 policy 的文件或网络 host import。

`mcp::McpManager` 使用官方 `rmcp 3.8.0`，提供 `connect_http`、`connect_sandboxed_stdio`、`tool_definitions`、`execute_tool`、`list_resources`、`read_resource`、`disconnect`。连接需要显式授权；stdio 接收平台沙箱创建的读写管道，不自行启动普通不受限命令。工具名为 `mcp__<server>__<tool>`。新连接成功发现工具后才替换旧连接，并取消旧连接在途调用；不自动重放工具副作用。工具调用采用 SDK `call_tool_once`，丢弃 future 由官方 AbandonedRequestGuard 发取消通知；60秒超时，连接和资源请求30秒。远端HTTP端点不受本地Windows进程沙箱约束。

## 已实现

- TOML manifest 严格拒绝未知字段；semver API和依赖检查；无安装脚本和 Node 核心依赖。
- 目录/ZIP先复制到随机 staging，再验证、编译、重命名发布及保存注册表；失败清理 staging，不发布半成品。
- 拒绝绝对路径、父目录穿越、反斜杠、UNC、ADS、保留设备名、尾点/空格、大小写重复条目、符号链接/Windows reparse points和特殊文件。限制2048条、单文件32MiB、总计128MiB、压缩比200倍。
- 使用 Wasmtime 49 的 component model 和版本化 `rust-harness:plugin@0.1.0` WIT，实际编译执行WASM。WASI2不继承目录、网络、环境、标准IO；默认仍有WASI时钟和随机能力。
- 每次调用独立 Store，工作区不会共享 guest 内存；当前manager串行执行以保证KV事务，排队时仍可取消；单 memory上限256MiB、fuel 5e9、epoch轮询10ms，activate/deactivate及hook5秒，普通tool60秒。
- 生命周期采用每次调用 `activate → execute → deactivate`。此为首版设计修订：没有驻留 Store/后台 guest 生命周期。组件编译在执行工作线程前进行，编译耗时不计 guest deadline。
- KV按插件/工作区隔离，整个生命周期完成、输出校验成功且未取消/停用后才原子提交。调用失败保留原KV；文件/HTTP外部副作用无法回滚。
- 停用先注销声明、取消当前generation，阻止新调用；在途WASM在下一epoch trap并释放Store。同步host policy本身必须有界，epoch不能打断宿主正在阻塞的系统调用。

## 实际验证

执行 `cargo +stable test -p harness-extensions --tests --offline`。当前套件18项全部通过（8项component、6项manager、4项MCP），无编译警告。包括：

- 真实WAT组件：成功输出、工作区权限拒绝、256MiB内存超限、无限循环取消、无外部取消的无限activate由fuel终止、在途停用取消、activation trap不提交KV、真实hook输出/额外授权、非法hook输出不提交KV。
- 安装：API不兼容、缺依赖、版本不可变、显式授权、启停/重启/卸载、ZIP遍历/ADS/UNC/大小写重复、symlink、压缩炸弹、启用依赖升级拒绝并保持旧状态。
- 官方MCP SDK：双工stdio实际握手/工具发现/工具调用/资源列表/断开，HTTP loopback实际StreamableHTTP握手/发现/调用，未授权HTTP拒绝、取消通知到达服务器，以及重连不重放工具调用。

这验证的是crate能力。平台受限令牌stdio进程、桌面授权入口、真实第三方服务器互操作、hook最终runtime guard和UI消费需要对应上层集成验收；本报告不将预留接口计成这些入口已完成。MCP MRTR输入请求、task continuation和自动后台重连未实现，遇到它们显式失败，不静默重复执行。
