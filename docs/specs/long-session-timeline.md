# 长会话时间线性能与有界渲染规范

- 日期：2026-11-10（设计草案）
- 状态：待确认
- 范围：桌面端长会话的事件分页、载荷惰性读取、客户端缓存、可变高度虚拟时间线与流式渲染节流；定义 React/Tauri 与后续 GPUI 前端共用的时间线契约。

## 背景与意图

Agent 长程任务会持续追加消息、工具调用、工具结果和差异文本。现有桌面端切换会话时从 `seq=1` 读取全部事件，Zustand 持有所有已打开会话的完整事件数组，聊天区为全部历史创建 DOM 卡片；流式阶段还会反复解析整段 Markdown。会话越长，宿主/渲染进程内存、会话切换和流式重绘成本越高。

`/compact` 仅在模型投影历史中以摘要替代旧消息，SQLite 中的原始事件继续保留，因此不能作为界面内存控制手段。原始事件日志仍是事实来源，必须保留供导出、搜索、审计和恢复使用。

本规范的核心目标是使桌面 UI 的资源消耗由“当前可见区域与有限缓存”决定，而不随单个会话的历史总量线性增长。它不预设最终 UI 框架：先建立跨渲染器的数据和渲染约束，再决定是否将界面迁移至 GPUI。

## 目标与非目标

### 目标

- 打开或切换会话时只读取最新一页时间线，不传输完整历史和未展开的工具正文。
- 聊天视图只挂载可视区域及其小范围缓冲内的回合卡片，支持向上加载更早历史且不跳动。
- 非当前会话不在客户端常驻完整事件正文；会话切换成本与页大小有关，而与全量历史无关。
- 流式增量按帧合并；未完成消息避免反复解析完整 GFM Markdown。
- 保持事件顺序、导出、全文搜索、审批审计、`/compact` 的原始事件保留语义以及 CLI/ACP 现有接口兼容。
- 让同一 `TimelinePage` / `TimelineEventDetail` 契约可同时服务现有 Tauri 前端和未来 GPUI 前端。

### 非目标

- 不删除、重写或压缩 SQLite 中的原始 `events` 记录。
- 不改变模型上下文压缩、工具审批、工作副本或 `workspace_apply` 语义。
- 不在本规范中直接实施 GPUI、替换 Tauri，或引入嵌入式浏览器。
- 不承诺未经基准测量的固定毫秒数或内存 MiB 阈值；验收以有界载荷、可复现基准和同机前后对比为准。

## 方案比较与结论

| 方案 | 优点 | 代价与问题 | 结论 |
|---|---|---|---|
| 仅将 React/Tauri 改为虚拟列表 | 改动最小，能直接解决全量 DOM 和全量读取 | 若接口、缓存和流式策略仍无界，问题会残留 | 必须做，但应抽象为独立时间线层。 |
| 直接重写为 GPUI | 去除 WebView2 基础开销，适合代码/时间线类自绘界面 | 重写 Markdown、设置面板、窗口集成；若仍常驻全量事件，内存仍线性增长 | 不能替代本规范。 |
| 先落地渲染器无关的有界时间线，再评估 GPUI | 立即改善现有版本；未来迁移复用存储、分页、缓存和测试 | 需要增加明确的桌面查询契约与性能基准 | 采用。第一期接入现有 Tauri 前端；GPUI 仅在该契约稳定后做独立 POC。 |

## 现状证据

- `frontend/App.tsx` 切换会话时调用 `session_events` 未传游标；结束一次发送或压缩后也会再次全量读取。
- `src-tauri/src/commands.rs` 虽接受可选 `after`，但默认值为 `0`。
- `crates/harness-storage/src/lib.rs` 使用 `seq>? ORDER BY seq`，没有 `LIMIT`。
- `frontend/store.ts` 以会话 id 为键持有完整事件数组，未设置 LRU 或正文淘汰。
- `frontend/App.tsx` 对全部事件构建卡片，并对每条助手消息执行 `ReactMarkdown + remarkGfm`。
- `crates/harness-orchestration/src/tests.rs` 验证压缩后原始消息依然存在，符合事件日志保留要求。

## 总体架构

```text
SQLite events（完整、持久、仅追加）
        │
        ├─ 既有 session_events(after)：CLI / ACP / 增量补拉兼容接口
        │
        └─ 新增 TimelineRepository
              ├─ 快照订阅 / 前后分页：受限条数的显示摘要
              └─ detail / payload_chunk：按需、限字节读取工具结果、diff、终端输出等正文
                       │
                       ├─ Tauri command → React 可变高度虚拟时间线
                       └─ Rust facade → GPUI 虚拟时间线（后续 POC）
```

