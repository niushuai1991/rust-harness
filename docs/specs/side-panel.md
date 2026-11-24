# 右侧面板（文件预览、改动审阅、网页浏览、计划抽屉与侧聊）规范

- 日期：2026-11-07（计划拟制）；2026-11-12（网页浏览）修订；2026-11-16（网页视图承载可视浏览器）修订；2026-11-20（计划视图联动与消息流计划卡）修订；2026-11-23（本地 PDF 预览）修订；2026-11-24（侧聊抽屉）修订
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：主界面右侧面板统一承载「更改审阅」、工作副本文件预览、会话网页浏览、只读计划抽屉与 `/side` 侧聊聊天；计划模式与抽屉改动的实现和验证仍在进行。

## 背景与范围

桌面端查看工作副本改动此前只有居中 Modal `DiffPanel`：before/after 原文双栏并排，无行级差异；聊天流中的 write/edit 工具卡仅渲染参数 JSON，无法直接查看目标文件。本规范新增主界面右侧面板，统一承载「更改审阅」「文件预览」、会话网页和计划抽屉。

范围：

- 右侧面板容器：与左栏对称的 `aside`，宽度可拖拽并记忆，Escape 关闭。
- 更改视图：原 DiffPanel 的审阅与同步能力整体迁入，文件行升级为手风琴 + 行级 unified diff。
- 预览视图：工作副本单文件文本预览（32KiB 截断、二进制占位）。
- 计划视图：只读汇总跨会话计划、目标和待办，可查看历史并跳转会话，不直接修改内容，详见[计划/目标管理面板规范](plan-goal-panel.md)。明确审批与拒绝或反馈遵循[会话计划模式规范](plan-mode.md)，不由只读抽屉代替。
- 侧聊视图：`/side`（别名 `/btw`）派生的侧聊会话在抽屉内独立聊天，主会话保持可见；入口、切回、审批与提问归属见下文「侧聊抽屉」。
- 联动：write/edit 工具卡文件按钮打开预览；`/diff` 与顶栏「查看更改」开关面板；面板打开期间轮询刷新。

明确不做（后续迭代）：

- 语法高亮（聊天代码块同样没有，待统一选型）。
- 除 PDF 外的图片/二进制内容预览、Office 文档预览（capability-matrix 已标注未实现）。
- 文件树、多 tab 预览、diff 虚拟滚动、文件系统 watch 事件。

交互取舍：改动审查位于侧栏、点击工具卡才打开预览、agent 写文件后不自动弹出、截断时显示提示。

## 设计

### 后端

- unified diff 渲染自 `harness-cli/src/diffview.rs` 迁移为共享模块 `harness-platform/src/diffview.rs`（`sides`/`render`/`fallback_note`/`MAX_DIFF_BYTES`/`MAX_DIFF_LINES`），CLI re-export 保持原路径可用，CLI 专属的 `ColorChoice`/`select`/`write_listing` 留在 CLI；CLI 不再直接依赖 `similar`。渲染语义不变，沿用 [CLI 命令行规范](cli.md) 的 diff 契约（1MiB 阈值、2000 行显示截断、二进制不渲染正文）。
- `WorkspaceManager::diff_view(id, path)`：单文件改动视图，复用 `changes()` 后按 path 过滤，渲染 unified diff；未变更文件返回 `NOT_FOUND`。冲突文件照常渲染（对比的是基线与副本，apply 侧哈希校验兜底）。
- `harness_platform::read_file_bounded_sized(root, relative, max)`：有界读取并附带文件完整字节数与实际读取字节数，供截断判定。
- 契约新增（`harness-contracts`，specta 导出）：
  - `WorkspaceFileDiff { path, unified: Option<String> }`——`unified=None` 表示二进制或超限。
  - `WorkspaceFilePreview { path, content, truncated, binary, sizeBytes }`——32KiB 截断；`binary` 按内容含 NUL 判定，二进制时 content 为空串，前端显示占位。
- 新 Tauri 命令：`workspace_file_diff(workspaceId, path)`（spawn_blocking）、`workspace_file_preview(workspaceId, path)`；`FileChange` 契约不变，CLI 不受影响。

### 前端

