# 右侧面板（文件预览、改动审阅与网页浏览）规范

- 日期：2026-11-07（计划拟制）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：主界面右侧面板统一承载「更改审阅」、工作副本文件预览与会话网页浏览；参考 zcode 右侧面板的形态取舍。

## 背景与范围

桌面端查看工作副本改动此前只有居中 Modal `DiffPanel`：before/after 原文双栏并排，无行级差异；聊天流中的 write/edit 工具卡仅渲染参数 JSON，无法直接查看目标文件。本规范新增主界面右侧面板，统一承载「更改审阅」与「文件预览」。

范围：

- 右侧面板容器：与左栏对称的 `aside`，宽度可拖拽并记忆，Escape 关闭。
- 更改视图：原 DiffPanel 的审阅与同步能力整体迁入，文件行升级为手风琴 + 行级 unified diff。
- 预览视图：工作副本单文件文本预览（32KiB 截断、二进制占位）。
- 联动：write/edit 工具卡文件按钮打开预览；`/diff` 与顶栏「查看更改」开关面板；面板打开期间轮询刷新。

明确不做（后续迭代）：

- 语法高亮（聊天代码块同样没有，待统一选型）。
- 图片/二进制内容预览、Office 文档预览（capability-matrix 已标注未实现）。
- 文件树、多 tab 预览、diff 虚拟滚动、文件系统 watch 事件。

参考 zcode（本地 `D:\code\zcode`）的取舍：改动审查常驻侧栏、点击工具卡才打开预览、agent 写文件后克制不自动弹出、截断显示提示而非半截内容。

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

- `sidePanelStore.ts`：独立 zustand store `useSidePanel`（`open`/`view: 'changes'|'preview'`/`previewPath` + `openChanges`/`openPreview`/`setView`/`toggle`/`close`/`reset`）。预览路径由深层 EventCard 触发，故独立于 App 局部 state；切换工作区时 `reset()`。
- `SidePanel.tsx`：容器。宽度手写 pointer 拖拽（`--panel` 左缘 6px 热区，280px ~ 60vw），`localStorage['side-panel-width']` 记忆；Escape 关闭；header 为「更改/预览」tab + 关闭按钮；关闭即卸载（数据重挂载重拉）。
- `ChangesView.tsx`：自 DiffPanel 迁移。保留 `workspace_diff` 加载、勾选、二次确认、`workspace_apply`（reviewed 快照 + STALE_DIFF 校验）、冲突禁选与空态；文件行升级为手风琴：
  - 展开时懒调 `workspace_file_diff` 渲染 `DiffView`，并从 diff 文本统计 +n/−n 显示于文件行。
  - conflict 文件勾选框禁用，展开区顶部显示处理提示条。
  - 打开期间 5s 轮询 `workspace_diff`（刷新保留用户勾选，新增非冲突文件自动勾选）；手动刷新重置勾选并清空差异缓存，对保持展开的文件自动重算。
- `DiffView.tsx`：unified diff 解析纯函数 `parseUnifiedDiff`（跳过首个 hunk 前的 `---`/`+++` 文件头，hunk 后 `-`/`+` 开头行一律按正文处理，空行按上下文推进行号，尾部换行不产生幽灵行，截断提示行归为 note）+ 渲染组件（双行号列、增删行着色、hunk/note 整行）；`diffStats` 供统计。
- `PreviewView.tsx`：`workspace_file_preview` 内容 + 行号 gutter（sticky）+ 截断提示条 + 二进制占位；未选文件显示引导空态。
- `App.tsx`：`Panel` 类型移除 `'diff'`，删除 Modal 版 DiffPanel；顶栏「查看更改」与 `/diff` 斜杠命令改走 `useSidePanel`；`EventCard` 对内置 `write`/`edit` 工具从 `arguments.path` 提取路径渲染文件按钮（`preventDefault` 防止折叠工具卡），点击 `openPreview`；挂载 `<SidePanel workspaceId>`，切换工作区重置面板。
- 样式沿用现有 design token（`--border`/`--panel`/`--accent`），diff 增删行用项目既有红绿语义色衍生，无新增依赖。

## 边界

- diff 懒加载与 apply 之间副本可能变化：apply 侧 reviewed 快照哈希校验（STALE_DIFF）兜底，预览仅展示。
- 展开的 diff 与轮询刷新不联动（懒加载缓存），手动刷新重算；后续如需实时可在 tool.result 事件上挂钩。
- 大文件沿用 1MiB/2000 行阈值，超限显示占位说明；无虚拟滚动。
- 面板关闭即卸载，重开重新拉取；预览路径与视图状态跨开关保留（同工作区内）。


## 会话网页浏览（2026-11-12 修订）

- 会话中助手 Markdown 和用户纯文本中的 HTTP/HTTPS 链接点击后打开右侧「网页」视图；用户文本保留原格式。链接右键提供复制地址和系统默认浏览器打开，并显示操作失败。
- 网页由独立 WebView2 子视图加载，不改变主界面导航；同一视图承载网页内跳转与新窗口请求。面板尺寸和窗口变化时同步子视图位置，其他视图、面板关闭及应用弹窗出现时隐藏或销毁子视图。
- 仅允许 HTTP/HTTPS 导航；应用自身内部地址不能在网页视图打开。远程网页不授予应用命令权限。网页加载失败时提示并提供系统浏览器入口；部分网站的登录或兼容性取决于 WebView2。
