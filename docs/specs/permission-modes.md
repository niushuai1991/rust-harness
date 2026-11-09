# 权限模式（五档审批策略）

- 日期：2026-11-09（计划批准）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：五档权限模式（审批策略）统一定义与四端接入：CLI `--approval`、能力预设 `approval` 字段、ACP modes、桌面发送框权限模式选择器；`guarded` 档的危险 shell 命令识别规则。设计参考 codex approval presets 与 zcode permissionMode（plan/build/edit/guarded/yolo）。

## 背景与决策

此前 CLI 已有三档审批策略（`--approval ask|auto|never`，见 [CLI 规范](cli.md)），但桌面端永远逐次弹审批卡片，每条命令都要人工批准；且不存在中间档位（"自动编辑但命令仍确认"、"自主执行但危险命令确认"）。

2026-11-09 用户确认三项决策：

1. **保持副本边界**：所有档位只改审批层放行行为；一切 Agent 文件/shell 操作仍只落隔离工作副本，进原项目仍需 diff 审阅后 `apply`，Windows 受限令牌沙箱不变（与 [总体规范](overall.md) 安全不变量一致）。
2. **五档**：只读 / 每次确认 / 自动编辑 / 完全访问 / 自主（guarded）。
3. **预设 approval 桌面保持独立**：预设的 approval 字段仍仅 CLI/JSON-RPC/ACP 生效；桌面权限模式选择器独立于预设选择（[配置预设规范](config-presets.md) 中「桌面端审批策略接管」非目标据此修订）。

## 档位定义

统一值集 `KNOWN_APPROVALS = ["ask","edit","auto","never","guarded"]`（harness-contracts），决策矩阵唯一实现在 `harness_contracts::ApprovalPolicy::decide`，四端共用：

