import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import {
  Activity,
  ArrowRight,
  Bot,
  CalendarClock,
  Check,
  ChevronRight,
  CircleCheck,
  Copy,
  Download,
  FileCode,
  FilePlus,
  ListTodo,
  MessageSquare,
  NotepadText,
  Pencil,
  Send,
  Sparkles,
  Target,
  Terminal,
} from "lucide-react";
import { SessionMarkdown, LinkedText } from "./SessionLinks";
import { command, errorText } from "./api";
import { DiffView, diffStats, parseUnifiedDiff } from "./DiffView";
import { groupFileEdits, summarizeFileEdits, type FileEditGroup } from "./fileEdits";
import { projectShellEvents, type ShellTimelineNode } from "./shellEvents";
import { projectToolEvents, type ToolTimelineNode } from "./toolCallPairs";
import { ToolCard } from "./ToolCard";
import { ShellCommandCard } from "./ShellCommandCard";
import { PluginResultCards } from "./PluginResultCards";
import { JOB_STATUS, SCHEDULE_STATUS } from "./BackgroundTasksPanel";
import { useSidePanel } from "./sidePanelStore";
import type { SessionEvent, TimelineTurn, ArtifactMetadata } from "./types";

const basename = (path: string) => path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
const dirOf = (path: string) => {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length > 1 ? parts.slice(0, -1).join("/") : "";
};
const printable = (value: unknown) =>
  typeof value === "string" ? value : JSON.stringify(value, null, 2);
const TERMINAL_STATUS: { [status: string]: string } = {
  running: "运行中",
  exited: "已退出",
  terminated: "已终止",
  interrupted: "已中断",
};

