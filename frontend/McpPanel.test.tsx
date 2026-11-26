import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { McpPanel } from "./McpPanel";
const { command } = vi.hoisted(() => ({ command: vi.fn() }));
vi.mock("./api", () => ({
  desktop: () => true,
  command,
  errorText: (e: unknown) => (e instanceof Error ? e.message : String(e)),
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
