# 平台实现与验证

## 接口

- `WorkspaceManager::new(PathBuf)`、`create(&Path)`、`list()`、`get(id)`、`diff(id)`、`refresh(id)`。
- IPC应调用 `apply_reviewed(id, &[FileChange])`，独占锁内重新检查用户预览的before/after/source哈希，过期返回`STALE_DIFF`。兼容`apply(id, &[String])`只适合宿主内部明确即时确认的操作。
- `acquire(id) -> WorkspaceLease`持跨进程共享文件锁；同步独占锁遇活动lease返回`BUSY`。运行时和stdio生命周期应持lease。
- `read_file(root, relative) -> AppResult<String>`、`write_file(root, relative, text) -> AppResult<()>`、`list_files(root) -> AppResult<Vec<String>>`、`resolve_path(root, relative, must_exist) -> AppResult<PathBuf>`。返回PathBuf不会延续句柄保护；文件操作应直接使用read/write helper。
- `ShellService::run(root, command, CancellationToken)`返回stdout/stderr/exit_code；`spawn_stdio(root, program, &[String], cancel)`返回实现AsyncRead/AsyncWrite的`SandboxedStdio`。使用`tokio::io::split(child)`保留Job/lease，最后一个half释放时杀进程树。stdio只执行.exe绝对路径或PATH名字；.cmd/.bat需调用明确的.exe解释器。
- `set_secret(reference,value)`、`get_secret(reference)`、`delete_secret(reference)`使用Windows Credential Manager。非Windows返回UNSUPPORTED，不写明文文件。

## 安全实现

原项目与数据目录必须互不包含。数据布局为`workspaces/<uuid>/root`及副本外的宿主record/baseline、sync.lock、backups。普通文件按字节复制；Git采用`clone --no-local --no-hardlinks --no-checkout`和独立索引，再覆盖脏/未跟踪文件；无Git时普通复制。ignore只控制新文件发现，已有基线路径始终按真实磁盘状态读取，防新增ignore误报删除。路径检查拒绝绝对路径、穿越、ADS、保留设备名、符号链接、junction/reparse及文件硬链接。Windows操作期间持目录句柄禁止删除/替换，中途组件再次验证。

apply全部选择先预检冲突，写前复查来源字节；逐文件同目录临时文件+atomic replace，宿主保存原数据/selection和prepared/committed/rolled_back状态。失败回滚前验证当前文件仍等于本次写入结果，避免覆盖外部修改；有冲突则停止并保留备份。

Windows首次创建副本时才施ACL；原项目不施ACL。仅托管副本和每进程私有temp增加能力SID、Low mandatory label、Everyone FILE_DELETE_CHILD deny。非提升宿主作为owner先在这些自有目录给自己FullControl，以取得设置Low label所需WRITE_OWNER。运行时不反复向可能已被子进程修改的树传播ACL。token使用DISABLE_MAX_PRIVILEGE、WRITE_RESTRICTED、Low IL、logon/Everyone及工作区/私有temp SID，设置默认DACL支持受限孙进程管道。任何授权失败拒绝启动。子进程以挂起状态创建，加入KILL_ON_JOB_CLOSE Job后才resume；只继承stdio白名单句柄。取消及Drop杀整个Job。run输出分别限16MiB并继续排空；stdio stderr限64KiB并继续排空。

## 实测

2026-11-03，Windows 11、本机stable Rust，执行`cargo +stable test -p harness-platform`（为避其他crate构建锁，CARGO_TARGET_DIR指向本项目`.cache/platform-target`）：**13/13通过**。

- 10个workspace测试：预览/确认、原项目外改冲突、审批后副本/原项目hash变化拒绝、ignore基线回归、跨管理器lease、持久化/应用、穿越/ADS/嵌套数据拒绝、忽略缓存/硬链接拒绝、junction拒绝、Git独立历史/dirty/untracked及空Git仓库。
- 3个真实Windows进程测试：副本普通文件可写且副本外source普通文件写入被系统拒绝；取消后实际子进程PID已终止；受限stdio往返和split halves持有/释放lease。
- 调试中曾观察到Low label授权失败并修复宿主WRITE_OWNER前提；初版ignore基线回归及空Git导入均先观察失败后修复。未改参考项目、全局配置，未创建Node运行时依赖。

## 已知边界

- WRITE_RESTRICTED+Low IL提供真实系统写入边界，不隔离读取和网络；保留logon/Everyone以兼容Windows运行库，其ambient可写对象仍属于此机制的边界，不等同AppContainer或完整VM。
- 不提供跨多文件原子事务或断电后自动恢复；宿主备份与prepared日志用于人工恢复。同步前及每次写入前检查外部修改，不能将不受本产品锁约束的外部编辑器变成事务参与者。
- baseline/备份当前为宿主JSON字节快照，较大仓库会占较多内存/磁盘；refresh更新工作文件与基线，不重新拉取副本Git refs。
- 未在本机实测Windows Credential Manager读写往返；非Windows受限Shell故意不降级为无隔离进程。
