# 2026-11-18 桌面基准：复现资料

本记录覆盖本次官方桌面 GUI 的短任务。逐次脱敏指标在 [GUI 样本](results/2026-11-18-gui-samples.json)，汇总及测量限制在 [结果记录](RESULTS.md)。先前 2026-11-17 的 24 次 Harness CLI 试采是另一组实验，其配置及逐次脱敏指标在 [CLI 样本](results/2026-11-17-cli-samples.json)，不能混入 GUI 样本。

CLI 长会话的原始元数据保存了 `legacy_fixture_sha256`：按文件名排序，对每个文件依次散列相对路径 UTF-8、空字节、文件原文、空字节。事后按此旧算法核对，哈希与当前短 fixture 一致；后来运行器的 fixture 算法已有变化，不能直接比较两种算法的摘要。CLI 短任务当时未保存 fixture 哈希，当前文件内容无法作为其历史字节完全一致的证明；CLI 试采所用 debug 可执行文件的哈希也未保存。

## 被测环境

- 测试时间：2026-11-18 01:01–01:10，Asia/Shanghai。GUI 样本文件保留每次回合起止时间；先运行 ZCode 三次，再运行 Rust Harness 三次。
- ZCode：官方安装版 **3.18.2**，Windows 可执行文件版本 `3.18.2.10443`。Rust Harness：安装版 **0.1.6**。两者主程序的 SHA-256 在[结果记录](RESULTS.md)中；产品版本不能唯一确定源码提交或完整安装内容。
- 当前工作区在 GUI 测试开始时为提交 `2e5b4f0`，测试文件在该提交中。测试后核验文件与该提交及 Harness 隔离副本的字节一致。
- 测试机器 CPU：Intel Core i5-13600KF。2026-11-18 事后查询到 Windows 11 专业版 64 位、构建 26300，内存约 63.8 GiB，WebView2 Runtime `157.0.4341.46`；这些事后读取值没有当时的独立快照，不能证明期间绝无变化。
- 两端均使用智谱 Coding Plan 的 GLM 5.3 Flash、`high` 档位。测试时的模型记录、供应商配置显示两端使用 Anthropic 协议，端点主机为 `open.bigmodel.cn`；未保存 HTTP 线路记录，不能独立验证每个请求的实际路由。
- ZCode 使用 `yolo` 模式，工作区为测试文件原目录；Harness 根据界面选择使用 `auto`、隔离副本（`replica`）。两种权限和隔离机制不完全等价。Harness 并发、并发上限、自动压缩和连接复用设置键在测试前及事后读取时均未单独设置；安装程序的实际默认值与当时生效配置没有独立快照。
- 六次均为新会话，但“新会话”不等于相同的模型上下文：ZCode 会附加内部消息和工具，具体系统提示、工具定义、插件、技能、记忆、缓存状态及其精确版本未完整保存。

## 输入和判定

项目目录使用本仓库的 [fixtures/short](fixtures/short/)。原测试目录位于任务 worktree 的 `benchmarks/fixtures/short`，其父目录处于 Git worktree 中；改变目录层级可能改变指令文件、技能和项目上下文的发现结果。三个文件当时均为 UTF-8、LF 换行、41 字节：

| 文件 | SHA-256 |
|---|---|
| `alpha.txt` | `0dcdf05155c7eff2ef8c716dcf091807b7acaca7b4604e8c38c593235926c278` |
| `beta.txt` | `40be670bacf7cb33c4958952811f92c4c9c713fca82395acf4b4ba0c3fdd2e58` |
| `gamma.txt` | `0361d96346630fcc7cb332ff8b587a9da50bd396bbfdd4174cec24e93ab17f0e` |

测试前可用 `Get-FileHash -Algorithm SHA256 benchmarks/fixtures/short/*.txt` 核对；Windows 换行转换也会改变文件哈希。

六次使用完全相同的提示词：

> 读取 alpha.txt、beta.txt 和 gamma.txt。分别列出三个项目代号和对应负责人。

每个会话均检查回合成功结束，最后一条回答包含 `海棠`、`林青`、`松林`、`周明`、`星河`、`陈宁` 六个词。该检查不验证对应关系：正确配对是 alpha = 海棠/林青、beta = 松林/周明、gamma = 星河/陈宁。逐个文件和对应关系的人工验证可用于今后的更严格测试，但不应追认成本次已执行的检查。

## 执行方法

