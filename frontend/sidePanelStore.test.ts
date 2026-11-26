import { describe, expect, it } from "vitest";
import { useSidePanel } from "./sidePanelStore";

describe("右侧面板状态", () => {
  it("openPlans 打开计划视图，close 保留视图而 reset 清理全部状态", () => {
    useSidePanel.getState().reset();
    useSidePanel.getState().openPreview("a.ts");
    useSidePanel.getState().openPlans();
    expect(useSidePanel.getState()).toMatchObject({
      open: true,
      view: "plans",
      previewPath: "a.ts",
    });
    useSidePanel.getState().close();
    expect(useSidePanel.getState()).toMatchObject({ open: false, view: "plans" });
    useSidePanel.getState().reset();
    expect(useSidePanel.getState()).toMatchObject({
      open: false,
      view: "changes",
      previewPath: null,
      webUrl: null,
    });
  });
  it("openPreview 打开面板、切到预览并记录文件路径", () => {
    useSidePanel.getState().reset();
    useSidePanel.getState().openPreview("src/app.ts");
    const state = useSidePanel.getState();
    expect(state.open).toBe(true);
    expect(state.view).toBe("preview");
    expect(state.previewPath).toBe("src/app.ts");
  });
  it("openChanges 打开面板并回到更改视图，保留预览路径", () => {
    useSidePanel.getState().reset();
    useSidePanel.getState().openPreview("a.ts");
    useSidePanel.getState().openChanges();
    const state = useSidePanel.getState();
    expect(state.open).toBe(true);
    expect(state.view).toBe("changes");
    expect(state.previewPath).toBe("a.ts");
  });
  it("toggle 切换开合，close 只隐藏不丢视图状态，reset 全部还原", () => {
    useSidePanel.getState().reset();
    useSidePanel.getState().openPreview("a.ts");
    useSidePanel.getState().toggle();
    expect(useSidePanel.getState().open).toBe(false);
    useSidePanel.getState().toggle();
    expect(useSidePanel.getState().open).toBe(true);
    useSidePanel.getState().close();
    expect(useSidePanel.getState().previewPath).toBe("a.ts");
    useSidePanel.getState().reset();
    const state = useSidePanel.getState();
    expect(state.open).toBe(false);
    expect(state.view).toBe("changes");
    expect(state.previewPath).toBeNull();
  });
  it("updateWebUrl 同步网页面板实际地址，reset 清理", () => {
    useSidePanel.getState().reset();
    useSidePanel.getState().openWeb("https://example.com");
    useSidePanel.getState().updateWebUrl("https://example.com/next");
    expect(useSidePanel.getState().webUrl).toBe("https://example.com/next");
    useSidePanel.getState().reset();
    expect(useSidePanel.getState().webUrl).toBeNull();
  });
  it("openChat 打开侧聊视图并记录会话与父会话，首条文字只消费一次", () => {
    useSidePanel.getState().reset();
    useSidePanel.getState().openChat("side-1", "a", "顺便问个问题");
    expect(useSidePanel.getState()).toMatchObject({
      open: true,
      view: "chat",
      chatSessionId: "side-1",
      chatParentSessionId: "a",
      chatInitial: { sessionId: "side-1", text: "顺便问个问题" },
    });
    expect(useSidePanel.getState().consumeChatInitial("side-1")).toBe("顺便问个问题");
    expect(useSidePanel.getState().chatInitial).toBeNull();
    expect(useSidePanel.getState().consumeChatInitial("side-1")).toBeNull();
  });
  it("切到其他视图或关闭面板保留侧聊会话，closeChat/reset 清理", () => {
    useSidePanel.getState().reset();
    useSidePanel.getState().openChat("side-1", "a");
    useSidePanel.getState().openPreview("a.ts");
    expect(useSidePanel.getState()).toMatchObject({ view: "preview", chatSessionId: "side-1" });
    useSidePanel.getState().close();
    expect(useSidePanel.getState()).toMatchObject({ open: false, chatSessionId: "side-1" });
    useSidePanel.getState().setView("chat");
    useSidePanel.getState().closeChat();
    expect(useSidePanel.getState()).toMatchObject({
      view: "changes",
      chatSessionId: null,
      chatParentSessionId: null,
    });
    useSidePanel.getState().reset();
  });
  it("侧聊草稿按会话保留", () => {
    useSidePanel.getState().reset();
    useSidePanel.getState().setChatDraft("side-1", "草稿一");
    useSidePanel.getState().setChatDraft("side-2", "草稿二");
    expect(useSidePanel.getState().chatDrafts).toEqual({ "side-1": "草稿一", "side-2": "草稿二" });
    useSidePanel.getState().reset();
    expect(useSidePanel.getState().chatDrafts).toEqual({});
  });
});
