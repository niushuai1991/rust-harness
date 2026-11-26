# MCP 服务器使用说明

在桌面端打开「插件与权限」中的 MCP 区域，可保存全局或当前项目的服务器配置。选择项目配置时，系统会先展示配置内容，确认信任后才会启动；修改配置会撤销原有信任。配置相同时，项目配置覆盖同名全局配置。无项目会话只使用全局配置。

启用并获信任的服务器在会话开始时启动，也可在已有会话中手动连接。首个模型请求会等待本次服务器启动和工具发现完成，响应缓慢的服务器可能使请求延迟至配置的启动超时；服务器失败后普通对话仍可继续。连接只属于该会话；子代理继承父会话的可用连接。计划模式不会启动或使用 MCP 工具；切换到计划模式前须断开该会话的 MCP 连接。连接失败不会自动重放工具调用，状态区显示错误代码。

## 配置示例

以下是本地服务器配置；启动参数按数组逐项填写，工作目录相对于会话执行根：

```json
{
  "id": "local-tools",
  "enabled": true,
  "startupTimeoutSecs": 30,
  "toolTimeoutSecs": 60,
  "transport": {
    "type": "stdio",
    "command": "python.exe",
    "args": ["server.py"],
    "cwd": "tools",
    "env": {
      "SERVICE_TOKEN": { "source": "environment", "name": "SERVICE_TOKEN" }
    }
  }
}
```

远程服务可通过请求头引用环境变量或系统凭据：

```json
{
  "id": "remote-tools",
  "enabled": true,
  "enabledTools": ["search", "read"],
  "disabledTools": [],
  "transport": {
    "type": "streamableHttp",
    "url": "https://example.invalid/mcp",
    "headers": {
      "Authorization": { "source": "environment", "name": "MCP_AUTHORIZATION" }
    }
  }
}
```

环境变量的值会原样作为请求头值；例如使用 Bearer 认证时，环境变量应包含认证方案和令牌。也可使用 `{"source":"credential","name":"凭据引用"}` 引用系统凭据。不要将令牌写入 URL、参数或普通配置。启动及调用超时可设置为 1 至 3600 秒；允许列表先过滤，再应用禁止列表。

本地进程在直接模式下使用当前用户权限；副本、worktree 和无项目会话使用各自的受限执行根。工作目录不得越过执行根或经过链接。远程服务器不受本地进程沙箱约束。

## 命令行

CLI 的 `harness call` 可通过 `mcp.config.list`、`mcp.config.save`、`mcp.config.remove` 和 `mcp.config.trust` 管理配置。参数采用 JSON：`scope` 是 `{"kind":"global"}` 或 `{"kind":"project","workspaceId":"项目 ID"}`，`config` 是上面的服务器配置。信任项目配置时需提供完整的当前配置，以防确认期间配置发生变化。

`harness run` 和 `harness chat` 在会话中使用生效配置。通过 JSON-RPC 长连接可调用 `mcp.config.connect`、`mcp.config.status` 和 `mcp.session.disconnect`；单次 `harness call` 退出后，其进程内连接也随之关闭。

## 当前限制

本阶段支持工具调用；适配器保留资源列表和读取接口，但尚未提供会话级资源工具；OAuth、资源模板、ACP 传入服务器配置、交互式服务端请求和 MCP Apps 尚未接入。图片和音频内容目前以说明替代，结构化结果单独保存在事件中，超大结果使用附件保存。Windows 本地服务器的命令须为可执行文件，目前不能直接使用 `.cmd` 或 `.ps1` 脚本作为命令。
