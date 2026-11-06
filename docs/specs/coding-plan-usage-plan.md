# 智谱 Coding Plan 剩余额度与模型设置布局计划

- 文档日期：2026-11-07（计划批准）
- 状态：已实施，Rust 与前端测试通过（2026-11-07）；桌面 UI 与真实额度展示抽查待用户
- 范围：智谱 Coding Plan 套餐名与剩余额度展示（quota/limit 接口）、模型设置面板布局重排（左栏分组导航/右栏详情视图/模型编辑弹窗化）。

## 背景与目标

模型设置已具备供应商→模型两级结构（见模型设置计划），但仍有两点不足：

1. 智谱 GLM Coding Plan 的套餐名与剩余额度无处查看；
2. 设置面板左栏混排供应商与模型列表，模型编辑表单占用右栏表单区，信息密度低。

参考 zcode 源码（`packages/services/src/usage-stats`、`packages/ui/src/settings/model-provider-section`）的额度接口与布局，目标：设置面板显示智谱套餐名与三张剩余额度卡（5小时/每周/工具调用），布局重排为左栏分组导航 + 右栏详情视图，模型编辑改为弹窗。

用户确认的裁剪：只显示套餐名 + 剩余额度，不做续费日期、升级/管理入口（因此不需要订阅接口）；zcode 的 "ZCode MCP" 额度卡与"获得N次重置额度"链接依赖 zcode 自家服务（zcode.z.ai 与其专属 token），不接入。

## 设计

### 额度接口与映射

- 端点：`GET https://bigmodel.cn/api/monitor/usage/quota/limit`，头 `authorization: <API Key 原文>`（monitor 接口要求直传完整凭据，无 Bearer 前缀），20s 超时、禁重定向、带供应商代理。
- 响应 envelope：`{code,msg,success,data:{level,limits:[{type,unit,number,usage,currentValue,remaining,percentage,nextResetTime(毫秒),usageDetails}]}}`；成功判定 `success !== false && (code == null || code == 0 || code == 200)`，失败时报 `MODEL_USAGE` 并透出 `msg`。
- 卡片匹配（type+unit，未知条目忽略，固定顺序）：5小时 = `TOKENS_LIMIT` unit=3；每周 = `TOKENS_LIMIT` unit=6；工具调用 = `TIME_LIMIT` unit=5。
- 百分比：接口 `percentage` 为**已用**口径，展示反转 `100 - clamp(percentage,0,100)`；缺失时回退 `remaining/usage`（usage 缺失时 `currentValue+remaining`）。
- 套餐名：`data.level` 归一化（`lite/pro/max` → GLM Coding Lite/Pro/Max，其余原样保留；空 → None）。
- `nextResetTime` 为毫秒时间戳；刚重置的 5 小时窗可能缺失该字段（UI 不显示时间）。

### 契约与命令

- `harness-contracts` 新增 `UsageCardKind`（fiveHour/weekly/tools）、`ProviderUsageCard`（kind/remainingPercent/resetAt）、`ProviderUsageSnapshot`（planName/cards）；`reset_at` 用 `#[specta(type = f64)]` 导出 number，绕开 specta 对 BigInt 类型（i64）的导出禁令（毫秒时间戳远小于 2^53，无精度损失）。
- `ModelService::fetch_usage(provider_id)`：仅内置 bigmodel（`builtin && compatibility==bigModel`）支持，其余报"该供应商不支持额度查询"；无密钥报 `MODEL_CREDENTIAL`；快照即查即返不落库——不挂 ProviderInfo 字段，不触发内置供应商覆盖校验。
- 桌面命令 `provider_usage(providerId) -> ProviderUsageSnapshot`，`frontend/bindings.ts` 随 export_bindings 重新生成，手写镜像在 `frontend/types.ts`。

### 设置面板布局

- 左栏导航（`nav[aria-label="供应商目录"]`）：分组标题"内置供应商"（deepseek、bigmodel）/"自定义供应商"；行 = 名称 + 徽标 + 启停开关；底部虚线按钮"添加自定义供应商"。
- 右栏详情视图（纵向）：
  1. 标题行：供应商名 + 徽标 +（仅智谱）刷新额度按钮；
  2. 计划卡（仅智谱）：套餐名 + 分隔线 + "剩余额度" + 三张额度卡（label + 剩余% + 重置时间 + 进度条；剩余 <20% 进度条转警示色）；重置时间 5小时卡当天显示 `HH:mm`、否则 `M月d日`，其余卡显示 `M月d日`；
  3. 连接设置表单（原供应商表单：名称/密钥/代理/复制/删除/默认模型/保存）；
  4. 模型列表：标题 + 添加模型按钮；行 = 名称 + 徽章（内置/档位）+ 编辑 + 删除（仅自定义）+ 启停开关。
- 拉取时机：选中智谱且有凭据引用时自动拉取 + 手动刷新 + 保存供应商后强刷；60s 内防重复拉取；错误显示在额度区内并提供重试按钮。
- 添加/编辑模型改为嵌套 Modal 弹窗（复用 Modal 焦点陷阱与 Esc 关闭）；内置模型查看改为只读弹窗；删除按钮移至模型行——只有已保存模型有行，天然满足"先保存后删除"约束。

## 测试与验证

- Rust：`map_usage_quota` 纯函数 4 项（真实样例映射与反转、乱序/未知条目忽略/percentage 缺失回退、错误 envelope、level 缺失）；全量门禁四件套（fmt/clippy/test/build，`--all-features`）。
- 前端 vitest：分组导航、智谱额度渲染（百分比/重置时间）、失败重试、非智谱不发请求、模型弹窗化后的编辑/删除/启停/测试流等 14 项；重置时间断言用本地时区构造的时间戳，避免 CI 时区差异。
- 实测接口（2026-11-07，真实凭据）：HTTP 200，`level=pro`，每周已用 86%（剩余 14%，重置 11月8日）、工具调用剩余 99%（重置 12月1日）、5小时窗刚重置（剩余 100%，无 nextResetTime）——与映射假设一致。
- 桌面 UI 抽查待用户（真实额度数值与 bigmodel 官网对照）。

## 边界与风险

- 接口要求密钥具备 Coding Plan 权益；无套餐时 limits 可能为空，UI 显示"当前套餐未返回额度明细"。
- 复制的 bigmodel 副本（非内置）不支持额度查询。
- 布局重排触达 ModelPanel 全部既有断言，已逐一适配；模型编辑弹窗嵌套在主 Modal 之上，焦点陷阱各自独立。