`TimelineRepository` 是运行时/存储层的 UI 查询投影，不是第二份事实数据。它从原始事件投影，不能写入或改变事件内容。

## 时间线查询契约

既有 `session_events(sessionId, after?)` 保持不变，继续用于 CLI、ACP 和兼容用途；它不再是桌面长会话的丢帧追赶接口，因为 `after` 到当前的事件数可能无界。桌面端改用以下新增查询：

```text
session_timeline_subscribe(sessionId) -> TimelineSubscription
session_timeline_page(sessionId, beforeSeq?, throughSeq, limit?) -> TimelinePage
session_timeline_forward_page(sessionId, afterSeq, throughSeq?, limit?) -> TimelinePage
session_timeline_event_detail(sessionId, eventId) -> TimelineEventDetail
session_timeline_payload_chunk(sessionId, eventId, field, offset?, limitBytes?) -> TimelinePayloadPage
```

`session_timeline_subscribe` 必须在线程/事务边界内先注册该会话的订阅，再捕获并返回 `headSeq`。此后 `seq > headSeq` 的事件先进入该订阅的有界缓冲，`seq <= headSeq` 的事件只从快照分页返回；两者不能在注册与读快照之间留下空窗。实现可使用 Tauri `Channel`、Rust stream 或 GPUI callback，但线性化语义必须相同。

客户端先订阅取得 `headSeq`，再调用最新页（`throughSeq=headSeq`），最后按 `seq` 顺序合并订阅缓冲。若缓冲溢出、检测到 seq 缺口或订阅重连，客户端以最后一个连续 `seq` 为 `afterSeq` 调用有上限的 `session_timeline_forward_page`，持续分页至观察到的水位；不得退回调用无界的 `session_events(after)`。订阅缓冲只含 `TimelineEvent` 摘要，达到上限时丢弃缓冲并标记为需要上述补拉，不能把完整事件正文堆积在内存。

### `TimelinePage`

```text
TimelinePage {
  sessionId: String,
  events: Vec<TimelineEvent>,       // seq 升序，长度受服务端上限限制
  olderCursor: Option<String>,      // 本页最早事件之前的游标；None 表示没有更早记录
  throughSeq: String,               // 本次快照的固定上界，不会混入更晚事件
  hasOlder: bool,
}

TimelineSubscription { subscriptionId: String, headSeq: String }

TimelinePayloadPage {
  field: String,
  offset: u64,
  bytes: Vec<u8>,
  totalBytes: u64,
  nextOffset: Option<u64>,
  truncated: bool,
}
```

- `beforeSeq=None` 请求不晚于 `throughSeq` 的最新页；传入游标时请求严格早于该游标的一页。初始 `throughSeq` 必须使用订阅返回的 `headSeq`；旧页请求沿用当前快照上界。
- `session_timeline_forward_page` 返回严格大于 `afterSeq` 且不晚于可选 `throughSeq` 的一页，并携带本页实际观察到的水位。连续追赶时，客户端只推进到连续收到的最大 `seq`，直至追到水位。
- 存储层以 `(session_id, seq)` 主键逆序限量读取，再恢复为 seq 升序返回；不得先读取全量事件后再在内存切片。
- 初始建议页大小为 80 个显示事件，服务端硬上限为 200；实现计划须通过基准验证并把这两个值集中为可调常量。
- 页边界允许落在一个回合内部。渲染器把边界处的工具链标记为“本回合较早步骤尚未加载”，加载上一页后再合并，不得为了补齐一个回合而无界扩页。

### `TimelineEvent`

列表接口不返回无显示必要的大型 `data` 正文。每个 `TimelineEvent` 至少有稳定的 `id`、`sessionId`、`seq`、`kind`、`createdAt`、`runId?`、`callId?`、`approvalId?`、状态和受限的显示摘要；工具调用/结果以相同 `callId` 关联，审批请求/处理结果以相同 `approvalId` 关联，回合开始/结束与消息以 `runId` 关联。这些关联键即使分处不同页也不能被摘要投影省略。

原始事件尚未具备某个关联键时，投影层必须在事件写入或增量索引时依据当时的运行上下文有界地派生，或明确返回缺失值并降级为单卡展示；不得为一次分页请求反向扫描整段会话来推断 `runId`。若需要持久辅助索引，其键必须是事件 id/seq，且只能作为可重建投影，不能取代原始事件日志。

它返回的渲染摘要如下：

