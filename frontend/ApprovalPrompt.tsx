import { ShieldCheck } from "lucide-react";
import type { ApprovalRequest } from "./types";

const printable = (value: unknown) =>
  typeof value === "string" ? value : JSON.stringify(value, null, 2);

/// 审批内容主体：主窗口用 Modal 包裹（由 Modal 提供 dialog 语义），侧聊抽屉用内联卡包裹。
export function ApprovalPrompt({
  approval,
  sessionTitle,
  onResolve,
  compact = false,
}: {
  approval: ApprovalRequest;
  sessionTitle: string;
  onResolve: (allow: boolean) => void;
  compact?: boolean;
}) {
  return (
    <div
      className={`approval-prompt${compact ? " compact" : ""}`}
      {...(compact ? { role: "dialog", "aria-label": "操作需要你的批准" } : {})}
    >
      <div className="approval-heading">
        <ShieldCheck size={compact ? 20 : 24} />
        <strong>{approval.tool}</strong>
      </div>
      <p>{approval.reason}</p>
      <pre className="code-block">{printable(approval.arguments)}</pre>
      <p className="muted">会话：{sessionTitle}</p>
      <div className="actions">
        <button type="button" onClick={() => onResolve(false)}>
          拒绝
        </button>
        <button type="button" className="primary" onClick={() => onResolve(true)}>
          批准此次操作
        </button>
      </div>
    </div>
  );
}
