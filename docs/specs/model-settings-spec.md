# 模型设置：厂商→模型两级选择与 effort 档位规范

- 日期：2026-11-06（计划批准）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：模型设置两级结构（供应商→模型）、内置供应商与内置模型目录、reasoning effort 档位、按 API 自动检测模型列表、发送框级联选择、卸载"清理用户数据"勾选行为。

## 背景与目标

改造前发送框是全部已存模型的扁平下拉，不区分供应商、无 effort；设置弹窗中模型标识只能手填，供应商下没有模型列表；仓库内没有任何 reasoning effort 概念；卸载页"删除应用数据"勾选框因 `tauri.conf.json` 未设置 `deleteAppDataOnUninstall` 而从不出现（`nsis-hooks.nsh` 的清理逻辑永远不触发）。

参考 zcode（`D:\code\zcode`）的两层设计（内置只读目录 + 个人层覆盖），目标：

1. 先选厂商、再选模型：设置页按供应商维护模型列表；发送框改为 供应商 → 模型 → 档位 三级联下拉，选择跨重启保留。
2. 内置供应商（deepseek、bigmodel）名称/端点固定不可改删，新增启用/禁用开关，保留"复制后可编辑"。
3. 内置模型清单随程序分发，可启停、不可改删；内置模型固定走供应商的 Anthropic 协议端点（与 zcode 模板一致），需要其他协议时手动添加自定义模型。
4. 模型可 API 自动检测或手动录入；自定义供应商/模型支持增删改与启停。
5. effort 按模型声明档位，按协议映射请求参数。
6. 模型设置随升级保留；卸载时按"清理用户数据"勾选决定是否保留。

本规范替代[多协议接入规范](multi-protocol-spec.md)中"模型标识手动录入"的单层模型形态。

## 设计：两层目录 + 个人覆盖

内置目录在代码中（`builtin_providers()`），用户的全部"修改"落为个人覆盖存储（SQLite settings 的 `providers`/`models` 键），升级更新内置目录不丢失用户意图。来源标记（`builtin` 字段 + 目录成员关系）控制"可启停、不可改删"。

### 内置目录

| 供应商 | 内置模型 | effort 档位 | 协议端点 |
|---|---|---|---|
| deepseek | deepseek-flash、deepseek-v4-pro | disabled/enabled | Anthropic（`https://api.deepseek.com/anthropic`） |
| bigmodel（智谱 Coding Plan） | glm-5.3、glm-5.3-flash、glm-5.2、glm-5-turbo | low/high/max；disabled/high/max；disabled/enabled | Anthropic（`https://open.bigmodel.cn/api/anthropic`） |

档位与成员对照 zcode 内置目录 `config/provider/zcode-builtin.json` 的 modelRules/模板 `builtinModelIds`。

### 契约与存储

- `harness-contracts`：新增 `BuiltinModel`、`KNOWN_EFFORTS`（disabled/minimal/low/medium/high/max，有序）、`DefaultSelection`；`ProviderInfo` 增 `enabled`（默认 true）、`builtin_models`、`disabled_models`；`ModelConfig` 增 `efforts`、`effort`（None=跟随供应商默认）、`enabled`、`builtin`。全部 `#[serde(default)]`，旧 JSON 原样读入，SCHEMA_VERSION 保持 2，无需迁移。
- 内置供应商的个人覆盖只允许 `proxy`、`enabled`、`disabled_models` 三个字段变化；`builtin_models` 不落库，始终以程序内置为准。
- `ModelConfig.builtin=true` 的条目由服务层合成（id 形如 `builtin:bigmodel:glm-5.3`），拒绝经 `model.save` 修改。
- 停用语义：条目 `enabled=false` 或所属供应商停用时，`build_model` 报 `MODEL_DISABLED`；内置供应商缺密钥仍报 `MODEL_CREDENTIAL`。

### effort 请求编码（对齐 zcode modelApiRules）

`config.effort=Some(v)` 时按协议注入；None 不发送任何字段：

- Anthropic：`disabled/none` → `{"thinking":{"type":"disabled"}}`；其余 → `{"thinking":{"type":"adaptive"},"output_config":{"effort":v}}`
- ChatCompletions：`thinking.type`=disabled/enabled、`enable_thinking`、`reasoning_effort` 与 `reasoning.effort`（disabled/none→"none"，enabled→"high"，其余原值）
- Responses：`{"reasoning":{"effort":v}}`（disabled/none 时省略）

`ModelRoute` 指纹不含 effort，同模型换档位可复用协议回放。

### 模型列表自动检测

`model_fetch_available`（桌面）/`model.fetchAvailable`（JSON-RPC）：按供应商已存密钥请求模型列表——ChatCompletions/Responses 用 `GET {base}/models`（Bearer），Anthropic 用 `GET {base}/v1/models`（`x-api-key` + `anthropic-version`），解析 `data[].id` 去重排序；带供应商代理。端点不支持时错误透传，设置页保留手动录入。

### 桌面与 CLI

- 桌面命令：`session_send` 增 `effort` 参数（`build_model_with` 覆盖档位；后台任务/计划仍用 `build_model` 走模型默认档位）；新增 `model_fetch_available`、`default_selection_get/save`（KV `app.default_selection`）。
- CLI：`run`/`chat` 增 `--effort`（校验 ∈ 模型 efforts，`/model` 切换后档位保留）；`models list` 输出追加 efforts 列与来源/停用标记；JSON-RPC `session.send` 支持可选 `effort`，新增 `model.setBuiltinEnabled`。
- 发送框：供应商（enabled 且有可用模型）→ 该供应商下启用模型 → effort（首项"默认"=不发送；无档位隐藏）。启动从 `app.default_selection` 恢复，失效回退第一个可用模型。

### 卸载行为

Tauri 2.12 的 NSIS 卸载确认页**无条件提供**"Delete app data"复选框（模板变量 `$DeleteAppDataCheckboxState`，与升级模式变量 `$UpdateMode` 一并由模板定义），无需任何配置字段——`deleteAppDataOnUninstall` 是 Tauri 3 的字段，在本项目锁定的 tauri-build 2.7.4 与 @tauri-apps/cli 2.12.4 双侧 schema 中均不存在，写入会导致构建失败。勾选且非升级模式时，模板先删除 `$APPDATA\BUNDLEID`/`$LOCALAPPDATA\BUNDLEID`，既有 `nsis-hooks.nsh` 借用同一勾选状态执行 `--purge-local-data`（删除 `%LOCALAPPDATA%\RustHarness` 与系统凭据）并跳过模板的递归删除；不勾选则全部保留。模型/供应商设置存于 SQLite 与系统凭据管理器，升级天然保留。

## 边界与风险

- 自动检测依赖供应商支持 `/models` 端点；Coding Plan 代理端点如不支持则手动录入兜底。
- effort 字段对严格校验的自定义端点可能返回 400；用户可在模型设置清空档位规避（不发 effort 字段）。
- 内置模型固定 Anthropic 协议；需要其他协议时用"添加模型"手动录入同标识模型。
