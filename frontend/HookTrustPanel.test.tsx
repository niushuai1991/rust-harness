import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { HookTrustPanel } from "./HookTrustPanel";

const { command } = vi.hoisted(() => ({ command: vi.fn() }));
vi.mock("./api", () => ({ command, errorText: String }));
afterEach(cleanup);

it("展示具体命令并支持逐条信任与撤销", async () => {
  let trusted = false;
  command.mockImplementation(async (name) => {
    if (name === "hooks_project_trust") trusted = true;
    if (name === "hooks_project_revoke") trusted = false;
    if (name === "hooks_project_review")
      return [
        {
          digest: "abc",
          source: ".rh/.settings.json",
          event: "PreToolUse",
          matcher: "write",
          command: "powershell.exe",
          args: ["-File", "check.ps1"],
          scriptPath: "check.ps1",
          scriptBound: true,
          trusted,
        },
      ];
  });
  render(<HookTrustPanel sessionId="session" />);
  await screen.findByText("powershell.exe");
  fireEvent.click(screen.getByRole("button", { name: "信任此 Hook" }));
  await waitFor(() =>
    expect(command).toHaveBeenCalledWith("hooks_project_trust", {
      sessionId: "session",
      digest: "abc",
    }),
  );
  fireEvent.click(await screen.findByRole("button", { name: "撤销信任" }));
  await waitFor(() =>
    expect(command).toHaveBeenCalledWith("hooks_project_revoke", {
      sessionId: "session",
      digest: "abc",
    }),
  );
});

it("所有 Hook 都获信任后通知等待中的任务", async () => {
  const completed = vi.fn();
  let trusted = false;
  command.mockImplementation(async (name) => {
    if (name === "hooks_project_trust") trusted = true;
    if (name === "hooks_project_review")
      return [
        {
          digest: "abc",
          source: "settings",
          event: "Stop",
          matcher: null,
          command: "echo",
          args: null,
          scriptPath: null,
          scriptBound: false,
          trusted,
        },
      ];
  });
  render(<HookTrustPanel sessionId="session" onComplete={completed} />);
  fireEvent.click(await screen.findByRole("button", { name: "信任此 Hook" }));
  await waitFor(() => expect(completed).toHaveBeenCalledOnce());
});
