import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { McpPanel } from "./McpPanel";
const { command } = vi.hoisted(() => ({ command: vi.fn() }));
vi.mock("./api", () => ({
  desktop: () => true,
  command,
  errorText: (e: unknown) =>
    e && typeof e === "object" && "message" in e ? String(e.message) : String(e),
}));
beforeEach(() => {
  command.mockReset();
  command.mockResolvedValue([]);
});
afterEach(cleanup);
describe("MCP 显式连接", () => {
  it("不自动连接，HTTP 提交绑定当前工作区并允许显式断开", async () => {
    let connected = false;
    command.mockImplementation(async (name) => {
      if (name === "mcp_connect") {
        connected = true;
        return;
      }
      if (name === "mcp_disconnect") {
        connected = false;
        return;
      }
      return connected ? [{ id: "remote", workspaceId: "w" }] : [];
    });
    render(<McpPanel workspaceId="w" sessionId="s" />);
    await screen.findByText("没有活动 MCP 连接。");
    expect(command).not.toHaveBeenCalledWith("mcp_connect", expect.anything());
    fireEvent.change(screen.getByLabelText("连接标识"), { target: { value: "remote" } });
    fireEvent.change(screen.getByLabelText("服务器地址"), {
      target: { value: "https://example.test/mcp" },
    });
    fireEvent.click(screen.getByRole("button", { name: "连接 MCP" }));
    await waitFor(() =>
      expect(command).toHaveBeenCalledWith("mcp_connect", {
        workspaceId: "w",
        sessionId: "s",
        config: { transport: "streamableHttp", id: "remote", url: "https://example.test/mcp" },
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "断开 remote" }));
    await waitFor(() =>
      expect(command).toHaveBeenCalledWith("mcp_disconnect", { id: "remote", workspaceId: "w" }),
    );
    await screen.findByText("MCP 连接已断开。");
  });
  it("stdio 显示副本占用说明，校验参数数组并传递真实配置", async () => {
    render(<McpPanel workspaceId="w" sessionId="s" />);
    expect(screen.getByText(/本地服务器/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("MCP 传输方式"), { target: { value: "stdio" } });
    fireEvent.change(screen.getByLabelText("连接标识"), { target: { value: "local" } });
    fireEvent.change(screen.getByLabelText("可执行程序"), {
      target: { value: "D:/tools/server.exe" },
    });
    fireEvent.change(screen.getByLabelText("启动参数（JSON 字符串数组）"), {
      target: { value: "[1]" },
    });
    fireEvent.click(screen.getByRole("button", { name: "连接 MCP" }));
    await screen.findByText("启动参数必须是 JSON 字符串数组。");
    expect(command).not.toHaveBeenCalledWith("mcp_connect", expect.anything());
    fireEvent.change(screen.getByLabelText("启动参数（JSON 字符串数组）"), {
      target: { value: '["--mode", "dev"]' },
    });
    fireEvent.click(screen.getByRole("button", { name: "连接 MCP" }));
    await waitFor(() =>
      expect(command).toHaveBeenCalledWith("mcp_connect", {
        workspaceId: "w",
        sessionId: "s",
        config: {
          transport: "stdio",
          id: "local",
          command: "D:/tools/server.exe",
          args: ["--mode", "dev"],
        },
      }),
    );
  });
  it("连接失败保持错误信息，不显示虚假成功", async () => {
    command.mockImplementation(async (name) => {
      if (name === "mcp_connect") throw new Error("握手失败");
      return [];
    });
    render(<McpPanel workspaceId="w" sessionId="s" />);
    fireEvent.change(screen.getByLabelText("连接标识"), { target: { value: "remote" } });
    fireEvent.change(screen.getByLabelText("服务器地址"), {
      target: { value: "https://example.test/mcp" },
    });
    fireEvent.click(screen.getByRole("button", { name: "连接 MCP" }));
    await screen.findByText("握手失败");
    expect(screen.queryByText("MCP 连接已建立。")).not.toBeInTheDocument();
  });
});
describe("MCP 已保存配置", () => {
  it("展示配置来源并可连接当前会话", async () => {
    command.mockImplementation(async (name) => {
      if (name === "mcp_config_list")
        return [
          {
            scope: { kind: "global" },
            trusted: true,
            config: {
              id: "saved",
              enabled: true,
              transport: { type: "streamableHttp", url: "https://example.test/mcp", headers: {} },
            },
          },
        ];
      return [];
    });
    render(<McpPanel workspaceId="w" sessionId="s" />);
    expect(await screen.findByText("saved（全局）")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "连接已保存的 saved" }));
    await waitFor(() =>
      expect(command).toHaveBeenCalledWith("mcp_config_connect", { sessionId: "s", id: "saved" }),
    );
  });
});
it("会话连接按所属会话断开", async () => {
  command.mockImplementation(async (name) => {
    if (name === "mcp_list") return [{ id: "saved", workspaceId: "w", sessionId: "s" }];
    return [];
  });
  render(<McpPanel workspaceId="w" sessionId="s" />);
  fireEvent.click(await screen.findByRole("button", { name: "断开 saved" }));
  await waitFor(() =>
    expect(command).toHaveBeenCalledWith("mcp_session_disconnect", { id: "saved", sessionId: "s" }),
  );
});
it("展示未信任的项目配置后才允许信任", async () => {
  const config = {
    id: "local",
    enabled: true,
    transport: { type: "stdio", command: "server.exe", args: ["--mode"] },
  };
  command.mockImplementation(async (name) => {
    if (name === "mcp_config_list")
      return [{ scope: { kind: "project", workspaceId: "w" }, config, trusted: false }];
    return [];
  });
  render(<McpPanel workspaceId="w" sessionId="s" />);
  expect(await screen.findByText("等待信任")).toBeInTheDocument();
  expect(screen.getByText(/server.exe/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "连接已保存的 local" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "信任 local" }));
  await waitFor(() =>
    expect(command).toHaveBeenCalledWith("mcp_config_trust", {
      scope: { kind: "project", workspaceId: "w" },
      config,
    }),
  );
});
it("显示已退出服务器的断开状态", async () => {
  command.mockImplementation(async (name) => {
    if (name === "mcp_list")
      return [{ id: "stopped", workspaceId: "w", sessionId: "s", status: "disconnected" }];
    return [];
  });
  render(<McpPanel workspaceId="w" sessionId="s" />);
  expect(await screen.findByText(/连接已断开/)).toBeInTheDocument();
});
it("展示当前会话的服务器启动失败状态", async () => {
  command.mockImplementation(async (name) => {
    if (name === "mcp_config_list")
      return [
        {
          scope: { kind: "global" },
          trusted: true,
          config: {
            id: "failed",
            enabled: true,
            transport: { type: "streamableHttp", url: "https://example.test/mcp" },
          },
        },
      ];
    if (name === "mcp_config_status")
      return [{ id: "failed", status: "failed", code: "MCP_ERROR" }];
    return [];
  });
  render(<McpPanel workspaceId="w" sessionId="s" />);
  expect(await screen.findByText("连接失败（MCP_ERROR）")).toBeInTheDocument();
});

