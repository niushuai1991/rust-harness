import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import type { LiveEvent } from "./types";
export const desktop = () => isTauri();
export async function command<T>(name: string, body?: unknown): Promise<T> {
  if (!desktop()) throw new Error("需要桌面宿主：请使用 Rust Harness 桌面应用打开。");
  return invoke<T>(name, body === undefined ? {} : (body as Record<string, unknown>));
}
export function send(
  sessionId: string,
  text: string,
  modelId: string,
  onEvent: (event: LiveEvent) => void,
  attachments?: string[],
  effort?: string | null,
  quotes?: string[],
) {
  const channel = new Channel<LiveEvent>();
  channel.onmessage = onEvent;
  return command<void>("session_send", {
    sessionId,
    text,
    modelId,
    effort: effort ?? null,
    onEvent: channel,
    ...(attachments?.length ? { attachments } : {}),
    ...(quotes?.length ? { quotes } : {}),
  });
}
export function resume(
  sessionId: string,
  expectedEndSeq: string,
  modelId: string,
  onEvent: (event: LiveEvent) => void,
  effort?: string | null,
) {
  const channel = new Channel<LiveEvent>();
  channel.onmessage = onEvent;
  return command<void>("session_resume", {
    sessionId,
    expectedEndSeq,
    modelId,
    effort: effort ?? null,
    onEvent: channel,
  });
}
export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) return String(error.message);
  return "操作失败，请重试。";
}
