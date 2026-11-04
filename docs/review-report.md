# 独立代码审阅（2026-11-03）

范围：依据 [总体实施计划](specs/implementation-plan.md) 静态审阅桌面、持久化、运行时、工作副本和插件生命周期。当前无基准提交，且其他实现者正在修改文件；以下行号为审阅时位置。未修改实现或运行新的验证；所列复现为需要加入的验证场景。未将 P2/P3 预留能力视为已交付。结论：以下 Important 问题应在宣称 P1 验收前修复。未确认 Critical 问题。

## Important

1. **工具批次没有持久化完整结算边界，错误或恢复后会话不可继续。** [agent.rs](D:/code/rust-harness/crates/harness-runtime/src/agent.rs:112)、[storage lib.rs](D:/code/rust-harness/crates/harness-storage/src/lib.rs:85)。`assistant.message` 先保存所有 toolCalls，随后逐个检查重复 ID、写入 tool.call 并执行；重复 ID 会在前一个调用已执行后返回错误。若崩溃发生在 assistant.message 后或批次中途，recover 只检查已写入的 tool.call，没有为其余已宣布调用补结果；正常错误已写 turn.end 时更不会修补。project_messages 原样投影后向模型发送缺失结果/重复 ID 的历史。应在任何执行及 assistant.message 入库前验证完整批次，原子保存批次声明，结算或恢复所有已宣布调用并区分未执行/未知。验证：两个重复 ID 不产生副作用；两调用批次在第一条前/后崩溃，重启请求每个调用恰有一个结果。

2. **人工同步没有绑定实际审阅内容。** [App.tsx](D:/code/rust-harness/frontend/App.tsx:99)、[workspace.rs](D:/code/rust-harness/crates/harness-platform/src/workspace.rs:225)。UI 只提交路径，apply 获取独占锁后重新读取副本，没有比对预览的 afterHash。打开 v1 差异后，另一会话修改为 v2 并结束，再确认会将未审阅的 v2 写入原项目。提交包含路径及三方哈希的预览快照或宿主预览 token，在独占锁内校验；内容改变要求重新审阅。验证上述 v1→v2 场景应拒绝同步，原项目不变。

3. **修改忽略规则会将仍存在的基线文件视为删除。** [workspace.rs](D:/code/rust-harness/crates/harness-platform/src/workspace.rs:28)、[paths.rs](D:/code/rust-harness/crates/harness-platform/src/paths.rs:195)。同步 snapshot 复用遵守 .gitignore 的 list_files。基线存在 a.txt，在副本写入忽略 a.txt 的 .gitignore 后，after 不再含 a.txt，changes 会生成 deleted；默认选中并同步将删除原项目的真实文件。忽略规则应只影响新增文件发现，已纳入基线的路径应独立按实际存在性读取，并考虑 tracked 文件。验证改变忽略规则不会产生虚假的删除；实际删除基线文件仍正确报告。

4. **取消/超时把可能已发生副作用的工具统一记为普通失败。** [agent.rs](D:/code/rust-harness/crates/harness-runtime/src/agent.rs:128)。取消分支取消子 token 后丢弃 work.await 的结果，统一写 CANCELLED/TOOL_TIMEOUT；tool.result 没有 unknown 状态。批准 shell 先写文件再休眠，此时取消，磁盘已修改但持久化和 UI 只表示执行失败；继续会话可能重做非幂等操作。应记录是否开始执行、明确已知结果，无法确认副作用完成性时使用 unknown 并提示先检查，不能将取消解释为未执行。验证先追加一行再休眠的 shell 被取消后，恢复历史明确显示副作用未知且不自动重试。

5. **新建会话的异步响应可串入另一个工作区。** [App.tsx](D:/code/rust-harness/frontend/App.tsx:28)。session_list 有 generation 防护，newSession 没有。工作区 A 点击新建后，在响应前切换到 B，A 的响应随后将 A 会话插入 B 列表并设为当前，界面显示 B 但提交消息实际运行 A 副本。捕获 workspace/generation，只在当前工作区仍匹配时更新列表和选中项。验证使用延迟 session_create 的组件测试：切换 B 后 A 的响应不能改变 B 的当前会话。

6. **升级依赖插件会破坏已启用依赖关系。** [extensions lib.rs](D:/code/rust-harness/crates/harness-extensions/src/lib.rs:140)。install 检查升级版本满足已启用依赖方的版本约束，但新 Installed 总是 enabled=false。启用 B，安装并启用依赖 B 的 A，再将 B 升级到兼容版本，A 保持启用、B 被停用；这绕过 set_enabled 禁止停用被依赖插件的约束。升级应保持旧启用状态并原子校验新图，或拒绝升级直至依赖方停用。验证升级成功后所有已启用插件的依赖仍存在且启用、版本满足要求。

7. **导入没有跨会话和事件的事务，会留下部分导入。** [storage lib.rs](D:/code/rust-harness/crates/harness-storage/src/lib.rs:124)。预解析虽避免 JSON 语法错误产生残留，但 create_session 和每次 append 都分别提交；第 N 条 SQL/FTS 写入失败、future 被取消或进程中断，留下可见的半截会话。此项针对已有 Store::import API，不声称桌面已交付导入。用单事务写会话、事件及搜索索引，成功后才发布。验证在第 N 条事件插入处注入失败后，session/events/FTS 均无新增；成功输入完整提交。

## 尚未验证的边界

Windows 沙箱代码仍在实现者修订和真机测试，本报告不据静态阅读宣称隔离通过。同步期间原项目外部编辑的 compare/write 间隙、祖先目录 reparse 竞态应保留实际竞争测试。当前未将这些未复现疑点提升为确定缺陷。