it("无会话时可通过表单保存全局 HTTP 服务器", async () => {
  render(<McpPanel workspaceId="" sessionId="" />);
  fireEvent.click(screen.getByRole("button", { name: "添加服务器" }));
  fireEvent.change(screen.getByLabelText("服务器标识"), { target: { value: "manual" } });
  fireEvent.change(screen.getByLabelText("配置服务器地址"), {
    target: { value: "https://example.test/mcp" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存服务器配置" }));
  await waitFor(() =>
    expect(command).toHaveBeenCalledWith(
      "mcp_config_create",
      expect.objectContaining({
        scope: { kind: "global" },
        config: expect.objectContaining({
          id: "manual",
          transport: expect.objectContaining({
            type: "streamableHttp",
            url: "https://example.test/mcp",
          }),
        }),
      }),
    ),
  );
});

it("导入多个常见 JSON 配置后选择服务器进入表单", async () => {
  render(<McpPanel workspaceId="w" />);
  fireEvent.click(screen.getByRole("button", { name: "添加服务器" }));
  fireEvent.change(screen.getByLabelText("导入服务器 JSON"), {
    target: {
      value: JSON.stringify({
        mcpServers: { first: { command: "one.exe" }, second: { url: "https://example.test/mcp" } },
      }),
    },
  });
  fireEvent.click(screen.getByRole("button", { name: "解析 JSON" }));
  fireEvent.change(screen.getByLabelText("选择导入的服务器"), { target: { value: "second" } });
  fireEvent.click(screen.getByRole("button", { name: "导入所选服务器" }));
  expect(screen.getByLabelText("服务器标识")).toHaveValue("second");
  expect(screen.getByLabelText("配置服务器地址")).toHaveValue("https://example.test/mcp");
});

it("编辑本地项目配置时保留高级字段并锁定身份", async () => {
  const config = {
    id: "local",
    enabled: true,
    startupTimeoutSecs: 12,
    toolTimeoutSecs: 34,
    enabledTools: ["read"],
    disabledTools: ["write"],
    transport: {
      type: "stdio",
      command: "server.exe",
      args: ["--x"],
      cwd: "tools",
      env: { TOKEN: { source: "environment", name: "MCP_TOKEN" } },
    },
  };
  command.mockImplementation(async (name) =>
    name === "mcp_config_list"
      ? [{ scope: { kind: "project", workspaceId: "w" }, config, trusted: true }]
      : [],
  );
  render(<McpPanel workspaceId="w" />);
  fireEvent.click(await screen.findByRole("button", { name: "编辑 local" }));
  expect(screen.getByLabelText("服务器标识")).toBeDisabled();
  expect(screen.getByLabelText("配置范围")).toBeDisabled();
  fireEvent.change(screen.getByLabelText("配置可执行程序"), { target: { value: "new.exe" } });
  fireEvent.click(screen.getByRole("button", { name: "保存服务器配置" }));
  await waitFor(() =>
    expect(command).toHaveBeenCalledWith("mcp_config_save", {
      scope: { kind: "project", workspaceId: "w" },
      config: { ...config, transport: { ...config.transport, command: "new.exe" } },
    }),
  );
});

it("导入明文请求头与未知字段时拒绝并说明原因", async () => {
  render(<McpPanel workspaceId="w" />);
  fireEvent.click(screen.getByRole("button", { name: "添加服务器" }));
  const input = screen.getByLabelText("导入服务器 JSON");
  fireEvent.change(input, {
    target: {
      value: JSON.stringify({
        mcpServers: {
          remote: { url: "https://example.test", headers: { Authorization: "secret" } },
        },
      }),
    },
  });
  fireEvent.click(screen.getByRole("button", { name: "解析 JSON" }));
  expect(screen.getByRole("alert")).toHaveTextContent("不能使用明文值");
  fireEvent.change(input, {
    target: {
      value: JSON.stringify({
        mcpServers: { remote: { url: "https://example.test", typo: true } },
      }),
    },
  });
  fireEvent.click(screen.getByRole("button", { name: "解析 JSON" }));
  expect(screen.getByRole("alert")).toHaveTextContent("不支持的字段：typo");
});

it("导入传输字段类型错误时不会默默清空", async () => {
  render(<McpPanel workspaceId="w" />);
  fireEvent.click(screen.getByRole("button", { name: "添加服务器" }));
  fireEvent.change(screen.getByLabelText("导入服务器 JSON"), {
    target: { value: JSON.stringify({ mcpServers: { local: { command: 123 } } }) },
  });
  fireEvent.click(screen.getByRole("button", { name: "解析 JSON" }));
  expect(screen.getByRole("alert")).toHaveTextContent("可执行程序必须是字符串");
});

it("切换项目时关闭属于旧项目的编辑表单", async () => {
  const { rerender } = render(<McpPanel workspaceId="first" />);
  fireEvent.click(screen.getByRole("button", { name: "添加服务器" }));
  fireEvent.change(screen.getByLabelText("配置范围"), { target: { value: "project" } });
  rerender(<McpPanel workspaceId="second" />);
  expect(screen.queryByLabelText("服务器标识")).not.toBeInTheDocument();
});

it("可添加项目本地服务器并引用环境变量", async () => {
  render(<McpPanel workspaceId="w" />);
  fireEvent.click(screen.getByRole("button", { name: "添加服务器" }));
  fireEvent.change(screen.getByLabelText("配置范围"), { target: { value: "project" } });
  fireEvent.change(screen.getByLabelText("服务器标识"), { target: { value: "local" } });
  fireEvent.change(screen.getByLabelText("配置传输方式"), { target: { value: "stdio" } });
  fireEvent.change(screen.getByLabelText("配置可执行程序"), { target: { value: "server.exe" } });
  fireEvent.change(screen.getByLabelText("配置启动参数（JSON 字符串数组）"), {
    target: { value: '["--run"]' },
  });
  fireEvent.click(screen.getByRole("button", { name: "添加引用" }));
  fireEvent.change(screen.getByLabelText("名称"), { target: { value: "TOKEN" } });
  fireEvent.change(screen.getByLabelText("引用名称"), { target: { value: "MY_TOKEN" } });
  fireEvent.click(screen.getByRole("button", { name: "保存服务器配置" }));
  await waitFor(() =>
    expect(command).toHaveBeenCalledWith(
      "mcp_config_create",
      expect.objectContaining({
        scope: { kind: "project", workspaceId: "w" },
        config: expect.objectContaining({
          transport: expect.objectContaining({
            type: "stdio",
            command: "server.exe",
            args: ["--run"],
            env: { TOKEN: { source: "environment", name: "MY_TOKEN" } },
          }),
        }),
      }),
    ),
  );
});

it("导入本系统 JSON 后保留超时和过滤条件", async () => {
  render(<McpPanel workspaceId="w" />);
  fireEvent.click(screen.getByRole("button", { name: "添加服务器" }));
  fireEvent.change(screen.getByLabelText("导入服务器 JSON"), {
    target: {
      value: JSON.stringify({
        id: "remote",
        enabled: false,
        startupTimeoutSecs: 7,
        toolTimeoutSecs: 9,
        enabledTools: ["read"],
        disabledTools: ["write"],
        transport: {
          type: "streamableHttp",
          url: "https://example.test/mcp",
          headers: { Authorization: { source: "credential", name: "auth" } },
        },
      }),
    },
  });
  fireEvent.click(screen.getByRole("button", { name: "解析 JSON" }));
  fireEvent.click(screen.getByRole("button", { name: "导入所选服务器" }));
  expect(screen.getByLabelText("启动超时（秒）")).toHaveValue(7);
  expect(screen.getByLabelText("引用名称")).toHaveValue("auth");
  expect(screen.getByLabelText("允许工具（JSON 数组，null 表示全部）")).toHaveValue('["read"]');
  expect(screen.getByLabelText("启用服务器")).not.toBeChecked();
});

it("显示后端结构化保存错误", async () => {
  command.mockImplementation(async (name) => {
    if (name === "mcp_config_create")
      throw { code: "MCP_CONFIG", message: "该范围内的服务器标识已存在" };
    return [];
  });
  render(<McpPanel workspaceId="w" />);
  fireEvent.click(screen.getByRole("button", { name: "添加服务器" }));
  fireEvent.change(screen.getByLabelText("服务器标识"), { target: { value: "same" } });
  fireEvent.change(screen.getByLabelText("配置服务器地址"), {
    target: { value: "https://example.test" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存服务器配置" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("该范围内的服务器标识已存在");
});

it("导入并保存原型同名的合法引用名称", async () => {
  render(<McpPanel workspaceId="w" />);
  fireEvent.click(screen.getByRole("button", { name: "添加服务器" }));
  const raw =
    '{"id":"remote","transport":{"type":"streamableHttp","url":"https://example.test","headers":{"__proto__":{"source":"environment","name":"TOKEN"},"constructor":{"source":"environment","name":"TOKEN2"}}}}';
  fireEvent.change(screen.getByLabelText("导入服务器 JSON"), { target: { value: raw } });
  fireEvent.click(screen.getByRole("button", { name: "解析 JSON" }));
  fireEvent.click(screen.getByRole("button", { name: "导入所选服务器" }));
  fireEvent.click(screen.getByRole("button", { name: "保存服务器配置" }));
  await waitFor(() => {
    const call = command.mock.calls.find(([name]) => name === "mcp_config_create");
    expect(call?.[1].config.transport.headers).toEqual(
      JSON.parse(
        '{"__proto__":{"source":"environment","name":"TOKEN"},"constructor":{"source":"environment","name":"TOKEN2"}}',
      ),
    );
  });
});

it("保存成功后刷新失败时关闭新增表单并说明已经保存", async () => {
  let saved = false;
  command.mockImplementation(async (name) => {
    if (name === "mcp_config_create") {
      saved = true;
      return;
    }
    if (saved && name === "mcp_config_list") throw new Error("读取失败");
    return [];
  });
  render(<McpPanel workspaceId="w" />);
  fireEvent.click(screen.getByRole("button", { name: "添加服务器" }));
  fireEvent.change(screen.getByLabelText("服务器标识"), { target: { value: "saved" } });
  fireEvent.change(screen.getByLabelText("配置服务器地址"), {
    target: { value: "https://example.test" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存服务器配置" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("已保存，但刷新失败");
  expect(screen.queryByLabelText("服务器标识")).not.toBeInTheDocument();
});
