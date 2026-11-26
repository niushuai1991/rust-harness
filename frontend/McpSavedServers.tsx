import { useEffect, useState } from "react";
import { command, desktop, errorText } from "./api";

type Scope = { kind: "global" } | { kind: "project"; workspaceId: string };
type ServerConfig = { id: string; enabled: boolean; transport: unknown; [key: string]: unknown };
type Entry = { scope: Scope; config: ServerConfig; trusted: boolean };
type Status = { id: string; status: string; code?: string | null };

export function McpSavedServers({
  workspaceId,
  sessionId,
  onChanged,
}: {
  workspaceId: string;
  sessionId: string;
  onChanged: () => Promise<void>;
}) {
  const [servers, setServers] = useState<Entry[]>([]);
  const [statuses, setStatuses] = useState<Status[]>([]);
  const [project, setProject] = useState(false);
  const [raw, setRaw] = useState(
    '{"id":"example","transport":{"type":"streamableHttp","url":"https://example.test/mcp"}}',
  );
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const scope: Scope = project ? { kind: "project", workspaceId } : { kind: "global" };

  async function load() {
    setServers(await command<Entry[]>("mcp_config_list", { workspaceId }));
  }
  useEffect(() => {
    let alive = true;
    void command<Entry[]>("mcp_config_list", { workspaceId })
      .then((entries) => {
        if (alive) setServers(entries);
      })
      .catch((e) => {
        if (alive) setError(errorText(e));
      });
    return () => {
      alive = false;
    };
  }, [workspaceId]);

  useEffect(() => {
    if (!sessionId) {
      setStatuses([]);
      return;
    }
    let alive = true;
    const refresh = () => {
      void command<Status[]>("mcp_config_status", { sessionId })
        .then((values) => {
          if (alive) setStatuses(values);
        })
        .catch((e) => {
          if (alive) setError(errorText(e));
        });
    };
    refresh();
    const timer = window.setInterval(refresh, 5000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [sessionId]);

  async function action(run: () => Promise<unknown>) {
    setBusy(true);
    setError("");
    try {
      await run();
      await load();
      if (sessionId) setStatuses(await command<Status[]>("mcp_config_status", { sessionId }));
      await onChanged();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="mcp-saved">
      <h4>已保存的服务器</h4>
      <p className="muted">
        直接执行模式下，本地服务器以当前用户权限运行；远程服务器不受本地沙箱约束。
      </p>
      {error && <p role="alert">{error}</p>}
      {servers.map((entry) => (
        <article key={JSON.stringify(entry.scope) + entry.config.id}>
          <strong>
            {entry.config.id}（{entry.scope.kind === "global" ? "全局" : "当前项目"}）
          </strong>
          <small>{entry.trusted ? (entry.config.enabled ? "已启用" : "已禁用") : "等待信任"}</small>
          {statuses
            .filter((status) => status.id === entry.config.id)
            .map((status) => (
              <small key={status.id}>
                {status.status === "failed"
                  ? `连接失败（${status.code || "未知错误"}）`
                  : status.status === "connecting"
                    ? "连接中"
                    : status.status === "connected"
                      ? "已连接"
                      : status.status === "disconnected"
                        ? "未连接"
                        : status.status === "untrusted"
                          ? "等待信任"
                          : "已禁用"}
              </small>
            ))}
          {!entry.trusted && (
            <>
              <pre>{JSON.stringify(entry.config, null, 2)}</pre>
              <button
                disabled={busy}
                onClick={() =>
                  void action(() =>
                    command("mcp_config_trust", { scope: entry.scope, config: entry.config }),
                  )
                }
              >
                信任 {entry.config.id}
              </button>
            </>
          )}
          <button
            disabled={busy || !sessionId || !entry.trusted || !entry.config.enabled}
            onClick={() =>
              void action(() => command("mcp_config_connect", { sessionId, id: entry.config.id }))
            }
          >
            连接已保存的 {entry.config.id}
          </button>
          <button
            disabled={busy}
            onClick={() => {
              setProject(entry.scope.kind === "project");
              setRaw(JSON.stringify(entry.config, null, 2));
            }}
          >
            编辑 {entry.config.id}
          </button>
          <button
            disabled={busy}
            onClick={() =>
              void action(() =>
                command("mcp_config_save", {
                  scope: entry.scope,
                  config: { ...entry.config, enabled: !entry.config.enabled },
                }),
              )
            }
          >
            {entry.config.enabled ? "禁用" : "启用"} {entry.config.id}
          </button>
          <button
            disabled={busy}
            onClick={() =>
              void action(() =>
                command("mcp_config_remove", { scope: entry.scope, id: entry.config.id }),
              )
            }
          >
            删除 {entry.config.id}
          </button>
        </article>
      ))}
      <label>
        配置范围
        <select
          aria-label="配置范围"
          value={project ? "project" : "global"}
          onChange={(e) => setProject(e.target.value === "project")}
          disabled={busy}
        >
          <option value="global">全局</option>
          <option value="project" disabled={!workspaceId}>
            当前项目
          </option>
        </select>
      </label>
      <label>
        服务器配置 JSON
        <textarea value={raw} onChange={(e) => setRaw(e.target.value)} disabled={busy} />
      </label>
      <p className="muted">请求头和环境变量使用环境变量名或凭据引用，不要输入明文密钥。</p>
      <button
        disabled={busy || !desktop()}
        onClick={() =>
          void action(() => command("mcp_config_save", { scope, config: JSON.parse(raw) }))
        }
      >
        保存服务器配置
      </button>
    </div>
  );
}
