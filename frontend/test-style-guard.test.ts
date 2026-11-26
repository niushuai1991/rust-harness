import "./style.css";
import { expect, it } from "vitest";
import { assertNoAppGlobalStyle } from "./test-style-guard";

it("拦截实际加载的全局样式并给出替代测试位置", () => {
  expect(() => assertNoAppGlobalStyle()).toThrow(/App\.style\.test\.tsx/);
});

it("允许测试使用局部样式", () => {
  const globalStyle = document.querySelector('style[data-vite-dev-id$="/frontend/style.css"]')!;
  const localStyle = document.createElement("style");
  localStyle.setAttribute("data-vite-dev-id", "/frontend/component.css");
  localStyle.textContent = ".test-component { display: block }";
  globalStyle.remove();
  document.head.append(localStyle);
  try {
    expect(() => assertNoAppGlobalStyle()).not.toThrow();
  } finally {
    localStyle.remove();
    document.head.append(globalStyle);
  }
});