| 事件类型 | 列表载荷 | 展开后载荷 |
|---|---|---|
| 用户/助手消息 | 限字节正文预览、附件元数据、必要状态；助手 reasoning 默认折叠 | 按字段分块读取完整正文 |
| `tool.call` / `tool.result` | 工具名、状态、路径、短摘要、可展开标记、已知 diff 统计 | 参数、完整工具输出、完整 unified diff、artifact 引用 |
| 终端/后台任务 | 命令、状态、退出码、短摘要 | 完整 stdout/stderr 或 artifact 内容 |
| 计划、目标、审批、子代理 | 当前卡片所需的结构化摘要和稳定关联键 | 限字节详情字段或原始事件元数据 |

列表摘要中的正文大小必须有明确上限；超过上限时携带 `truncated`、`detailAvailable` 与字段 `payloadRef`，而不是静默截断或将完整内容传入 UI。`TimelineEventDetail` 返回元数据、受限的标量预览和可读取字段的 `payloadRef`，不直接返回完整 `SessionEvent` 或任意长度字符串。`session_timeline_payload_chunk` 按 `sessionId + eventId + field` 校验归属，单次 `limitBytes` 受服务端硬上限限制，并以 `nextOffset`、`totalBytes` 和 `truncated` 让客户端显式继续读取。原始 `SessionEvent` 仍只在存储/导出等既有边界内使用。

## 客户端状态与渲染规则

### 有界缓存

- 活动会话缓存当前页、向上加载的有限页和当前展开卡片的详情；页数与详情缓存均设 LRU 上限，并同时设置 resident `TimelineEvent` 数量、详情项数和序列化字节总额三种预算。任一预算触顶时优先逐出最久未见的详情、Markdown AST 和语法高亮缓存，再逐出整页。
- 非活动会话最多保留滚动锚点、最新游标、加载状态和轻量会话摘要；切出后释放事件正文、Markdown AST、diff 行和工具输出。
- 建议初值为“活动会话至多 3 页、最多 2 个非活动会话保留轻量元数据”。这些是受基准调整的产品常量，不得散落在组件状态中。
- 收到非活动会话的新 durable event 时仅失效其最新页和侧栏摘要，不把正文写入缓存；重新激活时通过快照订阅和有界前向页刷新最新页。

### 可变高度虚拟时间线

- 以可见回合/事件卡片作为虚拟化单元，保留视窗上下少量 overscan，不为历史全量创建 React DOM 或 GPUI 元素树。
- 消息、工具链和最终回答的关联按 seq 在 resident window 内投影；跨页边界使用显式 continuation 卡片，而不是依赖全历史数组。
- 顶部接近阈值时加载旧页。插入旧页前记录首个可见卡片的 id 与偏移，插入后恢复该锚点，禁止画面跳跃。
- 自动跟随只在用户本来位于底部附近时启用；用户上滚阅读历史后，新事件只显示“有新消息”提示，不强制 `scrollIntoView`。

### 流式消息

- IPC/订阅继续接收全部 delta，但 UI 在 animation frame 或短时间窗内合并多个 delta 后提交一次可见状态。
- 流式中显示轻量纯文本/等宽预览，不对每个 token 重跑完整 GFM 解析。临时流卡只保留受预算约束的前缀预览与末尾窗口；超出部分写入 durable 事件或暂存的 payload 引用，不能在 UI 字符串中无限追加。
- 收到 durable `assistant.message` 后，临时流卡原子替换为带 `payloadRef` 的最终 Markdown 卡片。首次展开可按块增量展示正文，但完整文本、Markdown AST 和高亮结果必须受详情字节预算约束。
- 去重索引和订阅追赶状态不能随输出 token 数无界增长；只保留请求 id、最后连续 index 与一个小型乱序窗口，完成或取消后立即释放。

### Markdown、代码与 diff

- Markdown AST 与语法高亮结果只归属于已挂载卡片或 LRU 详情缓存；卡片卸载即释放。
- edit/write 工具卡默认显示文件、状态和增删统计，完整 diff 仅在用户展开或右侧预览请求时生成/读取。
- 大型终端输出、浏览器快照和 artifact 一律以摘要或 artifact 引用进入列表，不能作为 `TimelinePage` 常驻字符串。

## 与 GPUI 的关系

GPUI 可以通过自绘文本和虚拟列表降低 WebView2/DOM 的基础与节点开销，但不能替代分页、载荷分层、LRU 和流式节流。后续 GPUI POC 必须直接消费同一 `TimelinePage` 与详情模型，并满足以下约束：

- GPUI 内存中只保留当前可视卡片、overscan 和已定义缓存预算。
- Markdown/代码块的布局、着色缓存和文本选择状态随卡片卸载释放。
- 切换会话不因 Rust 侧直接访问 SQLite 而绕过时间线页接口。