- `sidePanelStore.ts`：独立 zustand store `useSidePanel`（`open`/`view: 'changes'|'preview'|'web'|'plans'|'pdf'|'chat'`/`previewPath`/`webUrl`/`pdfPath`/`chatSessionId`/`chatParentSessionId`/`chatDrafts`/`chatInitial` + `openChanges`/`openPreview`/`openWeb`/`openPlans`/`openPdf`/`openChat`/`setChatDraft`/`consumeChatInitial`/`closeChat`/`setView`/`toggle`/`close`/`reset`）。预览路径与侧聊会话由深层 EventCard 触发，故独立于 App 局部 state；切换工作区时 `reset()`。
- `SidePanel.tsx`：容器。宽度手写 pointer 拖拽（`--panel` 左缘 6px 热区，280px ~ 60vw），`localStorage['side-panel-width']` 记忆；Escape 关闭（忽略来自输入框/文本域的按键与已 `defaultPrevented` 的事件）；header 为「更改/预览/计划」及有 PDF、网页、侧聊时的对应 tab + 关闭按钮；关闭即卸载（数据重挂载重拉）。
- `ChangesView.tsx`：自 DiffPanel 迁移。保留 `workspace_diff` 加载、勾选、二次确认、`workspace_apply`（reviewed 快照 + STALE_DIFF 校验）、冲突禁选与空态；文件行升级为手风琴：
  - 展开时懒调 `workspace_file_diff` 渲染 `DiffView`，并从 diff 文本统计 +n/−n 显示于文件行。
  - conflict 文件勾选框禁用，展开区顶部显示处理提示条。
  - 打开期间 5s 轮询 `workspace_diff`（刷新保留用户勾选，新增非冲突文件自动勾选）；手动刷新重置勾选并清空差异缓存，对保持展开的文件自动重算。
- `DiffView.tsx`：unified diff 解析纯函数 `parseUnifiedDiff`（跳过首个 hunk 前的 `---`/`+++` 文件头，hunk 后 `-`/`+` 开头行一律按正文处理，空行按上下文推进行号，尾部换行不产生幽灵行，截断提示行归为 note）+ 渲染组件（双行号列、增删行着色、hunk/note 整行）；`diffStats` 供统计。
- `PreviewView.tsx`：`workspace_file_preview` 内容 + 行号 gutter（sticky）+ 截断提示条 + 二进制占位；未选文件显示引导空态。
- `App.tsx`：`Panel` 类型移除 `'diff'`，删除 Modal 版 DiffPanel；顶栏「查看更改」与 `/diff` 斜杠命令改走 `useSidePanel`；`EventCard` 对内置 `write`/`edit` 工具从 `arguments.path` 提取路径渲染文件按钮（`preventDefault` 防止折叠工具卡），点击 `openPreview`；挂载右侧面板并提供会话跳转回调，切换工作区重置面板；「计划与目标」入口打开只读计划视图。
- 样式沿用现有 design token（`--border`/`--panel`/`--accent`），diff 增删行用项目既有红绿语义色衍生，无新增依赖。


## 计划视图联动与消息流计划卡（2026-11-20 修订）

- 联动：`receiveEvent` 收到 `plan_write` 产生的 `plan.state` 实时事件后自动弹出「计划」视图（对齐可视浏览器「模型操作时自动弹出网页视图」先例）；收到任意 `todo.state`/`plan.state`/`goal.state` 时递增 `planPanelRevision` 并经 `SidePanel` 透传给 `PlansPanel`，打开期间事件驱动重拉 `plan_overview`（无轮询）。
- 事件覆盖边界：`session_send` 的实时通道按会话过滤，联动仅覆盖当前交互会话；子代理/自动化会话的写入不触发实时联动，打开抽屉时 `plan_overview` 重拉保证最新。
- 自动弹出为强切语义：面板在其他视图（更改/预览/网页）时切换到「计划」，已在计划视图时幂等。
- 消息流 `plan.state`/`goal.state` 事件卡重构为 zcode 式计划 Markdown 卡（对齐 zcode `switch-mode` 渲染）：白卡（12px 圆角 + 1px 边框）+ 头部（NotepadText/Target 图标 +「计划/目标」+ 复制按钮，复制带 ✓ 反馈）+ 正文预览（256px 上限 + 底部渐隐 mask）+ 悬浮胶囊按钮「查看完整计划 →」；整卡点击打开「计划」抽屉。
- 消息流 `todo.state` 事件卡对齐 zcode 待办卡：无框折叠头（图标 +「待办清单」+ 当前项 + `完成/总数`）+ 展开态淡灰内衬状态行（✓ 划线 / → 当前项 / ○ 待办）；实时收到的新卡默认展开（`liveTodoIds` 集合判定，上限 64 整体清空），历史回放收起。
- 样式新增 `.zc-plan`/`.zc-card` 系列，全部沿用现有 design token，无新增依赖；卡片样式详见 `frontend/style.css` 末尾注释块。

## 边界

- diff 懒加载与 apply 之间副本可能变化：apply 侧 reviewed 快照哈希校验（STALE_DIFF）兜底，预览仅展示。
- 展开的 diff 与轮询刷新不联动（懒加载缓存），手动刷新重算；后续如需实时可在 tool.result 事件上挂钩。
- 大文件沿用 1MiB/2000 行阈值，超限显示占位说明；无虚拟滚动。
- 面板关闭即卸载，重开重新拉取；预览路径与视图状态跨开关保留（同工作区内）。


## 会话网页浏览（2026-11-12 修订）

