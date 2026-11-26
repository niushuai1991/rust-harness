import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { SessionMarkdown } from "./SessionLinks";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
describe("Markdown 代码块复制", () => {
  it("分别复制完整原始 Markdown，保留语言、空白与换行", async () => {
    const copy = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: copy },
    });
    const first = "```rust title=test\r\nfn main() {  \r\n}\r\n```";
    const second = "```\n  text\n```";
    render(<SessionMarkdown content={`前文\r\n\r\n${first}\r\n\r\n${second}`} />);
    const buttons = screen.getAllByRole("button", { name: "复制代码块" });
    expect(buttons).toHaveLength(2);
    expect(buttons[0].textContent).toBe("");
    fireEvent.click(buttons[0]);
    await waitFor(() => expect(copy).toHaveBeenLastCalledWith(first));
    fireEvent.click(buttons[1]);
    await waitFor(() => expect(copy).toHaveBeenLastCalledWith(second));
  });
  it("行内代码、缩进代码和未闭合围栏没有复制按钮", () => {
    render(<SessionMarkdown content={"`inline`\n\n    indented\n\n```rust\nunfinished"} />);
    expect(screen.queryByRole("button", { name: "复制代码块" })).not.toBeInTheDocument();
  });
  it("保留仅使用回车的原始换行", async () => {
    const copy = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: copy },
    });
    const raw = "```js\rconst x = 1;\r```";
    render(<SessionMarkdown content={raw} />);
    fireEvent.click(screen.getByRole("button", { name: "复制代码块" }));
    await waitFor(() => expect(copy).toHaveBeenCalledWith(raw));
  });
  it("引用块和列表中的围栏保留源文本", async () => {
    const copy = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: copy },
    });
    render(
      <SessionMarkdown
        content={" > ```js\n > const x = 1;\n > ```\n\n- 内容\n\n  ```sh\n  echo hi\n  ```"}
      />,
    );
    const buttons = screen.getAllByRole("button", { name: "复制代码块" });
    expect(buttons).toHaveLength(2);
    fireEvent.click(buttons[0]);
    await waitFor(() => expect(copy).toHaveBeenLastCalledWith("```js\n > const x = 1;\n > ```"));
    fireEvent.click(buttons[1]);
    await waitFor(() => expect(copy).toHaveBeenLastCalledWith("```sh\n  echo hi\n  ```"));
  });
  it("复制失败时给出反馈并可重试", async () => {
    const copy = vi
      .fn()
      .mockRejectedValueOnce(new Error("denied"))
      .mockResolvedValueOnce(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: copy },
    });
    render(<SessionMarkdown content={"```js\nx\n```"} />);
    fireEvent.click(screen.getByRole("button", { name: "复制代码块" }));
    expect(await screen.findByRole("status")).toHaveTextContent("复制失败");
    fireEvent.click(screen.getByRole("button", { name: "复制代码块" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("已复制"));
  });
});