只有在现有前端已具备有界时间线并获得基准数据后，才能量化 WebView2 的基础开销是否仍足以证明整体迁移的收益。

## 一致性、并发与错误处理

- 分页游标使用持久 `seq`，不使用数组下标或 UI 生成 id；重复页与并发返回结果以事件 id 去重并按 seq 排序。
- 初始订阅、快照页、缓冲事件与前向补拉遵循“订阅注册 → 固定 `headSeq` 快照 → 按 seq 合并缓冲/补拉”的顺序。任何缺口、缓冲溢出或重连都必须进入有界前向补拉；不得假设事件到达顺序天然连续。
- 切换会话或工作区时，旧查询结果必须通过请求 generation/session id 校验后才能写入缓存。
- 新事件到达与加载旧页并发时，最新页按 `seq` 合并；旧页不会覆盖更高 seq 的实时事件。
- `session_timeline_event_detail` 读取失败时保留摘要卡，显示可重试错误；不得导致整个时间线重拉全量历史。
- 存储数据损坏、缺失或未知事件种类沿用现有错误/占位语义，且不能阻塞同页其余事件展示。

## 验证与基准

实现前先建立不含敏感内容的可重复长会话 fixture：包含普通消息、长 Markdown、多个工具调用、较大工具结果、diff、终端输出、压缩摘要和跨页回合。fixture 的事件数和字节分布须依据真实长程任务的采样记录确定。

至少验证：

1. 分页无重复、无遗漏、seq 顺序稳定；跨页回合能正确显示 continuation 并在合并后恢复完整结构。工具 call/result、审批 request/resolved 和回合关联跨页时仍以稳定关联键正确配对。
2. 切换到长会话的查询结果条数、传输字节和首次挂载卡片数均受页大小限制。
3. 连续打开多个长会话后，非活动会话的完整正文已从客户端缓存释放。
4. 向上加载时滚动锚点不跳动；阅读历史时流式新消息不夺取滚动位置。
5. 在“订阅注册前后追加事件、快照读取期间追加事件、订阅缓冲溢出、重连”四种场景下，客户端都能通过有界前向页得到每一个 seq，且不会调用无界的 `session_events(after)`。
6. 单条超大助手消息、工具结果、diff 和流式输出均不会突破列表、详情和流卡的字节预算；分块读取能依 `nextOffset` 无重复地还原内容。
7. 流式持续输出时，重绘次数受节流窗口限制，完成后最终 Markdown、代码块和工具卡内容正确。
8. 导出、恢复、CLI/ACP 的 `session_events(after)` 行为不变；`/compact` 后仍可浏览和导出原始历史。

基准报告必须记录：事件总数与正文总字节、每页事件数和响应字节、切换冷/热耗时、滚动帧时间、应用进程与 WebView2 子进程的总工作集、客户端 resident event/detail/DOM（或 GPUI 元素）数量。先记录现状基线，再在同一机器、同一 fixture 下比较改造后数据。

## 实施顺序与风险控制

1. 在存储/运行时增加受限的页查询与详情查询，保留既有全量接口。
2. 为分页、摘要投影、跨页边界和详情权限编写单元/集成测试，并建立长会话 fixture。
3. 以 feature flag 接入桌面端分页缓存和可变高度虚拟时间线；出现问题可回退到既有读取路径。
4. 接入流式合帧、滚动锚定和工具详情惰性加载，完成真实会话基准。
5. 基于结果决定维持 Tauri/React，或令 GPUI POC 消费同一契约；UI 框架迁移另立规范。

主要风险是变量高度 Markdown 的测量误差、跨页回合聚合、工具详情按需读取后的状态竞争，以及既有前端测试对“全量立即可见”行为的隐性依赖。以上风险都应以页边界、generation 校验、LRU 预算和 fixture 回归测试约束，禁止用扩大缓存或退回全量加载规避。

## 待确认项

- 真实长程任务的代表性事件数、正文大小和目标硬件，以便冻结基准阈值。
- 初始页大小、活动/非活动缓存预算，以及工具摘要最大字节数。
- 用户是否需要在聊天区直接搜索全部历史。现有 `message_search` 只索引会话 id 与内容，不能定位到事件 id/seq；该能力不属于本期。若后续实施，须另行迁移 FTS schema，加入可定位的 `event_id`/`seq`、完成回填与重建，并以命中 seq 分页定位，不能预加载全部事件。
- 在有界时间线完成后的决策门槛：WebView2 的实际基础内存与 GPUI 重写成本达到何种比例时，才启动 GPUI UI 迁移规范。
