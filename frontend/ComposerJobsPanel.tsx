import { useEffect, useRef, useState } from "react";
import { Terminal } from "lucide-react";
import { command, errorText } from "./api";
import type { JobInfo } from "./types";
import { formatElapsed } from "./elapsed";

/// 输入框下方终端图标弹出的浮层卡片：仅列本会话运行中的后台任务
///（含前台到限转后台与模型主动 background_run 发起，同为 job.started 事件），
/// 每条显示命令、已运行时长（展开期间秒级刷新）与取消按钮。
/// 点开关按钮、点外部或按 Escape 收起（开关按钮的收起由 App 的 toggle 负责）。
export function ComposerJobsPanel({
  sessionId,
  onClose,
}: {
  sessionId: string;
  onClose: () => void;
}) {
  const [jobs, setJobs] = useState<JobInfo[]>([]);
  const [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [loaded, setLoaded] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const root = useRef<HTMLElement>(null);
  async function load() {
    try {
      setJobs(await command<JobInfo[]>("job_list", { sessionId }));
      setError("");
      setLoaded(true);
    } catch (e) {
      setError(errorText(e));
    }
  }
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), 2000);
    return () => clearInterval(timer);
  }, [sessionId]);
  // 时长显示秒级刷新；任务列表仍按 2 秒轮询，二者互不影响。
  useEffect(() => {
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(tick);
  }, []);
  useEffect(() => {
    function outside(e: MouseEvent) {
      const t = e.target as Node;
      if (root.current?.contains(t)) return;
      if (t instanceof Element && t.closest('[aria-controls="jobs-popover"]')) return;
      onClose();
    }
    function key(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", key);
    };
  }, [onClose]);
  async function cancel(id: string) {
    setBusy(true);
    setError("");
    try {
      await command("job_cancel", { sessionId, jobId: id });
      await load();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }
  const running = jobs.filter((j) => j.status === "running");
  return (
    <section
      ref={root}
      id="jobs-popover"
      className="composer-jobs-panel"
      role="region"
      aria-label="运行中后台任务"
    >
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
      {loaded && !error && !running.length && (
        <p className="muted empty-small">本会话暂无运行中的后台任务。</p>
      )}
      {running.map((j) => (
        <div key={j.id} className="composer-job">
          <Terminal size={14} aria-hidden="true" />
          <span className="composer-job-command" title={j.command}>
            {j.command}
          </span>
          <span className="composer-job-elapsed" aria-label="已运行时长">
            {formatElapsed(j.createdAt, now)}
          </span>
          {j.cancelRequested ? (
            <small className="muted">已请求取消</small>
          ) : (
            <button
              type="button"
              className="danger composer-job-cancel"
              disabled={busy}
              onClick={() => void cancel(j.id)}
            >
              取消
            </button>
          )}
        </div>
      ))}
    </section>
  );
}
