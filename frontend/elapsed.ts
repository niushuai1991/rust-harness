/// 运行中后台任务的已执行时长：mm:ss 补零，满一小时进位为 h:mm:ss。
/// now 由调用方传入（毫秒时间戳），便于测试与秒级 ticker 复用。
export function formatElapsed(startIso: string, now: number): string {
  const start = Date.parse(startIso);
  if (!Number.isFinite(start)) return "--:--";
  const total = Math.max(0, Math.floor((now - start) / 1000));
  const h = Math.floor(total / 3600),
    m = Math.floor((total % 3600) / 60),
    s = total % 60;
  const mm = String(m).padStart(2, "0"),
    ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}
