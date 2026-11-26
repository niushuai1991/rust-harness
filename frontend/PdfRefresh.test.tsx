import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { SidePanel } from "./SidePanel";
import { useSidePanel } from "./sidePanelStore";
const mocks = vi.hoisted(() => ({ command: vi.fn(), getDocument: vi.fn() }));
vi.mock("./api", () => ({ command: mocks.command, errorText: (e: unknown) => String(e) }));
vi.mock("pdfjs-dist/legacy/build/pdf.mjs", () => ({
  GlobalWorkerOptions: {},
  getDocument: mocks.getDocument,
}));
vi.mock("pdfjs-dist/legacy/build/pdf.worker.min.mjs?url", () => ({ default: "worker.js" }));
afterEach(() => {
  cleanup();
  useSidePanel.getState().reset();
  vi.clearAllMocks();
});
it("再次打开同一 PDF 链接时重新读取文件", async () => {
  mocks.command.mockResolvedValue([37, 80, 68, 70]);
  mocks.getDocument.mockReturnValue({
    promise: Promise.resolve({
      numPages: 1,
      getPage: vi.fn().mockResolvedValue({
        getViewport: () => ({ width: 1, height: 1 }),
        render: () => ({ promise: Promise.resolve(), cancel: vi.fn() }),
      }),
    }),
    destroy: vi.fn(),
  });
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    {} as CanvasRenderingContext2D,
  );
  render(<SidePanel workspaceId="w" currentSessionId="s" />);
  act(() => useSidePanel.getState().openPdf("report.pdf"));
  await waitFor(() => expect(mocks.command).toHaveBeenCalledTimes(1));
  act(() => useSidePanel.getState().openPdf("report.pdf"));
  await waitFor(() => expect(mocks.command).toHaveBeenCalledTimes(2));
});