export function Markdown({ content }: { content: string }) {
  return <SessionMarkdown content={content} />;
}
// 总结复制按钮按需读取完整回合正文；失败时允许重试。
function MessageCopyButton({
  text,
  turn,
}: {
  text: string;
  turn?: { sessionId: string; startSeq: string; endSeq: string };
}) {
  const [failure, setFailure] = useState(""),
    [busy, setBusy] = useState(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  async function copy() {
    if (busy) return;
    setBusy(true);
    setFailure("");
    try {
      const parts = turn ? await command<unknown[]>("session_turn_assistant_content", turn) : null;
      if (!mounted.current) return;
      const value = parts
        ? parts.map((part) => (part == null ? "" : (printable(part) ?? ""))).join("\n\n")
        : text;
      if (!navigator.clipboard) throw new Error("clipboard");
      await navigator.clipboard.writeText(value);
    } catch (error) {
      if (mounted.current) {
        const message = errorText(error);
        setFailure(message.includes("上限 8 MiB") ? message : "复制失败，请重试");
      }
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  return (
    <>
      <button
        type="button"
        className="message-copy"
        aria-label="复制消息"
        title="复制"
        disabled={busy}
        onClick={() => void copy()}
      >
        <Copy size={13} />
      </button>
      {failure && (
        <span role="alert" className="inline-error">
          {failure}
        </span>
      )}
    </>
  );
}
function durationText(start?: SessionEvent, end?: SessionEvent) {
  const startAt = Date.parse(start?.createdAt ?? ""),
    endAt = Date.parse(end?.createdAt ?? "");
  if (!Number.isFinite(startAt) || !Number.isFinite(endAt) || endAt < startAt) return "";
  const seconds = Math.floor((endAt - startAt) / 1000),
    hours = Math.floor(seconds / 3600),
    minutes = Math.floor((seconds % 3600) / 60),
    remaining = seconds % 60;
  return `${hours ? `${hours}小时` : ""}${hours || minutes ? `${minutes}分` : ""}${remaining}秒`;
}
// 合并组统计口径：shell 调用计命令数，write/edit 调用计编辑数（按 tool.call 计，孤儿结果不计）。
function processStats(events: SessionEvent[]) {
  let commands = 0,
    edits = 0;
  for (const event of events) {
    if (event.kind !== "tool.call") continue;
    const name = String(event.data.name);
    if (name === "shell") commands++;
    else if (name === "write" || name === "edit") edits++;
  }
  return { commands, edits };
}
// 过程合并组：运行中组头统计扫光，完成后统计收起；纯对话回合（无命令无编辑）回退「本轮已完成」文案。
function ProcessGroup({
  events,
  running,
  durationText: duration,
  hideCopy,
  pairingEvents,
  ...extras
}: {
  events: SessionEvent[];
  running: boolean;
  durationText?: string;
  hideCopy?: boolean;
  pairingEvents?: SessionEvent[];
} & EventCardExtras) {
  const { commands, edits } = processStats(events);
  const stat = [commands ? `${commands} 条命令` : "", edits ? `${edits} 次编辑` : ""]
    .filter(Boolean)
    .join(" · ");
  const label = running
    ? `正在执行${stat ? ` · ${stat}` : "…"}`
    : stat
      ? `已执行 · ${stat}${duration ? ` · 用时${duration}` : ""}`
      : `本轮已完成${duration ? `，用时${duration}` : ""}`;
  // 运行中组始终展开（open 属性随重渲染强制复位，保证进度可见）；完成后不设 open，默认收起且不干扰手动展开。
  return (
    <details className={`process-group${running ? " is-running" : ""}`} open={running || undefined}>
      <summary>
        <span className="process-head-icon" aria-hidden="true">
          <Activity size={15} />
        </span>
        <span className={`process-stat${running ? " is-running" : ""}`}>{label}</span>
        <ChevronRight size={14} className="chev" aria-hidden="true" />
      </summary>
      <div className="process-body">
        <EventList events={events} pairingEvents={pairingEvents} hideCopy={hideCopy} {...extras} />
      </div>
    </details>
  );
}
type EventCardExtras = {
  workspaceId?: string;
  onOpenSession?: (id: string) => void;
  onOpenSideChat?: (id: string) => void;
  activeIds?: Set<string>;
  askAnswers?: Record<string, { answer: string; source: string }>;
  steerStatuses?: Map<string, string>;
};
function SteeringTurn({
  events,
  turn,
  running,
  failed = false,
  steerStatuses,
  ...extras
}: {
  events: SessionEvent[];
  turn?: TimelineTurn;
  running: boolean;
  failed?: boolean;
} & EventCardExtras) {
  const queued = new Set(
    events.filter((e) => e.kind === "steer.queued").map((e) => String(e.data.id)),
  );
  const statuses = new Map(
    events
      .filter((e) => e.kind === "steer.applied" || e.kind === "steer.failed")
      .map((e) => [String(e.data.id), e.kind]),
  );
  const finalId =
    turn?.finalMessageId ?? events.filter((e) => e.kind === "assistant.message").at(-1)?.id;
  const copyText = events
    .filter((e) => e.kind === "assistant.message")
    .map((e) => (e.data.content == null ? "" : printable(e.data.content)))
    .join("\n\n");
  const result: ReactNode[] = [];
  let process: SessionEvent[] = [];
  const flush = () => {
    if (process.length) {
      result.push(
        failed ? (
          <EventList
            key={`process-${process[0].id}`}
            events={process}
            pairingEvents={events}
            hideCopy
            {...extras}
          />
        ) : (
          <ProcessGroup
            key={`process-${process[0].id}`}
            events={process}
            pairingEvents={events}
            running={running}
            hideCopy
            {...extras}
          />
        ),
      );
      process = [];
    }
  };
  for (const event of events) {
    if (
      event.kind === "turn.start" ||
      event.kind === "turn.end" ||
      event.kind === "steer.applied" ||
      event.kind === "steer.failed"
    )
      continue;
    if (
      event.kind === "user.message" &&
      event.data.steerId &&
      queued.has(String(event.data.steerId))
    )
      continue;
    if (event.kind === "steer.queued") {
      flush();
      const status =
        steerStatuses?.get(`${event.data.runId}:${event.data.id}`) ??
        statuses.get(String(event.data.id)) ??
        event.data.steerStatus;
      result.push(
        <Fragment key={event.id}>
          <EventCard event={{ ...event, kind: "user.message" }} {...extras} />
          <small className="muted">
            {status === "steer.applied"
              ? "已纳入上下文"
              : status === "steer.failed"
                ? "未处理，请重新发送"
                : "等待当前步骤完成"}
          </small>
        </Fragment>,
      );
      continue;
    }
    if (event.kind === "user.message") {
      flush();
      result.push(<EventCard key={event.id} event={event} {...extras} />);
      continue;
    }
    if (!running && !failed && event.id === finalId) {
      flush();
      result.push(
        <EventCard
          key={event.id}
          event={event}
          copyEnabled
          copyText={copyText}
          copyTurn={
            turn?.endSeq
              ? { sessionId: event.sessionId, startSeq: turn.startSeq, endSeq: turn.endSeq }
              : undefined
          }
          {...extras}
        />,
      );
      continue;
    }
    process.push(event);
  }
  flush();
  return <>{result}</>;
}
function CompletedTurn({
  events,
  turn,
  ...extras
}: { events: SessionEvent[]; turn?: TimelineTurn } & EventCardExtras) {
  if (events.some((event) => event.kind === "steer.queued"))
    return <SteeringTurn events={events} turn={turn} running={false} {...extras} />;
  const finalIndex = turn
    ? events.findIndex((e) => e.id === turn.finalMessageId)
    : events.map((e) => e.kind).lastIndexOf("assistant.message");
  const process = events.filter(
    (event, index) =>
      index !== finalIndex && !["turn.start", "turn.end", "user.message"].includes(event.kind),
  );
  const users = events.filter((event) => event.kind === "user.message"),
    final = finalIndex >= 0 ? events[finalIndex] : undefined;
  const start = events.find((event) => event.kind === "turn.start"),
    end = events.at(-1);
  const copyText = events
    .filter((event) => event.kind === "assistant.message")
    .map((event) => (event.data.content == null ? "" : printable(event.data.content)))
    .join("\n\n");
  return (
    <>
      {users.map((user) => (
        <EventCard key={user.id} event={user} {...extras} />
      ))}{" "}
      {start?.id.startsWith("synthetic-start-") && (
        <small className="muted">更早步骤尚未加载</small>
      )}{" "}
      {process.length > 0 && (
        <ProcessGroup
          events={process}
          running={false}
          durationText={durationText(start, end)}
          hideCopy
          {...extras}
        />
      )}{" "}
      {final && (
        <EventCard
          event={final}
          copyEnabled
          copyText={copyText}
          copyTurn={
            turn?.endSeq
              ? { sessionId: final.sessionId, startSeq: turn.startSeq, endSeq: turn.endSeq }
              : undefined
          }
          {...extras}
        />
      )}
    </>
  );
}
// 运行中（或未闭合）回合：用户消息常驻，其余过程收进合并组。
function OpenTurnSection({ events, ...extras }: { events: SessionEvent[] } & EventCardExtras) {
  if (events.some((event) => event.kind === "steer.queued"))
    return <SteeringTurn events={events} running {...extras} />;
  const users = events.filter((event) => event.kind === "user.message");
  const rest = events.filter((event) => !["turn.start", "user.message"].includes(event.kind));
  return (
    <Fragment>
      {users.map((user) => (
        <EventCard key={user.id} event={user} {...extras} />
      ))}
      {events[0]?.id.startsWith("synthetic-start-") && (
        <small className="muted">更早步骤尚未加载</small>
      )}
      {rest.length > 0 && <ProcessGroup events={rest} running hideCopy {...extras} />}
    </Fragment>
  );
}
export function ConversationEvents({
  events,
  turns,
  ...extras
}: { events: SessionEvent[]; turns: TimelineTurn[] } & EventCardExtras) {
  const steerStatuses = new Map(
    events
      .filter((e) => e.kind === "steer.applied" || e.kind === "steer.failed")
      .map((e) => [`${e.data.runId}:${e.data.id}`, e.kind]),
  );
  extras = { ...extras, steerStatuses };
  const durable = events.filter((event) => event.kind !== "ui.model_note");
  if (durable.length && turns.length) {
    const first = BigInt(durable[0].seq),
      last = BigInt(durable[durable.length - 1].seq),
      existing = new Set(durable.map((event) => event.seq));
    const synthetic: SessionEvent[] = [];
    for (const turn of turns) {
      const start = BigInt(turn.startSeq),
        end = turn.endSeq ? BigInt(turn.endSeq) : null;
      if (start < first && (end === null || end >= first))
        synthetic.push({
          id: `synthetic-start-${turn.startSeq}`,
          sessionId: events[0].sessionId,
          seq: turn.startSeq,
          kind: "turn.start",
          data: {},
          createdAt: "",
        });
      if (end !== null && end > last && start <= last && !existing.has(turn.endSeq!))
        synthetic.push({
          id: `synthetic-end-${turn.endSeq}`,
          sessionId: events[0].sessionId,
          seq: turn.endSeq!,
          kind: "turn.end",
          data: { status: turn.status },
          createdAt: "",
        });
    }
    events = [...events, ...synthetic].sort((a, b) =>
      BigInt(a.seq) < BigInt(b.seq) ? -1 : BigInt(a.seq) > BigInt(b.seq) ? 1 : 0,
    );
  }
  const cards: ReactNode[] = [];
  let start = -1,
    loose: SessionEvent[] = [];
  const flushLoose = () => {
    if (!loose.length) return;
    cards.push(<EventList key={`loose-${loose[0].id}`} events={loose} {...extras} />);
    loose = [];
  };
  for (let index = 0; index < events.length; index++) {
    const event = events[index];
    if (event.kind === "turn.start") {
      flushLoose();
      if (start >= 0)
        cards.push(
          <OpenTurnSection
            key={`open-${events[start].id}`}
            events={events.slice(start, index)}
            {...extras}
          />,
        );
      start = index;
      continue;
    }
    if (event.kind === "turn.end" && start >= 0) {
      flushLoose();
      const turn = events.slice(start, index + 1);
      let meta = turns.find((item) => item.startSeq === turn[0].seq);
      if (meta && !meta.endSeq)
        meta = {
          ...meta,
          endSeq: event.seq,
          status: String(event.data.status),
          finalMessageId: turn.filter((e) => e.kind === "assistant.message").at(-1)?.id ?? null,
        };
      cards.push(
        event.data.status === "completed" ? (
          <CompletedTurn key={event.id} events={turn} turn={meta} {...extras} />
        ) : turn.some((e) => e.kind === "steer.queued") ? (
          <SteeringTurn key={event.id} events={turn} running={false} failed {...extras} />
        ) : (
          <EventList key={event.id} events={turn} {...extras} />
        ),
      );
      start = -1;
      continue;
    }
    if (start < 0) loose.push(event);
  }
  flushLoose();
  if (start >= 0)
    cards.push(
      <OpenTurnSection key={`open-${events[start].id}`} events={events.slice(start)} {...extras} />,
    );
  return <>{cards}</>;
}
function EventList({
  events,
  hideCopy,
  pairingEvents,
  ...extras
}: {
  events: SessionEvent[];
  hideCopy?: boolean;
  pairingEvents?: SessionEvent[];
} & EventCardExtras) {
  const shells = new Map<SessionEvent, Extract<ShellTimelineNode, { kind: "shell" }>>(),
    pairedResults = new Set<SessionEvent>();
  for (const node of projectShellEvents(pairingEvents ?? events))
    if (node.kind === "shell") {
      const anchor = node.call ?? node.result;
      if (anchor) shells.set(anchor, node);
      if (node.call && node.result) pairedResults.add(node.result);
    }
  const tools = new Map<SessionEvent, Extract<ToolTimelineNode, { kind: "tool" }>>(),
    pairedToolResults = new Set<SessionEvent>();
  for (const node of projectToolEvents(pairingEvents ?? events))
    if (node.kind === "tool") {
      const anchor = node.call ?? node.result;
      if (anchor) tools.set(anchor, node);
      if (node.call && node.result) pairedToolResults.add(node.result);
    }
  return (
    <>
      {groupFileEdits(events).map((node) => {
        if (node.kind === "group") return <FileEditGroup key={node.key} group={node.group} />;
        const event = node.event;
        if (pairedResults.has(event) || pairedToolResults.has(event)) return null;
        const shell = shells.get(event);
        if (shell) {
          const result = shell.result,
            artifact = result?.data.artifact as ArtifactMetadata | undefined;
          return (
            <ShellCommandCard
              key={shell.key}
              node={shell}
              artifact={
                result && artifact ? (
                  <ArtifactDownload sessionId={result.sessionId} artifact={artifact} />
                ) : undefined
              }
            />
          );
        }
        const tool = tools.get(event);
        if (tool) {
          const result = tool.result,
            artifact = result?.data.artifact as ArtifactMetadata | undefined;
          return (
            <ToolCard
              key={tool.key}
              node={tool}
              workspaceId={extras.workspaceId}
              artifact={
                result && artifact ? (
                  <ArtifactDownload sessionId={result.sessionId} artifact={artifact} />
                ) : undefined
              }
            />
          );
        }
        return <EventCard key={event.id} event={event} hideCopy={hideCopy} {...extras} />;
      })}
    </>
  );
}
function FileEditGroup({ group }: { group: FileEditGroup }) {
  const summary = summarizeFileEdits(group);
  const status =
    summary.failed > 0
      ? summary.pending === 0 && summary.failed === summary.results.length
        ? "执行失败"
        : "部分失败"
      : summary.pending > 0
        ? "进行中"
        : "已完成";
  const head = (
    <>
      {summary.label === "写入" ? (
        <FilePlus size={14} aria-hidden="true" />
      ) : (
        <Pencil size={14} aria-hidden="true" />
      )}
      <span className="ft-label">
        {summary.label} ×{summary.operations}
      </span>
      <button
        type="button"
        className="file-chip"
        title={summary.path}
        onClick={(e) => {
          e.preventDefault();
          useSidePanel.getState().openPreview(summary.path);
        }}
      >
        <FileCode size={12} />
        {basename(summary.path)}
      </button>
      {!!dirOf(summary.path) && <span className="ft-dir">{dirOf(summary.path)}</span>}
    </>
  );
  return (
    <details className="tool-card file-tool file-tool-group">
      <summary>
        {head}
        {summary.hasStats && (
          <span className="tool-stats">
            <b className="st-add">+{summary.add}</b>
            <b className="st-del">-{summary.del}</b>
          </span>
        )}
        <span className={`ft-status${summary.failed ? " is-error" : ""}`}>{status}</span>
        <ChevronRight size={14} className="chev" aria-hidden="true" />
      </summary>
      <div className="file-edit-log">
        {summary.results.map((event, index) => {
          const diff =
            typeof event.data.diff === "string" && event.data.diff ? event.data.diff : null;
          const stats = diff ? diffStats(parseUnifiedDiff(diff)) : null;
          const opName = event.data.name === "write" ? "写入" : "编辑";
          return (
            <section key={event.id} className="file-edit-entry">
              <div className="file-edit-meta">
                <span>
                  第 {index + 1} 次{opName}
                </span>
                {stats && (
                  <span className="tool-stats">
                    <b className="st-add">+{stats.add}</b>
                    <b className="st-del">-{stats.del}</b>
                  </span>
                )}
                <span className={`ft-status${event.data.isError ? " is-error" : ""}`}>
                  {event.data.isError ? "执行失败" : "已完成"}
                </span>
              </div>
              {diff ? (
                <div className="tool-diff">
                  <DiffView diff={diff} />
                </div>
              ) : (
                <pre>{printable(event.data.content)}</pre>
              )}
            </section>
          );
        })}
        {summary.pending > 0 && (
          <p className="file-edit-pending">还有 {summary.pending} 次修改等待执行结果…</p>
        )}
      </div>
    </details>
  );
}
// 消息流 zcode 风格计划/待办卡。PLAN_STATE_KINDS 供 receiveEvent 判断计划抽屉联动。
export const PLAN_STATE_KINDS = ["todo.state", "plan.state", "goal.state"];
// 实时收到的待办事件 id：仅这些卡默认展开（历史回放收起），超上限整体清空防泄漏。
const liveTodoIds = new Set<string>();
export function noteLiveStateEvent(event: SessionEvent) {
  if (liveTodoIds.size >= 64) liveTodoIds.clear();
  liveTodoIds.add(event.id);
}
const todoDone = (item: Record<string, unknown>) =>
  item.done === true || item.completed === true || item.status === "completed";
function TodoStateCard({
  event,
  items,
}: {
  event: SessionEvent;
  items: Record<string, unknown>[];
}) {
  const done = items.filter(todoDone).length;
  const current = items.findIndex((item) => !todoDone(item));
  const active = String(items[current]?.text ?? items.at(-1)?.text ?? "当前任务");
  return (
    <details className="zc-card" open={liveTodoIds.has(event.id)}>
      <summary>
        <ListTodo size={15} />
        <strong>待办清单</strong>
        <span className="zc-primary">{active}</span>
        <span className="zc-meta">
          {done}/{items.length}
        </span>
        <span className="zc-chev">
          <ChevronRight size={13} />
        </span>
      </summary>
      <div className="zc-body">
        <div className="zc-body-inner">
          <div className="zc-rows">
            {items.map((item, index) => {
              const isDone = todoDone(item);
              const ico = isDone ? (
                <span className="zc-ico ok">
                  <CircleCheck size={14} />
                </span>
              ) : index === current ? (
                <span className="zc-ico cur">
                  <ArrowRight size={14} />
                </span>
              ) : (
                <span className="zc-ico pending">
                  <span className="zc-circle" />
                </span>
              );
              return (
                <div
                  key={index}
                  className={`zc-row ${isDone ? "done" : index === current ? "current" : "pending"}`}
                >
                  {ico}
                  <span>{String(item.text ?? "")}</span>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </details>
  );
}
// 计划/目标 Markdown 卡（对齐 zcode switch-mode 形态）：白卡预览 + 渐隐 + 悬浮「查看完整计划」；
// 整卡点击打开右侧「计划」抽屉（onOpenPlanDetail 语义映射到 openPlans）。
function PlanStateCard({ kind, content }: { kind: string; content: string }) {
  const isGoal = kind === "goal.state";
  const [copied, setCopied] = useState(false);
  const openPlans = () => useSidePanel.getState().openPlans();
  const copyPlan = () => {
    navigator.clipboard
      ?.writeText(content)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  };
  return (
    <article
      className="zc-plan"
      tabIndex={0}
      aria-label={isGoal ? "目标卡片，查看完整目标" : "计划卡片，查看完整计划"}
      onClick={openPlans}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          openPlans();
        }
      }}
    >
      <div className="zc-plan-head">
        {isGoal ? <Target size={16} /> : <NotepadText size={16} />}
        <strong className="zc-plan-title">{isGoal ? "目标" : "计划"}</strong>
        <button
          type="button"
          className={`zc-plan-copy${copied ? " ok" : ""}`}
          aria-label="复制计划"
          title={copied ? "已复制" : "复制计划"}
          onClick={(e) => {
            e.stopPropagation();
            copyPlan();
          }}
        >
          {copied ? <Check size={14} /> : <Copy size={14} />}
        </button>
      </div>
      <div className="zc-plan-body">
        <div className="zc-plan-preview">
          <div className="prose">
            <Markdown content={content} />
          </div>
        </div>
        <button
          type="button"
          className="zc-plan-full"
          onClick={(e) => {
            e.stopPropagation();
            openPlans();
          }}
        >
          查看完整计划 <ArrowRight size={15} />
        </button>
      </div>
    </article>
  );
}
export function EventCard({
  event,
  copyText,
  copyTurn,
  copyEnabled,
  hideCopy,
  ...extras
}: {
  event: SessionEvent;
  copyText?: string;
  copyTurn?: { sessionId: string; startSeq: string; endSeq: string };
  copyEnabled?: boolean;
  hideCopy?: boolean;
} & EventCardExtras) {
  const { kind, data } = event;
  if (kind === "ui.model_note")
    return (
      <div className="model-note" role="status">
        {printable(data.text)}
      </div>
    );
  if (kind === "subagent.created" || kind === "subagent.message_sent") {
    const childId = typeof data.sessionId === "string" ? data.sessionId : "";
    const sent = kind === "subagent.message_sent";
    const side = !sent && data.source === "side";
    // 侧聊登记卡在抽屉打开会话；普通子代理仍切换主会话（无抽屉入口时退回切换）。
    const openChild = side ? (extras.onOpenSideChat ?? extras.onOpenSession) : extras.onOpenSession;
    return (
      <article className="tool-card subagent-card">
        <div className="subagent-row">
          {sent ? <Send size={15} /> : <Bot size={15} />}
          <strong>{sent ? "已向子代理投递任务" : side ? "已创建侧聊" : "已创建子代理"}</strong>
          {childId && data.historyOnly !== true && openChild && (
            <button type="button" onClick={() => openChild(childId)}>
              查看会话
            </button>
          )}
          {childId && extras.activeIds?.has(childId) && (
            <i className="live-dot" aria-label="子代理运行中" />
          )}
        </div>
        {data.historyOnly === true && <p className="muted">历史派生记录，原父子关系属于源会话。</p>}
        {side && (
          <p className="muted">侧聊继承本会话历史仅供模型参考，点「查看会话」在右侧抽屉进入。</p>
        )}
        {sent && <pre>{printable(data.content)}</pre>}
      </article>
    );
  }
  if (kind === "todo.state") {
    const items = Array.isArray(data.items) ? (data.items as Record<string, unknown>[]) : [];
    return <TodoStateCard event={event} items={items} />;
  }
  if (kind === "plan.state" || kind === "goal.state")
    return <PlanStateCard kind={kind} content={printable(data.content)} />;
  if (kind === "user.message")
    return (
      <article className="message user" data-message-id={event.id}>
        <div className="message-body">
          <div className="user-bubble">
            {Array.isArray(data.quotes) &&
              data.quotes.map((quote, index) => (
                <blockquote key={index} className="sent-quote" title={String(quote)}>
                  <b>引用：</b>
                  {String(quote)}
                </blockquote>
              ))}
            <div className="prose">
              <LinkedText content={printable(data.content)} />
            </div>
            {Array.isArray(data.attachments) &&
              data.attachments.map((a, index) => (
                <ArtifactDownload
                  key={index}
                  sessionId={event.sessionId}
                  artifact={a as ArtifactMetadata}
                />
              ))}
          </div>
          {!!data.steerId && <small className="muted">已纳入上下文</small>}
        </div>
      </article>
    );
  if (kind === "assistant.message" || kind === "assistant.attempt") {
    const content = data.content == null ? "" : (printable(data.content) ?? "");
    const hasBody = content.trim().length > 0;
    // 中间消息（hideCopy）：空正文且无思考不渲染；有正文弱化为小字说明，不占完整消息形态。
    if (hideCopy && !hasBody && !data.reasoning) return null;
    const reasoning = !!data.reasoning && (
      <details className="reasoning">
        <summary>思考过程</summary>
        <pre>{printable(data.reasoning)}</pre>
      </details>
    );
    const body =
      hasBody &&
      (hideCopy ? (
        <div className="proc-note prose">
          <Markdown content={content} />
        </div>
      ) : (
        <div className="prose">
          <Markdown content={content} />
        </div>
      ));
    return (
      <article className="message assistant">
        <div className="message-body">
          {reasoning}
          {body}
          {!!data.error && <div className="inline-error">{printable(data.error)}</div>}
          {copyEnabled && hasBody && (
            <MessageCopyButton text={copyText ?? content} turn={copyTurn} />
          )}
        </div>
      </article>
    );
  }
  if (kind === "plugin.command.result")
    return (
      <article className="tool-card">
        <strong>{printable(data.name)}</strong>
        <PluginResultCards workspaceId={extras.workspaceId} content={printable(data.content)} />
      </article>
    );
  // ask_user 提问卡：回放展示问题与选项；答复状态由 ask.resolved 按 id 配对（askAnswers 由调用方传入）。
  if (kind === "ask.request") {
    const resolved = extras.askAnswers?.[String(data.id)];
    const options = Array.isArray(data.options) ? (data.options as Record<string, unknown>[]) : [];
    const unanswered =
      resolved && (resolved.source === "unattended" || resolved.source === "timeout");
    return (
      <article className="tool-card ask-card">
        <div className="subagent-row">
          <MessageSquare size={15} />
          <strong>Agent 提问</strong>
          {resolved && (
            <span className={unanswered ? "conflict" : ""}>
              {unanswered ? "未获得答复" : "已答复"}
            </span>
          )}
        </div>
        <div className="prose">
          <Markdown content={printable(data.question)} />
        </div>
        {options.length > 0 && (
          <ul className="ask-options">
            {options.map((option, index) => (
              <li key={index}>
                {printable(option.label)}
                {option.description ? `（${printable(option.description)}）` : ""}
              </li>
            ))}
          </ul>
        )}
        {resolved && !unanswered && (
          <p className="muted">
            答复：{resolved.answer}
            {resolved.source === "text" ? "（文本答复）" : ""}
          </p>
        )}
        {resolved && unanswered && (
          <p className="muted">用户不在场或等待超时，模型已按现有信息继续。</p>
        )}
      </article>
    );
  }
  if (kind === "job.started")
    return (
      <details className="tool-card">
        <summary>
          <Terminal size={15} />
          <strong>{printable(data.command)}</strong>
          <span>{JOB_STATUS[String(data.status)] ?? "运行中"}</span>
        </summary>
        <pre>{printable(data)}</pre>
      </details>
    );
  if (kind === "job.finished") {
    const status = String(data.status),
      output = data.output as { stdout?: unknown; stderr?: unknown; exit_code?: unknown } | null;
    return (
      <details className="tool-card">
        <summary>
          <Terminal size={15} />
          <strong>后台任务已结束</strong>
          <span className={status === "completed" ? "" : "conflict"}>
            {JOB_STATUS[status] ?? status}
          </span>
        </summary>
        {output ? (
          <pre>{`退出码 ${printable(output.exit_code)}\n【stdout】\n${printable(output.stdout ?? "")}\n【stderr】\n${printable(output.stderr ?? "")}`}</pre>
        ) : (
          data.error != null && <pre>错误：{printable(data.error)}</pre>
        )}
      </details>
    );
  }
  if (kind === "job.cancel_requested")
    return <div className="turn-status">已请求取消后台任务 {printable(data.id)}</div>;
  if (kind === "terminal.opened")
    return (
      <details className="tool-card">
        <summary>
          <Terminal size={15} />
          <strong>{`已打开交互式终端 ${printable(data.id)}`}</strong>
          <span>{TERMINAL_STATUS[String(data.status)] ?? "运行中"}</span>
        </summary>
        <pre>{`shell：${printable(data.shell)}`}</pre>
      </details>
    );
  if (kind === "terminal.closed") {
    const status = String(data.status);
    return (
      <details className="tool-card">
        <summary>
          <Terminal size={15} />
          <strong>{`交互式终端已结束 ${printable(data.id)}`}</strong>
          <span className={status === "exited" ? "" : "conflict"}>
            {TERMINAL_STATUS[status] ?? status}
          </span>
        </summary>
        <pre>
          {[
            data.exitCode != null ? `退出码 ${printable(data.exitCode)}` : "",
            typeof data.outputBlobHash === "string"
              ? `尾部输出 artifact：${data.outputBlobHash}${data.outputTruncated === true ? "（有截断）" : ""}`
              : "",
            data.error != null ? `错误：${printable(data.error)}` : "",
          ]
            .filter(Boolean)
            .join("\n")}
        </pre>
      </details>
    );
  }
  if (kind === "schedule.created")
    return (
      <details className="tool-card">
        <summary>
          <CalendarClock size={15} />
          <strong>{printable(data.task)}</strong>
          <span>{printable(data.at)}</span>
        </summary>
        <pre>{`执行时间：${printable(data.at)}\n模型：${printable(data.modelId)}`}</pre>
      </details>
    );
  if (kind === "schedule.deleted")
    return <div className="turn-status">已删除定时任务 {printable(data.id)}</div>;
  if (kind.startsWith("schedule.")) {
    const status = String(data.status ?? kind.slice("schedule.".length));
    return (
      <div className="turn-status">
        定时任务 {printable(data.id)}：{SCHEDULE_STATUS[status] ?? status}
      </div>
    );
  }
  if (kind === "skill.loaded")
    return (
      <details className="tool-card">
        <summary>
          <Sparkles size={15} />
          <strong>已加载技能 {printable(data.name)}</strong>
          <span>$提及注入</span>
        </summary>
        <pre>{printable(data.content)}</pre>
      </details>
    );
  if (kind === "compaction.summary")
    return (
      <details className="tool-card">
        <summary>上下文摘要</summary>
        <pre>{printable(data.content)}</pre>
      </details>
    );
  if (kind === "turn.end" && data.status && data.status !== "completed")
    return <div className="turn-status">本轮状态：{printable(data.status)}</div>;
  return null;
}
function ArtifactDownload({
  sessionId,
  artifact,
}: {
  sessionId: string;
  artifact: ArtifactMetadata;
}) {
  const [error, setError] = useState(""),
    [busy, setBusy] = useState(false);
  async function download() {
    setBusy(true);
    setError("");
    try {
      const path = await save({ defaultPath: artifact.name });
      if (path) await command("artifact_save", { sessionId, hash: artifact.hash, path });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div>
      <button type="button" disabled={busy} aria-label={`下载 ${artifact.name}`} onClick={download}>
        <Download size={14} />
        {artifact.name} ({artifact.size} 字节)
      </button>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
