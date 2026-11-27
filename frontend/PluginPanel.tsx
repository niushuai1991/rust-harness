import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Unplug } from "lucide-react";
import { command, desktop, errorText } from "./api";
import { Modal } from "./Modal";
import { PluginContributionsPanel } from "./PluginContributionsPanel";
import type { PluginInfo } from "./types";

export function PluginPanel({
  workspaceId,
  sessionId,
  revision,
  onChanged,
  onClose,
}: {
  workspaceId: string;
  sessionId: string;
  revision: number;
  onChanged: () => void;
  onClose: () => void;
}) {
  const [plugins, setPlugins] = useState<PluginInfo[]>([]),
    [path, setPath] = useState(""),
    [permissions, setPermissions] = useState(""),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [removeId, setRemoveId] = useState("");
  const load = async () => {
    setPlugins(await command<PluginInfo[]>("plugin_list"));
    onChanged();
  };
  useEffect(() => {
    void load().catch((e) => setError(errorText(e)));
  }, []);
  async function action(fn: () => Promise<unknown>) {
    setBusy(true);
    setError("");
    try {
      await fn();
      await load();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title="插件与权限" onClose={onClose}>
      <p className="muted">从本地导入 WASM 插件。插件仅获得声明、授权与工作区策略的交集权限。</p>
      <p className="muted">
        插件的命令 Hook 需要清单声明并显式授予 hooks.command；直接执行模式下命令以当前用户权限运行。
      </p>
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
      <form
        className="settings-form"
        onSubmit={(e) => {
          e.preventDefault();
          void action(() =>
            command("plugin_install", {
              path,
              permissions: permissions
                .split(",")
                .map((s) => s.trim())
                .filter(Boolean),
            }),
          );
        }}
      >
        <label>
          插件目录或 ZIP 路径
          <div className="input-row">
            <input
              required
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="输入插件完整路径"
            />
            <button
              type="button"
              disabled={!desktop()}
              onClick={async () => {
                try {
                  const result = await open({ multiple: false, directory: true });
                  if (result) setPath(result);
                } catch (e) {
                  setError(errorText(e));
                }
              }}
            >
              浏览目录
            </button>
          </div>
        </label>
        <label>
          授予权限（逗号分隔，默认无权限）
          <input
            value={permissions}
            onChange={(e) => setPermissions(e.target.value)}
            placeholder="按插件清单填写能力名称"
          />
        </label>
        <button className="primary" disabled={busy || !desktop()}>
          导入并授权
        </button>
      </form>
      <div className="plugin-list">
        {!plugins.length && <p className="empty-small muted">暂无已安装插件。</p>}
        {plugins.map((p) => (
          <article key={p.id}>
            <div className="plugin-heading">
              <Unplug size={19} />
              <strong>{p.name}</strong>
              <small>
                {p.version} · {p.kind}
              </small>
            </div>
            <p>{p.description}</p>
            <div className="permission-tags">
              {p.permissions.length ? (
                p.permissions.map((x) => <code key={x}>{x}</code>)
              ) : (
                <span>无额外授权</span>
              )}
            </div>
            <div className="plugin-actions">
              <small>{p.status}</small>
              <button
                disabled={busy}
                onClick={() =>
                  action(() => command("plugin_enable", { id: p.id, enabled: !p.enabled }))
                }
              >
                {p.enabled ? "停用" : "启用"}
              </button>
              <button className="danger" disabled={busy} onClick={() => setRemoveId(p.id)}>
                移除
              </button>
            </div>
            {removeId === p.id && (
              <div className="confirm">
                确定移除此插件？<button onClick={() => setRemoveId("")}>取消</button>
                <button
                  className="danger"
                  onClick={() =>
                    action(async () => {
                      await command("plugin_remove", { id: p.id });
                      setRemoveId("");
                    })
                  }
                >
                  确认移除
                </button>
              </div>
            )}
          </article>
        ))}
      </div>
      <PluginContributionsPanel
        workspaceId={workspaceId}
        sessionId={sessionId}
        revision={revision}
      />
    </Modal>
  );
}