1. 使用目标版本，核对可执行文件哈希，打开上述测试目录；两个程序都选择相同供应商、模型和 `high` 档位。按本次条件设置 ZCode `yolo`，Harness `auto` 和 `replica`。记录实际生效的模型、端点和设置；不要依赖产品版本默认值。
2. 在 ZCode 新建一个会话，发送上述提示词，等待回合完成；重复新建会话两次。原测试期间没有刻意清空服务端缓存，且三个样本都有缓存读取。
3. 在 Harness 中对同一份测试文件的新建工作区进行同样三次新会话测试。打开工作区和建立副本在回合计时之前；本次未测量这些步骤的耗时。
4. 检查最终回答、主模型请求和工具调用。ZCode 每次主回合为两次模型请求和三次 `Read`；Harness 为两次模型请求和三次 `read`。如调用次数或结果不同，应保留样本并注明，不把它归因于执行器调度。
5. 原始测试先做三次 ZCode，再做三次 Harness。若目标是获得更稳健的新结论，建议增加样本并交错两端的测试顺序；这属于改进实验，不是对原时间顺序的逐字复现。

## 计时与样本来源

ZCode 从本地 `.zcode/cli/db/db.sqlite` 的 `turn_usage` 读取回合 `status`、`started_at`、`completed_at` 和 `duration_ms`；从同会话、同回合的 `model_usage` 中选择 `query_source = 'main_turn'`；从 `tool_usage` 读取工具记录。`session_title` 模型请求单列，不能混进主回合。Harness 从 `.RustHarness/harness.db` 的 `events` 表按同会话 `seq` 读取 `turn.start`、`turn.end`、`assistant.message`、`tool.call`、`tool.result`：回合耗时为两端时间戳之差，模型用量取主回合的 `assistant.message`。本次用样本起止时间关联专用测试会话；公开数据将会话标识替换为 `Z1`–`Z3`、`R1`–`R3`。

两端均为运行时回合时长，测量起止实现不完全相同，不等于从鼠标点击到屏幕渲染完成的严格 GUI 延迟。GUI 样本包含 ZCode 的模型请求、首 token 和单工具耗时；Harness GUI 没有采集到等价的请求耗时、首 token 或单工具耗时，JSON 中以 `null` 表示，不得当作零。Harness 的 Anthropic 原始 `input_tokens_uncached` 不包含缓存读取；ZCode 的 `input_tokens_total` 已包含缓存读取，不应把后者的缓存量再次相加。

样本中的 ZCode 模型请求和工具时间戳及耗时来自对应的 `model_usage` 和 `tool_usage` 原始字段；工具的两个时间戳之差与记录的 `duration_ms` 不一定相等，不应强行替换。Harness 的 `assistant_event_recorded_at`、`call_event_recorded_at` 和 `result_event_recorded_at` 是持久化事件时间，不是模型请求或工具的实际起止时间；尤其并发工具按声明顺序结算，不能用这些事件时间倒推单工具耗时。

ZCode 每个测试会话还发生一次 `low` 档位的标题生成，记录在 GUI 样本的 `title_requests` 中，均在主回合结束后开始。Harness 三个会话的标题状态均为 `generated`；当前工作区源码允许标题请求与主回合并行，但安装版是否实际发起、何时完成及其耗时没有采集到，不能将这条源码行为直接当作安装版的实测事实。

读取正在使用的 SQLite 数据库时应采用只读连接和 SQLite 一致性读取；数据库和原始事件可能包含其他私人会话，整理前不要直接复制进仓库。导出的 JSON 只保留本次测试的指标、虚构 fixture、提示词和判定结果，不包含原始会话标识、私有目录、凭据、系统提示正文或工具结果正文。

## 安装包和缺失项

事后在本机找到了 `Rust Harness_0.1.6_x64-setup.exe`，SHA-256 为 `6c90c8d4fd3d57faf9a0ab7aa6344b46175bd6f46bc9c717f20a45a54d08531d`；从包中读取的 `rust-harness-desktop.exe` 哈希与测试程序一致。该安装包在本机构建产物目录，未提交或随 PR 归档，清理构建目录后可能丢失。

事后对当前 ZCode 安装目录另外计算了文件哈希：`resources/app.asar` 为 `4c0b8510a1ff7f1142714a1dc37139058254b784ad72b70ba781f09334b0c131`，`resources/glm/zcode.cjs` 为 `08726b5ef69e1e9072828baa4656713cb3aac93bb096938681278c5600312b2c`，`resources/config/provider/zcode-builtin.json` 为 `ef3d64656cb5281cce292b77ef12e96b5251582cfb00cdf9eb08e3d4565abb69`。这些是事后快照，不能证明测试时所有动态配置和资源均与之相同；未归档 ZCode 安装包，也未证明它在将来仍可下载。

未保存完整请求/响应、模型与工具定义、插件和技能状态、系统提示、代理及网络拓扑、远程模型后端版本、实时服务负载或 GUI 绘制完成时间。以上信息不能从脱敏样本中恢复；服务端缓存和模型输出也不能保证重现。因此现有材料足以复跑相同工作负载、复核已保存的统计，不能保证重现完全相同的上下文、执行轨迹或秒数。需精确复现时，应在本地另行安全保存两款安装包及适当的脱敏配置快照，并在采集时记录上述环境；不应提交凭据或其他用户会话。
