import { useState } from "react";
import { ChevronDown, MessageSquare, X } from "lucide-react";
import type { AskAnswer, AskRequest } from "./types";

/// ask_user 提问浮层：覆盖输入框（非阻塞、可折叠成摘要行）；自定义答复是选项列表的「其他」一项，「忽略」按 unattended 结算。
export function QuestionCard({
  question,
  sessionTitle,
  onSubmit,
}: {
  question: AskRequest;
  sessionTitle: string;
  onSubmit: (answer: AskAnswer) => void;
}) {
  const [choice, setChoice] = useState<number | null>(null),
    [freeText, setFreeText] = useState(""),
    [folded, setFolded] = useState(false);
  const hasOptions = question.options.length > 0,
    otherIndex = question.options.length;
  const decline = () => onSubmit({ text: "", source: "unattended" });
  const submit = () => {
    if (choice === otherIndex || !hasOptions) {
      if (freeText.trim()) onSubmit({ text: freeText.trim(), source: "text" });
      return;
    }
    if (choice !== null) onSubmit({ text: question.options[choice].label, source: "selected" });
  };
  const canSubmit = hasOptions
    ? choice !== null && (choice !== otherIndex || !!freeText.trim())
    : !!freeText.trim();
  return (
    <section
      className={`ask-popover${folded ? " folded" : ""}`}
      role="dialog"
      aria-modal="false"
      aria-label="Agent 需要你的输入"
    >
      <header className="ask-popover-head">
        <MessageSquare size={15} />
        <strong>{question.question}</strong>
        <button
          type="button"
          className="fold-btn"
          aria-label={folded ? "展开提问卡" : "折叠提问卡"}
          title={folded ? "展开提问卡" : "折叠提问卡"}
          onClick={() => setFolded((v) => !v)}
        >
          <ChevronDown size={15} />
        </button>
        <button
          type="button"
          aria-label="忽略提问"
          title="忽略（模型将自行决策继续）"
          onClick={decline}
        >
          <X size={15} />
        </button>
      </header>
      {!folded && (
        <>
          <p className="muted">会话：{sessionTitle}</p>
          {hasOptions && (
            <div role="radiogroup" aria-label="选项">
              {question.options.map((option, index) => (
                <label key={option.label} className="ask-option">
                  <input
                    type="radio"
                    name={`ask-${question.id}`}
                    aria-label={`选项 ${option.label}`}
                    checked={choice === index}
                    onChange={() => setChoice(index)}
                  />
                  <span>
                    {option.label}
                    {option.description ? `（${option.description}）` : ""}
                  </span>
                </label>
              ))}
              <label className="ask-option">
                <input
                  type="radio"
                  name={`ask-${question.id}`}
                  aria-label="选项 其他"
                  checked={choice === otherIndex}
                  onChange={() => setChoice(otherIndex)}
                />
                <span>其他（自定义答复）</span>
              </label>
            </div>
          )}
          {(!hasOptions || choice === otherIndex) && (
            <textarea
              className="ask-textarea"
              aria-label="自定义答复"
              placeholder="输入答复文本…"
              autoFocus
              value={freeText}
              onChange={(e) => setFreeText(e.target.value)}
            />
          )}
          <div className="actions">
            <button onClick={decline}>忽略</button>
            <button className="primary" disabled={!canSubmit} onClick={submit}>
              提交答复
            </button>
          </div>
        </>
      )}
    </section>
  );
}