| 值 | UI 标签 | 决策（`decide` 返回） | 说明 |
|---|---|---|---|
| `never` | 只读 | 需审批工具一律 `Some(false)` | 免审批只读白名单仍可用（read/search 等） |
| `ask` | 每次确认（默认） | 一律 `None` 交互询问 | 现行为 |
| `edit` | 自动编辑 | tool ∈ {`write`,`edit`} → `Some(true)`，其余 `None` | 写文件落副本本身可审阅，先放行；shell 等仍确认 |
| `auto` | 完全访问 | 一律 `Some(true)` | 改动仍只落副本；shell 仍受受限令牌沙箱约束 |
| `guarded` | 自主 | tool ≠ `shell` → `Some(true)`；`shell` 命中[危险命令规则](#危险命令规则-guarded-v1) → `None`，否则 `Some(true)` | 危险命令的审批理由点名规则（如「自主模式检测到危险命令（git-push-force）」） |

- `decide` 返回 `Some(allow)` 时审批不经 pending 等待直接结算；事件流仍完整记录 `approval.request` 与 `approval.resolved`，回放语义不变。
- `None` 时回落宿主交互裁决：CLI 终端 y/n、桌面审批卡片、ACP `session/request_permission`。
- `shell` 命令缺失或为空白时按需确认处理（保守）。

## 会话策略与运行时默认档

- `Runtime::set_approval_policy(session, Option<ApprovalPolicy>)`：会话显式策略（内存态，不落库，重启丢失）。
- `Runtime::set_default_approval_policy(Option<ApprovalPolicy>)`：运行时默认档，覆盖未显式设置的会话——含定时任务/自动化投递的会话与后续新建子代理。
- 会话生效策略 = 会话显式设置 > 运行时默认 > `None`（宿主交互裁决）。
- 子代理继承：`agent_spawn`（runtime 内置）与 `agent_fork`（orchestration）创建子会话时继承发起会话的生效策略，避免策略在子代理处失效。

## 危险命令规则（guarded v1）

`harness_contracts::dangerous_shell_rule(command) -> Option<&'static str>`（命中返回规则标识）：纯文本启发式，不解析完整 shell 语法。

**结构处理**：

- 按 `&&`/`||`/`;`/`&`/`|`/换行分段逐段评估；分段符在引号外才生效。
- 剥一层常见包装命令（`cmd`/`powershell`/`pwsh`/`bash`/`sh`/`wsl`/`start`）后评估内层，最多三层。
- git 先跳过全局选项（`-C`/`-c`/`--git-dir` 等含取值）再识别子命令。
- 含 `--help`/`-h`/`/?` 的段不拦（zcode 同语义；已知局限：POSIX `shutdown -h now` 亦被豁免）。
- 引号外出现 `%VAR%`、`$var`/`$()`、反引号视为动态片段，保守判 `unsupported`（需确认）；单引号内不视为动态。
- 命令超过 4096 字符判 `unsupported`。

**规则表**（v1）：

| 规则标识 | 触发 |
|---|---|
| `git-reset-hard` | `git reset --hard` |
| `git-clean-force` | `git clean` 带 `-f`/`--force` 或含 `f` 的短旗标组合（`-fd` 等） |
| `git-push-force` | `git push` 带 `-f`/`--force`/`--force-with-lease`/`--mirror` 或 `+refspec`；`--dry-run` 豁免 |
| `git-stash-clear` | `git stash drop`/`git stash clear` |
| `git-worktree-force-remove` | `git worktree remove --force` |
| `git-discard-tree` | `git checkout --`/`git checkout .`/`git restore`（丢弃工作树） |
| `remove-critical-path` | `rm` 递归且（多操作数或含 glob）；根路径/盘符根/`~` 无论操作数个数；cmd `rd`/`rmdir`/`del`/`erase` 带 `/s`；PowerShell `Remove-Item` 带 `-Recurse` |
| `system-storage` | `dd`/`mkfs*`/`mount`/`umount`/`format`/`diskpart`/`shutdown`/`reg delete`/`cipher /w`；`robocopy` 带 `/mir`/`/purge` |
| `unsupported` | 动态片段或超长命令（保守需确认） |

单字面路径的 `rm -rf build`（单操作数、无 glob、非根）不拦（zcode 同语义）。规则覆盖 Windows（cmd/PowerShell）与 POSIX 常见写法，大小写不敏感；识别为误拦时用户确认一次即可继续。

## 各端接入

**CLI（run/chat）**：`--approval` 值集扩为 `ask|edit|auto|never|guarded`；TTY 规则修订为交互档（`ask`/`edit`/`guarded`）需要 TTY，`auto`/`never` 非 TTY 可用。`/approval` 运行中切换继续生效。单次决策走 `ApprovalPolicy::decide`，`None` 才读 stdin y/n。

**能力预设**：`approval` 字段值集随 `KNOWN_APPROVALS` 扩为五档；解析层级不变（显式 `--approval` > 预设 > TTY 规则）；预设 approval 仍仅 CLI/JSON-RPC/ACP 生效。

**ACP**：modes 即五档（`session/new`、`session/load` 响应携带，`session/set_mode` 切换）；`approval.request` 按生效档 `decide`，`None` 才发 `session/request_permission`。

**桌面**：

- 发送框新增「权限模式」选择器（五项，含一句话说明），选择即 `approval_mode_save` 保存为默认档（KV `app.default_approval_mode`，跨重启保留）并对当前会话即时生效；会话内切换在运行时记录会话显式覆盖，其他会话跟随默认档。
- 启动时读取 KV 设为运行时默认档，定时任务/自动化投递的会话同样生效。
- 切到 `auto` 时放行该会话（含子代理）现存 pending 审批，切到 `never` 时拒绝，其余档位保留 pending 交用户处理。
- 桌面权限模式与预设 approval 互相独立；审批卡片（Modal）交互不变。

## 边界与非目标

- 副本+apply 边界、Windows 沙箱、免审批只读白名单、审批 600 秒超时兜底均不变。
- 不做 codex 式沙箱档位（workspace-write/danger-full-access）与"直写原项目"档位；如需绕过副本另行立项。
- 危险命令识别基于字符串启发式，动态片段保守询问兜底；不承诺穷尽危险命令。
- 会话策略不落库：会话内切换重启后回到默认档（与 CLI `/approval` 会话内切换同语义）。
- JSON-RPC（`harness rpc`）保持 `approval.list`/`approval.resolve` 交互式通道，不新增策略参数。
