# 0.1.1 安装权限修复

## 问题与原因

0.1.0 使用 Tauri NSIS `currentUser` 模式，实际EXE的PE清单为 `asInvoker`。用户选择受保护的 `D:\Program Files\Rust Harness` 后，安装器仍以普通权限写入，Windows不会自动将失败的写操作升级为UAC请求，因此出现无法写入exe的中止/重试/忽略对话框。

## 修复

- 将NSIS模式改为 `perMachine`，安装器在开始时请求管理员权限，而非等到复制文件失败。
- 默认位置采用系统Program Files，仍支持用户选择其他目录；安装注册信息使用HKLM。
- 不提高应用自身的执行级别，不修改工作区、用户数据或沙箱权限。
- 应用/安装包版本升为0.1.1，标准包和离线包共享同一权限配置。
- 增加读取实际PE资源的回归验证，并接入Windows构建脚本和CI；验证过程不执行安装器、不触发UAC、不写安装目录或注册表。

## 验证记录

旧0.1.0标准包：`verify-installer.py` 失败，实际级别为 `asInvoker`（红灯）。
新包构建及PE清单验证结果见 `.cache/nsis-uac-011-build.log` 与 `.cache/nsis-uac-011-offline.log`。
主应用仍应保持 `asInvoker`；不以安装器的管理员权限运行Agent工具。

实际结果：0.1.1标准包与离线包均为 `requireAdministrator`（绿灯），主程序为 `asInvoker`（绿灯）。两种安装包均已重新生成；标准包约12.55MiB，离线包约218.57MiB。构建过程中的前端生产构建通过，补丁格式与Python脚本语法检查通过。

未在用户的Program Files目录执行实际安装，也未代替用户接受UAC授权。系统策略或已经提升权限的父进程可能影响Windows是否显示UAC界面；PE请求级别为本次自动验收依据。
