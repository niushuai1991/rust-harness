import { useEffect, useState, type FormEvent } from "react";
import { command, desktop, errorText } from "./api";
import { McpSavedServers } from "./McpSavedServers";

interface McpConnection {
  id: string;
  workspaceId: string;
  sessionId?: string;
  status?: "connected" | "disconnected";
}
type Transport = "streamableHttp" | "stdio";
export function McpPanel({
  workspaceId,
  sessionId = "",
}: {
  workspaceId: string;
  sessionId?: string;
}) {
  const [connections, setConnections] = useState<McpConnection[]>([]),
    [transport, setTransport] = useState<Transport>("streamableHttp");
  const [id, setId] = useState(""),
    [url, setUrl] = useState(""),
    [executable, setExecutable] = useState(""),
    [args, setArgs] = useState("[]");
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [status, setStatus] = useState("");
  async function load() {
    setConnections(await command<McpConnection[]>("mcp_list"));
  }
  useEffect(() => {
    let alive = true;
    void command<McpConnection[]>("mcp_list")
      .then((list) => {
        if (alive) setConnections(list);
      })
      .catch((e) => {
        if (alive) setError(errorText(e));
      });
    const timer = window.setInterval(() => {
      void command<McpConnection[]>("mcp_list")
        .then((list) => {
          if (alive) setConnections(list);
        })
        .catch((e) => {
          if (alive) setError(errorText(e));
        });
    }, 5000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);
  async function connect(event: FormEvent) {
    event.preventDefault();
    if (!sessionId) return;
    setBusy(true);
    setError("");
    setStatus("");
    try {
      let config;
      if (transport === "stdio") {
        const parsed: unknown = JSON.parse(args);
        if (!Array.isArray(parsed) || !parsed.every((value) => typeof value === "string"))
          throw new Error("启动参数必须是 JSON 字符串数组。");
        config = { transport, id: id.trim(), command: executable.trim(), args: parsed as string[] };
      } else {
        const endpoint = new URL(url);
        if (!["http:", "https:"].includes(endpoint.protocol))
          throw new Error("MCP 地址必须使用 HTTP 或 HTTPS。");
        config = { transport, id: id.trim(), url };
      }
      await command("mcp_connect", { workspaceId, sessionId, config });
      await load();
      setStatus("MCP 连接已建立。");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }
  async function disconnect(
    connectionId: string,
    ownerWorkspaceId: string,
    ownerSessionId?: string,
  ) {
    setBusy(true);
    setError("");
    setStatus("");
    try {
      if (ownerSessionId)
        await command("mcp_session_disconnect", { id: connectionId, sessionId: ownerSessionId });
      else await command("mcp_disconnect", { id: connectionId, workspaceId: ownerWorkspaceId });
      await load();
      setStatus("MCP 连接已断开。");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="mcp-panel" aria-label="MCP 连接">
      <h3>MCP 连接</h3>
      <McpSavedServers workspaceId={workspaceId} sessionId={sessionId} onChanged={load} />
      <p className="muted">启用的已保存服务器在会话开始时连接，连接状态仅保留在当前进程中。</p>
      <p className="mcp-warning">
        stdio 服务按当前会话的执行位置运行；直接模式使用当前用户权限。远端服务不受本地沙箱约束。
      </p>
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
      {status && (
        <p role="status" className="success">
          {status}
        </p>
      )}
      <form className="settings-form" onSubmit={connect}>
        <label>
          连接标识
          <input
            required
            value={id}
            onChange={(e) => setId(e.target.value)}
            placeholder="例如 local-tools"
            disabled={busy}
          />
        </label>
        <label>
          传输方式
          <select
            aria-label="MCP 传输方式"
            value={transport}
            onChange={(e) => setTransport(e.target.value as Transport)}
            disabled={busy}
          >
            <option value="streamableHttp">Streamable HTTP</option>
            <option value="stdio">本地 stdio 进程</option>
          </select>
        </label>
        {transport === "streamableHttp" ? (
          <label>
            服务器地址
            <input
              required
              type="url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://server.example/mcp"
              disabled={busy}
            />
          </label>
        ) : (
          <>
            <label>
              可执行程序
              <input
                required
                value={executable}
                onChange={(e) => setExecutable(e.target.value)}
                placeholder="本地可执行文件路径，不是 Shell 命令"
                disabled={busy}
              />
            </label>
            <label>
              启动参数（JSON 字符串数组）
              <input
                required
                value={args}
                onChange={(e) => setArgs(e.target.value)}
                placeholder={'["--flag", "value"]'}
                disabled={busy}
              />
            </label>
          </>
        )}
        {!sessionId && <p className="muted">请先创建会话，再连接 MCP。</p>}
        <button className="primary" disabled={busy || !sessionId || !desktop()}>
          {busy ? "处理中…" : "连接 MCP"}
        </button>
      </form>
      <div className="mcp-connections">
        {!connections.length && <p className="muted">没有活动 MCP 连接。</p>}
        {connections.map((connection) => (
          <article key={`${connection.sessionId ?? connection.workspaceId}:${connection.id}`}>
            <div>
              <strong>{connection.id}</strong>
              <small>
                {connection.workspaceId === workspaceId
                  ? "当前工作区"
                  : `工作区：${connection.workspaceId}`}
                {connection.status === "disconnected"
                  ? " · 连接已断开"
                  : connection.status === "connected"
                    ? " · 已连接"
                    : ""}
              </small>
            </div>
            <button
              disabled={busy}
              onClick={() =>
                disconnect(connection.id, connection.workspaceId, connection.sessionId)
              }
            >
              断开 {connection.id}
            </button>
          </article>
        ))}
      </div>
    </section>
  );
}
