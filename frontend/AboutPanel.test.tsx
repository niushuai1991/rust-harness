import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { AboutPanel, formatBuildTime } from "./AboutPanel";

const mocks = vi.hoisted(() => ({ getVersion: vi.fn() }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: mocks.getVersion }));

afterEach(() => {
  cleanup();
  mocks.getVersion.mockReset();
});

describe("关于面板", () => {
  it("展示版本号、构建时间与提交信息", async () => {
    mocks.getVersion.mockResolvedValue("0.1.6");
    render(<AboutPanel />);
    expect(await screen.findByText("0.1.6")).toBeInTheDocument();
    expect(screen.getByText("桌面端版本号")).toBeInTheDocument();
    expect(screen.getByText("当前版本")).toBeInTheDocument();
    // 测试环境属于开发构建，构建时间显示为「开发构建」而非真实时间。
    expect(screen.getByText("开发构建")).toBeInTheDocument();
    expect(screen.getByText("构建时间（本地时区）")).toBeInTheDocument();
    expect(screen.getByText("构建对应的提交 ID")).toBeInTheDocument();
  });
  it("构建时间按本地时区格式化", () => {
    expect(formatBuildTime("2026-11-16T03:57:00Z")).toMatch(/^2026-11-1[56] \d{2}:57$/);
    expect(formatBuildTime("不是时间")).toBe("不是时间");
    expect(formatBuildTime("")).toBe("未知");
  });
});
