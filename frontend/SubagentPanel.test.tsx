import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { SubagentPanel } from "./SubagentPanel";
import type { SessionInfo } from "./types";

afterEach(cleanup);
const base = { workspaceId: "w", createdAt: "2026-11-10T08:00:00Z", parentId: "p" };
const child = (id: string, overrides: Partial<SessionInfo> = {}): SessionInfo => ({
  id,
  title: `子任务 ${id}`,
  updatedAt: "2026-11-10T09:00:00Z",
  ...base,
  ...overrides,
});

describe("子代理面板", () => {
  it("运行中的排前面并带停止按钮，其余按更新时间倒序", () => {
    const sessions = [
      child("old", { updatedAt: "2026-11-10T08:00:00Z" }),
      child("run", { updatedAt: "2026-11-10T10:00:00Z" }),
      child("new", { updatedAt: "2026-11-10T09:00:00Z" }),
    ];
    render(
      <SubagentPanel
        sessions={sessions}
        activeIds={new Set(["run"])}
        onOpen={() => {}}
        onCancel={() => {}}
        onClose={() => {}}
      />,
    );
    const rows = screen.getAllByRole("button", { name: /子任务 / });
    expect(rows.map((row) => row.textContent)).toEqual(["子任务 run", "子任务 new", "子任务 old"]);
    expect(screen.getByLabelText("子代理运行中")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "停止子代理" })).toBeInTheDocument();
  });
  it("点击行打开会话并收起面板，停止按钮只取消该会话", () => {
    const onOpen = vi.fn(),
      onCancel = vi.fn(),
      onClose = vi.fn();
    const sessions = [child("run"), child("idle")];
    render(
      <SubagentPanel
        sessions={sessions}
        activeIds={new Set(["run"])}
        onOpen={onOpen}
        onCancel={onCancel}
        onClose={onClose}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "停止子代理" }));
    expect(onCancel).toHaveBeenCalledWith("run");
    expect(onOpen).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "子任务 idle" }));
    expect(onOpen).toHaveBeenCalledWith(sessions[1]);
    expect(onClose).toHaveBeenCalled();
  });
  it("无子代理会话时显示空态", () => {
    render(
      <SubagentPanel
        sessions={[]}
        activeIds={new Set()}
        onOpen={() => {}}
        onCancel={() => {}}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText(/暂无子代理会话/)).toBeInTheDocument();
    expect(screen.getByText(/模型创建的子代理任务会显示在这里/)).toBeInTheDocument();
  });
  it("Escape 与点击外部收起面板，面板内点击不收起", () => {
    const onClose = vi.fn();
    const { container } = render(
      <div>
        <span data-testid="outside" />{" "}
        <SubagentPanel
          sessions={[child("c")]}
          activeIds={new Set()}
          onOpen={() => {}}
          onCancel={() => {}}
          onClose={onClose}
        />
      </div>,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    fireEvent.mouseDown(screen.getByRole("region", { name: "子代理" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    fireEvent.mouseDown(container.querySelector('[data-testid="outside"]')!);
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