- 会话中助手 Markdown 和用户纯文本中的 HTTP/HTTPS 链接点击后打开右侧「网页」视图；用户文本保留原格式。链接右键提供复制地址和系统默认浏览器打开，并显示操作失败。
- 网页由独立 WebView2 子视图加载，不改变主界面导航；同一视图承载网页内跳转与新窗口请求。面板尺寸和窗口变化时同步子视图位置，其他视图、面板关闭及应用弹窗出现时隐藏或销毁子视图。
- 仅允许 HTTP/HTTPS 导航；应用自身内部地址不能在网页视图打开。远程网页不授予应用命令权限。网页加载失败时提示并提供系统浏览器入口；部分网站的登录或兼容性取决于 WebView2。

### 可视浏览器（2026-11-16 修订）

- 「网页」视图升级为内嵌浏览器：面板 WebView2 同时是 `browser_*` 工具的操作对象（人机共用同一实例），实现路径、并发与安全边界（共用 user data folder、逐次审批把守）见[内置浏览器集成规范](browser.md)。
- 地址栏增加后退/前进/刷新工具栏（按历史可用性禁用），地址实时跟随实际导航；`web_panel_open` 幂等（不重建子视图、同址不重复导航）。
- 子视图持久化：面板关闭仅隐藏不销毁，模型操作时自动弹出「网页」视图；加载失败提示与系统浏览器入口不变，ESC 关闭面板语义不变。


## 本地 PDF 预览（2026-11-23 修订）

- 点击会话消息中的本地 PDF Markdown 链接、file 链接，或用户消息中的 Windows PDF 路径，在右侧 PDF 标签页预览；HTTP/HTTPS PDF 链接仍打开网页视图。相对路径按当前会话执行目录解析且不能越界；无项目会话同样可预览。
- 使用 PDF.js 离线渲染，提供上一页、下一页和 25%–400% 缩放；加载失败时显示错误。字符映射和标准字体随前端资源发布。
- 宿主按用户打开的链接读取文件，拒绝 Windows UNC 网络共享路径及重解析点（包括符号链接和目录联接）、非 PDF 文件和超过 32 MiB 的文件；映射盘符可能指向网络位置；校验 PDF 文件头。预览只读，不支持编辑、注释和 PDF 内链接。


## 侧聊抽屉（2026-11-24 修订）

- 目标：`/side`（别名 `/btw`）派生的侧聊在右侧面板「侧聊」视图中独立聊天，主会话保持可见；替代此前“派生后把主界面切到侧聊会话”的桌面行为。侧聊后端语义（`fork_as_side`、`side.boundary`、继承历史仅模型可见）见[斜杠命令规范](slash-commands.md)。
- 单一视图与常驻挂载：`SideView` 增加 `'chat'`；面板一次只显示一个视图，但侧聊组件在面板打开期间始终挂载——切到更改/预览/计划/网页/PDF 时隐藏而非卸载，切回保留未发送草稿与滚动位置。浏览容器在聊天态改为 `overflow:hidden`，消息区与输入框各自滚动/固定。
- 会话归属：`chatSessionId`/`chatParentSessionId` 由打开动作记录；抽屉一次只承载一个侧聊，多个侧聊经父会话时间线的「已创建侧聊」登记卡（`subagent.created{source:"side"}`）或子代理面板重新打开并替换当前侧聊。关闭面板或切换工作区（`reset()`）不删除侧聊会话；关闭面板后重开恢复上一次侧聊。
- 聊天能力：时间线按 `side.boundary` 从空白开始，支持向上加载更早、自动跟随、流式文本与思考指示、运行中转向（`session_steer`）与停止（`session_cancel`）；输入框只发送纯文本，使用主界面当前模型与推理档位，不支持附件、引用、斜杠命令菜单与回合导航（`turn_navigator_page` 未按侧聊边界过滤，会混入继承历史）。
- 审批与提问（按会话归属到各自面板）：侧聊产生的审批在抽屉内以内联卡显示，提问复用 `QuestionCard` 显示在侧聊输入框上方；抽屉「侧聊」tab 在有待处理审批/提问时显示提示点。主窗口审批弹窗在侧聊视图可见时跳过该侧聊的审批，避免重复；侧聊视图不可见（切到其他视图或关闭面板）时回落到主窗口全局弹窗，保证提示不丢。其他子代理/自动化会话维持原有全局弹窗行为。
- 首条文字：`/side 文字` 派生后先打开抽屉，待侧聊时间线激活后再发送文字，避免实时事件早于 timeline 建立被丢弃；主会话输入框只清除命令本身，不清附件与引用。
- 入口与切回：`/side`、父会话「已创建侧聊」卡片的「查看会话」、子代理面板中的侧聊行均打开抽屉；普通子代理仍切换主会话。主会话不因侧聊派生或聊天而切换。
- 边界：侧聊与父会话共享同一执行根，并行工具写同一文件可能冲突；侧聊继承派生时刻的审批策略，父会话后续改档不自动同步；侧聊仍作为父会话子会话参与子代理面板与相关检查。
