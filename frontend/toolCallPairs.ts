import type { SessionEvent } from "./types";

// 通用工具时间线节点：配对成功携带 call+result；孤儿调用/结果缺一端独立存在。
export type ToolTimelineNode =
  | { kind: "tool"; key: string; call?: SessionEvent; result?: SessionEvent }
  | { kind: "event"; event: SessionEvent };

// shell 由 projectShellEvents 专属投影处理，此处排除避免双渲染。
const EXCLUSIVE_NAMES = new Set(["shell"]);

// 时间线展示投影：同一时间线分组内按 callId 关联工具调用与结果；
// 匹配唯一且结果晚于调用时合并为单节点；缺失、重复、乱序或缺少关联键（旧数据）时
// 保留独立节点兜底，保证任何 tool.call/tool.result 都有渲染归属。
export function projectToolEvents(events: SessionEvent[]): ToolTimelineNode[] {
  const calls = new Map<string, SessionEvent[]>(),
    results = new Map<string, SessionEvent[]>(),
    positions = new Map<SessionEvent, number>();
  const key = (event: SessionEvent, id: string) => JSON.stringify([event.sessionId, id]);
  for (const [index, event] of events.entries()) {
    positions.set(event, index);
    if (event.kind !== "tool.call" && event.kind !== "tool.result") continue;
    if (EXCLUSIVE_NAMES.has(String(event.data.name ?? ""))) continue;
    const raw = event.kind === "tool.call" ? event.data.id : event.data.callId;
    if (typeof raw !== "string" || !raw) continue;
    const target = event.kind === "tool.call" ? calls : results,
      k = key(event, raw);
    target.set(k, [...(target.get(k) ?? []), event]);
  }
  const paired = new Map<SessionEvent, SessionEvent>(),
    used = new Set<SessionEvent>();
  for (const [id, entries] of calls) {
    const found = results.get(id);
    if (
      entries.length === 1 &&
      found?.length === 1 &&
      positions.get(entries[0])! < positions.get(found[0])!
    ) {
      paired.set(entries[0], found[0]);
      used.add(found[0]);
    }
  }
  const nodes: ToolTimelineNode[] = [];
  for (const event of events) {
    if (used.has(event)) continue;
    if (event.kind === "tool.call" || event.kind === "tool.result") {
      if (!EXCLUSIVE_NAMES.has(String(event.data.name ?? ""))) {
        if (event.kind === "tool.call") {
          const result = paired.get(event);
          nodes.push(
            result
              ? { kind: "tool", key: event.id, call: event, result }
              : { kind: "tool", key: event.id, call: event },
          );
        } else nodes.push({ kind: "tool", key: event.id, result: event });
        continue;
      }
    }
    nodes.push({ kind: "event", event });
  }
  return nodes;
}
