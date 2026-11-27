import { useEffect, useState } from "react";
import { command, errorText } from "./api";

type Hook = {
  digest: string;
  source: string;
  event: string;
  matcher: string | null;
  command: string;
  args: string[] | null;
  scriptPath: string | null;
  scriptBound: boolean;
  trusted: boolean;
};

export function HookTrustPanel({
  sessionId,
  onComplete,
}: {
  sessionId: string;
  onComplete?: () => void;
}) {
  const [entries, setEntries] = useState<Hook[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let active = true;
    setEntries([]);
    setError("");
    if (sessionId)
      void command<Hook[]>("hooks_project_review", { sessionId })
        .then((items) => {
          if (active) setEntries(items);
        })
        .catch((e) => {
          if (active) setError(errorText(e));
        });
    return () => {
      active = false;
    };
  }, [sessionId]);

  async function change(entry: Hook) {
    setBusy(true);
    setError("");
    try {
      await command(entry.trusted ? "hooks_project_revoke" : "hooks_project_trust", {
        sessionId,
        digest: entry.digest,
      });
      const updated = await command<Hook[]>("hooks_project_review", { sessionId });
      setEntries(updated);
      if (!entry.trusted && updated.every((item) => item.trusted)) onComplete?.();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  if (!sessionId) return <p>请选择项目会话后审核其 Hook。</p>;
  return (
    <section>
      <p>
        直接执行模式下命令以当前用户权限运行。入口脚本加载的其他文件和网络内容不包含在信任摘要中，请审核后再授权。
      </p>
      {error && <p role="alert">{error}</p>}
      {!error && entries.length === 0 && <p>当前会话没有项目 Hook。</p>}
      {entries.map((entry) => (
        <article key={entry.digest}>
          <h3>
            {entry.event} · {entry.trusted ? "已信任" : "等待审核"}
          </h3>
          <p>来源：{entry.source}</p>
          <p>匹配：{entry.matcher || "全部"}</p>
          <p>
            命令：<code>{entry.command}</code>
          </p>
          {entry.args && (
            <p>
              参数：<code>{JSON.stringify(entry.args)}</code>
            </p>
          )}
          <p>入口脚本：{entry.scriptPath || "无法识别，未纳入摘要"}</p>
          <button disabled={busy} onClick={() => void change(entry)}>
            {entry.trusted ? "撤销信任" : "信任此 Hook"}
          </button>
        </article>
      ))}
    </section>
  );
}
