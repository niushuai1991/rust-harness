import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ChevronRight, SquareTerminal } from "lucide-react";
import { parseShellResult, type ShellTimelineNode } from "./shellEvents";

type ShellNode = Extract<ShellTimelineNode, { kind: "shell" }>;

// Shell 命令内联行：收起为一行无边框摘要，点击展开浅底详情面板；结果未到达时显示扫光运行态。
export function ShellCommandCard({ node, artifact }: { node: ShellNode; artifact?: ReactNode }) {
  const args = node.call?.data.arguments;
  const command =
    args &&
    typeof args === "object" &&
    typeof (args as Record<string, unknown>).command === "string"
      ? (args as Record<string, string>).command
      : "";
  const status = node.result?.data.status;
  const running = !node.result;
  const label = !node.result
    ? "正在执行"
    : status === "denied"
      ? "已拒绝"
      : status === "unknown"
        ? "结果未知"
        : status === "cancelled"
          ? "已取消"
          : node.result.data.isError === true || status === "error"
            ? "执行失败"
            : "已完成";
  const failed = label === "执行失败" || label === "已拒绝" || label === "已取消";
  // 完成态不再重复状态词，只有异常终态才在行内提示。
  const statusVisible = !running && label !== "已完成";
  const parsed = node.result ? parseShellResult(node.result.data.content) : null;
  const [open, setOpen] = useState(false),
    [expanded, setExpanded] = useState(false),
    [overflow, setOverflow] = useState(command.split("\n").length > 3);
  const commandRef = useRef<HTMLPreElement>(null);
  useLayoutEffect(() => {
    const el = commandRef.current;
    if (!el || !open || expanded) return;
    const measure = () =>
      setOverflow(command.split("\n").length > 3 || el.scrollHeight > el.clientHeight + 1);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [command, expanded, open]);
  const toggle = () => setOpen((value) => !value);
  return (
    <div className="shell-row">
      <div
        className={`shell-trigger${running ? " is-running" : ""}`}
        role="button"
        tabIndex={0}
        aria-expanded={open}
        onClick={toggle}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            toggle();
          }
        }}
      >
        <span className="shell-icon" aria-hidden="true">
          <SquareTerminal size={15} />
        </span>
        <span className="shell-kind">{running ? "正在执行" : "已执行"}</span>
        <code className="shell-inline-cmd">{command || "命令不可用"}</code>
        {statusVisible && (
          <span className={`shell-status${failed ? " is-error" : ""}`}>{label}</span>
        )}
        <ChevronRight size={14} className="chev" aria-hidden="true" />
      </div>
      {open && (
        <div className="shell-detail">
          {command && (
            <div className="shell-cmd">
              <div className="shell-cmd-row">
                <span className="shell-prompt" aria-hidden="true">
                  $
                </span>
                <pre
                  ref={commandRef}
                  className={`shell-command-text${expanded ? " expanded" : ""}`}
                >
                  {command}
                </pre>
              </div>
              {(overflow || expanded) && (
                <button
                  type="button"
                  className="shell-cmd-toggle"
                  aria-expanded={expanded}
                  onClick={() => setExpanded((value) => !value)}
                >
                  {expanded ? "收起" : "展开命令"}
                </button>
              )}
            </div>
          )}
          {parsed?.kind === "output" ? (
            <div className="shell-out">
              {parsed.stdout && <pre>{parsed.stdout}</pre>}
              {parsed.stderr && <pre className="shell-stderr">{parsed.stderr}</pre>}
              {!parsed.stdout && !parsed.stderr && (
                <span className="shell-out-empty">没有输出。</span>
              )}
            </div>
          ) : parsed?.kind === "raw" ? (
            <div className="shell-out">
              <pre>{parsed.text || "没有输出。"}</pre>
            </div>
          ) : (
            <div className="shell-out">
              <span className="shell-out-empty">等待结果</span>
            </div>
          )}
          {parsed?.kind === "output" && (
            <div className={`shell-exit${parsed.exitCode !== 0 || failed ? " is-error" : ""}`}>
              退出码 {parsed.exitCode}
            </div>
          )}
          {artifact}
        </div>
      )}
    </div>
  );
}
