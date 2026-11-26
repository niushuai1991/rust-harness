import { useEffect, useState } from "react";
import { command, errorText } from "./api";
import { McpEditor, type Scope, type ServerConfig } from "./McpEditor";

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
  const [editor, setEditor] = useState<Entry | "new" | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

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
              setEditor(entry);
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
      {!editor && (
        <button type="button" onClick={() => setEditor("new")}>
          添加服务器
        </button>
      )}
      {editor && (
        <McpEditor
          key={editor === "new" ? "new" : JSON.stringify(editor.scope) + editor.config.id}
          entry={editor === "new" ? undefined : editor}
          workspaceId={workspaceId}
          busy={busy}
          onCancel={() => setEditor(null)}
          onSave={async (scope, config, create) => {
            setBusy(true);
            setError("");
            try {
              await command(create ? "mcp_config_create" : "mcp_config_save", { scope, config });
              setEditor(null);
              try {
                await load();
                if (sessionId)
                  setStatuses(await command<Status[]>("mcp_config_status", { sessionId }));
                await onChanged();
              } catch (e) {
                setError(`已保存，但刷新失败：${errorText(e)}`);
              }
            } finally {
              setBusy(false);
            }
          }}
        />
      )}
    </div>
  );
}
