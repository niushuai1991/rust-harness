import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ToolCard } from "./ToolCard";
import type { SessionEvent } from "./types";

vi.mock("./PluginResultCards", () => ({
  PluginResultCards: (props: { content: string }) => (
    <div data-testid="plugin-cards">{props.content}</div>
  ),
}));

const event = (
  id: string,
  kind: string,
  data: Record<string, unknown>,
  createdAt = "",
): SessionEvent => ({ id, sessionId: "s", seq: id, kind, data, createdAt });
const call = (data: Record<string, unknown>, createdAt = "") =>
  event("c1", "tool.call", { id: "c1", ...data }, createdAt);
const result = (data: Record<string, unknown>, createdAt = "") =>
  event("r1", "tool.result", { callId: "c1", ...data }, createdAt);
const T = "2026-01-01T00:00:00Z";
const expand = () => fireEvent.click(document.querySelector(".tcard > summary")!);

afterEach(cleanup);

describe("通用工具调用卡片", () => {
  it("运行中：脉冲徽章 + is-running，展开显示等待结果", () => {
    render(
      <ToolCard node={{ key: "c1", call: call({ name: "read", arguments: { path: "a.ts" } }) }} />,
    );
    expect(screen.getByText("运行中", { selector: ".pill" })).toBeVisible();
    expect(document.querySelector(".tcard.is-running")).not.toBeNull();
    expect(screen.queryByText(/^[\d.]+s$/)).toBeNull();
    expand();
    expect(screen.getByText("等待结果")).toBeVisible();
  });
  it("完成：绿色徽章与耗时，展开显示参数与结果", () => {
    render(
      <ToolCard
        node={{
          key: "c1",
          call: call({ name: "read", arguments: { path: "a.ts", offset: 10, length: 5 } }, T),
          result: result(
            { name: "read", content: "正文" },
            new Date(Date.parse(T) + 1500).toISOString(),
          ),
        }}
      />,
    );
    expect(screen.getByText("已完成", { selector: ".pill" })).toBeVisible();
    expect(screen.getByText("1.5s", { selector: ".t-time" })).toBeVisible();
    expand();
    expect(screen.getByText(/"path": "a\.ts"/)).toBeVisible();
    expect(screen.getByText("正文")).toBeVisible();
  });
  it("失败：红色徽章、错误标题与红色输出", () => {
    render(
      <ToolCard
        node={{
          key: "c1",
          call: call({ name: "web_fetch", arguments: { url: "https://x" } }),
          result: result({ name: "web_fetch", isError: true, content: "超时" }),
        }}
      />,
    );
    expect(screen.getByText("执行失败", { selector: ".pill" })).toBeVisible();
    expand();
    expect(screen.getByText("错误", { selector: ".t-sec h5" })).toBeVisible();
    expect(screen.getByText("超时", { selector: "pre.is-err" })).toBeVisible();
  });
  it("status 终态映射：已拒绝/结果未知/已取消", () => {
    const base = { name: "read", content: "" };
    render(
      <ToolCard
        node={{
          key: "c1",
          call: call({ name: "read" }),
          result: result({ ...base, status: "denied" }),
        }}
      />,
    );
    expect(screen.getByText("已拒绝", { selector: ".pill" })).toBeVisible();
    cleanup();
    render(
      <ToolCard
        node={{
          key: "c1",
          call: call({ name: "read" }),
          result: result({ ...base, status: "unknown" }),
        }}
      />,
    );
    expect(screen.getByText("结果未知", { selector: ".pill" })).toBeVisible();
    cleanup();
    render(
      <ToolCard
        node={{
          key: "c1",
          call: call({ name: "read" }),
          result: result({ ...base, status: "cancelled" }),
        }}
      />,
    );
    expect(screen.getByText("已取消", { selector: ".pill" })).toBeVisible();
  });
  it("按工具类型给出中文标题与语义摘要", () => {
    render(
      <ToolCard
        node={{
          key: "c1",
          call: call({
            name: "read",
            arguments: { path: "frontend/App.tsx", offset: 1020, length: 30 },
          }),
          result: result({ name: "read", content: "" }),
        }}
      />,
    );
    expect(screen.getByText("读取", { selector: ".t-title" })).toBeVisible();
    expect(screen.getByText(/frontend\/App\.tsx · 1020–1050 行/)).toBeVisible();
  });
  it("write/edit 显示文件 chip、增删统计与 diff", () => {
    const diff = "--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n";
    render(
      <ToolCard
        node={{
          key: "c1",
          call: call({ name: "edit", arguments: { path: "a.ts" } }),
          result: result({ name: "edit", diff, content: "" }),
        }}
        artifact={<button>下载</button>}
      />,
    );
    expect(screen.getByText("编辑", { selector: ".t-title" })).toBeVisible();
    expect(screen.getByText("a.ts", { selector: ".file-chip" })).toBeVisible();
    expect(screen.getByText("+1", { selector: ".t-stats .add" })).toBeVisible();
    expect(screen.getByText("-1", { selector: ".t-stats .del" })).toBeVisible();
    expand();
    expect(screen.getByText("结果", { selector: ".t-sec h5" })).toBeVisible();
    expect(document.querySelector(".tool-diff")).not.toBeNull();
    expect(screen.getByRole("button", { name: "下载" })).toBeVisible();
  });
  it("插件工具结果渲染插件结果卡", () => {
    render(
      <ToolCard
        node={{
          key: "c1",
          call: call({ name: "plugin__demo.run", arguments: {} }),
          result: result({ name: "plugin__demo.run", content: "表格数据" }, T),
        }}
        workspaceId="w1"
      />,
    );
    expect(screen.getByTestId("plugin-cards")).toHaveTextContent("表格数据");
  });
  it("孤儿 write 结果渲染文件 chip 与预览入口", () => {
    render(
      <ToolCard
        node={{
          key: "r1",
          result: result({ name: "write", content: "已写入", path: "src/main.rs" }),
        }}
        artifact={<button>下载</button>}
      />,
    );
    expect(screen.getByRole("button", { name: "main.rs" })).toBeInTheDocument();
    expect(screen.getByText("写入", { selector: ".t-title" })).toBeVisible();
  });
  it("孤儿结果：无参数事件时标注调用缺失", () => {
    render(<ToolCard node={{ key: "r1", result: result({ name: "read", content: "x" }) }} />);
    expect(screen.getByText("已完成", { selector: ".pill" })).toBeVisible();
    expand();
    expect(screen.getByText("（调用事件缺失）", { selector: ".t-sec pre" })).toBeVisible();
  });
});
