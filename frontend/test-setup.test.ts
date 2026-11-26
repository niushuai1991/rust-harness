import { expect, it } from "vitest";

it.each(["第一项", "第二项"])("浏览器存储在每个测试开始时独立：%s", () => {
  expect(localStorage.getItem("fixture-isolation")).toBeNull();
  expect(sessionStorage.getItem("fixture-isolation")).toBeNull();
  localStorage.setItem("fixture-isolation", "local");
  sessionStorage.setItem("fixture-isolation", "session");
  expect(localStorage.getItem("fixture-isolation")).toBe("local");
  expect(sessionStorage.getItem("fixture-isolation")).toBe("session");
});
