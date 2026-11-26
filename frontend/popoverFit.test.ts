import { describe, expect, it } from "vitest";
import { fitPopoverLeft } from "./popoverFit";

describe("fitPopoverLeft", () => {
  it("宽窗口下保持默认位置（锚点左缘 -6px）", () => {
    expect(fitPopoverLeft(600, 280, 1400)).toBe(-6);
  });
  it("右缘越界时整体左移，保证视口右侧留出 8px", () => {
    // 700-6=694 起向右 280px 可到 974，超出 900 视口；夹紧到 900-8-280=612。
    expect(fitPopoverLeft(700, 280, 900)).toBe(-88);
  });
  it("视口窄于浮层时退化为左缘对齐视口边距", () => {
    expect(fitPopoverLeft(300, 280, 280)).toBe(8 - 300);
  });
  it("锚点贴近视口左缘时向右推回视口内", () => {
    expect(fitPopoverLeft(10, 280, 800)).toBe(-2);
  });
  it("未完成布局（宽度或视口为 0）时返回 null 沿用默认定位", () => {
    expect(fitPopoverLeft(700, 0, 900)).toBeNull();
    expect(fitPopoverLeft(700, 280, 0)).toBeNull();
  });
});
