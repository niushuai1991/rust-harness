import { useState, type FormEvent } from "react";
import { desktop, errorText } from "./api";

type Ref = { source: "environment" | "credential"; name: string };
type Transport =
  | {
      type: "stdio";
      command: string;
      args: string[];
      cwd?: string | null;
      env?: Record<string, Ref>;
    }
  | { type: "streamableHttp"; url: string; headers?: Record<string, Ref> };
export type ServerConfig = {
  id: string;
  enabled: boolean;
  transport: Transport;
  startupTimeoutSecs?: number;
  toolTimeoutSecs?: number;
  enabledTools?: string[] | null;
  disabledTools?: string[];
};
export type Scope = { kind: "global" } | { kind: "project"; workspaceId: string };

type Entry = { scope: Scope; config: ServerConfig; trusted: boolean };
const blank = (): ServerConfig => ({
  id: "",
  enabled: true,
  transport: { type: "streamableHttp", url: "", headers: {} },
  startupTimeoutSecs: 30,
  toolTimeoutSecs: 60,
  enabledTools: null,
  disabledTools: [],
});
const object = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === "object" && !Array.isArray(v);
function keys(value: Record<string, unknown>, allowed: string[]) {
  const unknown = Object.keys(value).filter((k) => !allowed.includes(k));
  if (unknown.length) throw new Error(`不支持的字段：${unknown.join(", ")}`);
}
function refs(value: unknown): Record<string, Ref> {
  if (value === undefined) return {};
  if (!object(value)) throw new Error("环境变量或请求头必须是引用对象。");
  const result: Record<string, Ref> = Object.create(null);
  for (const [key, v] of Object.entries(value)) {
    if (
      !object(v) ||
      !["environment", "credential"].includes(String(v.source)) ||
      typeof v.name !== "string" ||
      !v.name
    )
      throw new Error(`环境变量或请求头 ${key} 必须使用环境变量或凭据引用，不能使用明文值。`);
    keys(v, ["source", "name"]);
    result[key] = { source: v.source as Ref["source"], name: v.name };
  }
  return result;
}
function stringArray(v: unknown, name: string): string[] {
  if (!Array.isArray(v) || !v.every((x) => typeof x === "string"))
    throw new Error(`${name}必须是字符串数组。`);
  return v;
}
function parseServer(value: unknown, id?: string): ServerConfig {
  if (!object(value)) throw new Error("服务器配置必须是 JSON 对象。");
  const native = "transport" in value;
  keys(
    value,
    native
      ? [
          "id",
          "enabled",
          "transport",
          "startupTimeoutSecs",
          "toolTimeoutSecs",
          "enabledTools",
          "disabledTools",
        ]
      : ["command", "args", "cwd", "env", "url", "headers", "type"],
  );
  const t = native ? value.transport : value;
  if (!object(t)) throw new Error("传输配置无效。");
  const type = native ? t.type : (t.type ?? ("command" in t ? "stdio" : "streamableHttp"));
  if (type !== "stdio" && type !== "streamableHttp" && type !== "http")
    throw new Error("不支持的传输方式。");
  keys(
    t,
    type === "stdio" ? ["type", "command", "args", "cwd", "env"] : ["type", "url", "headers"],
  );
  if (type === "stdio" && typeof t.command !== "string")
    throw new Error("可执行程序必须是字符串。");
  if (type !== "stdio" && typeof t.url !== "string") throw new Error("服务器地址必须是字符串。");
  const serverId = native ? value.id : id;
  if (typeof serverId !== "string") throw new Error("缺少服务器标识。");
  const transport: Transport =
    type === "stdio"
      ? {
          type: "stdio",
          command: typeof t.command === "string" ? t.command : "",
          args: t.args === undefined ? [] : stringArray(t.args, "启动参数"),
          cwd:
            t.cwd == null
              ? null
              : typeof t.cwd === "string"
                ? t.cwd
                : (() => {
                    throw new Error("工作目录必须是字符串。");
                  })(),
          env: refs(t.env),
        }
      : {
          type: "streamableHttp",
          url: typeof t.url === "string" ? t.url : "",
          headers: refs(t.headers),
        };
  if (value.enabled !== undefined && typeof value.enabled !== "boolean")
    throw new Error("启用状态无效。");
  for (const k of ["startupTimeoutSecs", "toolTimeoutSecs"])
    if (value[k] !== undefined && (typeof value[k] !== "number" || !Number.isInteger(value[k])))
      throw new Error(`${k} 必须是整数。`);
  return {
    id: serverId,
    enabled: value.enabled !== false,
    transport,
    startupTimeoutSecs: (value.startupTimeoutSecs as number | undefined) ?? 30,
    toolTimeoutSecs: (value.toolTimeoutSecs as number | undefined) ?? 60,
    enabledTools: value.enabledTools == null ? null : stringArray(value.enabledTools, "允许工具"),
    disabledTools:
      value.disabledTools === undefined ? [] : stringArray(value.disabledTools, "禁止工具"),
  };
}
function parseImport(raw: string): Record<string, ServerConfig> {
  const value: unknown = JSON.parse(raw);
  if (!object(value)) throw new Error("配置必须是 JSON 对象。");
  if ("mcpServers" in value) {
    keys(value, ["mcpServers"]);
    if (!object(value.mcpServers) || !Object.keys(value.mcpServers).length)
      throw new Error("mcpServers 必须包含服务器。");
    return Object.fromEntries(
      Object.entries(value.mcpServers).map(([id, v]) => [id, parseServer(v, id)]),
    );
  }
  const config = parseServer(value);
  return { [config.id]: config };
}

export function McpEditor({
  entry,
  workspaceId,
  busy,
  onSave,
  onCancel,
}: {
  entry?: Entry;
  workspaceId: string;
  busy: boolean;
  onSave: (scope: Scope, config: ServerConfig, create: boolean) => Promise<void>;
  onCancel: () => void;
}) {
  const [config, setConfig] = useState<ServerConfig>(() => entry?.config ?? blank());
  const [scope, setScope] = useState<Scope>(() => entry?.scope ?? { kind: "global" });
  const [args, setArgs] = useState(() =>
    JSON.stringify(config.transport.type === "stdio" ? config.transport.args : []),
  );
  const [allow, setAllow] = useState(() => JSON.stringify(config.enabledTools ?? null));
  const [deny, setDeny] = useState(() => JSON.stringify(config.disabledTools ?? []));
  const [raw, setRaw] = useState("");
  const [imports, setImports] = useState<Record<string, ServerConfig>>({});
  const [selected, setSelected] = useState("");
  const [error, setError] = useState("");
  const [rows, setRows] = useState(() =>
    Object.entries(
      config.transport.type === "stdio"
        ? (config.transport.env ?? {})
        : (config.transport.headers ?? {}),
    ).map(([key, ref]) => ({ key, ...ref })),
  );
  function load(c: ServerConfig) {
    setConfig(c);
    setArgs(JSON.stringify(c.transport.type === "stdio" ? c.transport.args : []));
    setAllow(JSON.stringify(c.enabledTools ?? null));
    setDeny(JSON.stringify(c.disabledTools ?? []));
    setRows(
      Object.entries(
        c.transport.type === "stdio" ? (c.transport.env ?? {}) : (c.transport.headers ?? {}),
      ).map(([key, ref]) => ({ key, ...ref })),
    );
  }
  async function submit(e: FormEvent) {
    e.preventDefault();
    setError("");
    try {
      const allowed: unknown = JSON.parse(allow),
        denied: unknown = JSON.parse(deny);
      const map: Record<string, Ref> = Object.create(null);
      for (const row of rows) {
        if (!row.key.trim() || !row.name.trim() || Object.hasOwn(map, row.key))
          throw new Error("引用名称不能为空或重复。");
        map[row.key] = { source: row.source, name: row.name };
      }
      const transport: Transport =
        config.transport.type === "stdio"
          ? { ...config.transport, args: stringArray(JSON.parse(args), "启动参数"), env: map }
          : { ...config.transport, headers: map };
      const final = {
        ...config,
        transport,
        enabledTools: allowed === null ? null : stringArray(allowed, "允许工具"),
        disabledTools: stringArray(denied, "禁止工具"),
      };
      await onSave(scope, final, !entry);
    } catch (e) {
      setError(errorText(e));
    }
  }
  return (
    <form className="settings-form mcp-editor" onSubmit={(e) => void submit(e)}>
      <h4>{entry ? "编辑服务器" : "添加服务器"}</h4>
      {!entry && (
        <details>
          <summary>从 JSON 导入</summary>
          <label>
            导入服务器 JSON
            <textarea
              value={raw}
              onChange={(e) => {
                setRaw(e.target.value);
                setImports({});
                setSelected("");
              }}
            />
          </label>
          <button
            type="button"
            onClick={() => {
              try {
                const parsed = parseImport(raw);
                setImports(parsed);
                setSelected(Object.keys(parsed)[0]);
                setError("");
              } catch (e) {
                setImports({});
                setError(errorText(e));
              }
            }}
          >
            解析 JSON
          </button>
          {selected && (
            <>
              <label>
                选择导入的服务器
                <select value={selected} onChange={(e) => setSelected(e.target.value)}>
                  {Object.keys(imports).map((k) => (
                    <option key={k} value={k}>
                      {k}
                    </option>
                  ))}
                </select>
              </label>
              <button
                type="button"
                onClick={() => {
                  load(imports[selected]);
                  setError("");
                }}
              >
                导入所选服务器
              </button>
            </>
          )}
        </details>
      )}
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
      <label>
        配置范围
        <select
          value={scope.kind}
          disabled={busy || !!entry}
          onChange={(e) =>
            setScope(
              e.target.value === "project" ? { kind: "project", workspaceId } : { kind: "global" },
            )
          }
        >
          <option value="global">全局</option>
          <option value="project" disabled={!workspaceId}>
            当前项目
          </option>
        </select>
      </label>
      <label>
        服务器标识
        <input
          required
          value={config.id}
          disabled={busy || !!entry}
          onChange={(e) => setConfig({ ...config, id: e.target.value })}
          placeholder="小写字母、数字或连字符"
        />
      </label>
      <label>
        配置传输方式
        <select
          value={config.transport.type}
          disabled={busy}
          onChange={(e) => {
            const transport: Transport =
              e.target.value === "stdio"
                ? { type: "stdio", command: "", args: [], env: {} }
                : { type: "streamableHttp", url: "", headers: {} };
            setConfig({ ...config, transport });
            setRows([]);
          }}
        >
          <option value="streamableHttp">Streamable HTTP</option>
          <option value="stdio">本地 stdio 进程</option>
        </select>
      </label>
      {config.transport.type === "stdio" ? (
        <>
          <label>
            配置可执行程序
            <input
              required
              value={config.transport.command}
              onChange={(e) =>
                setConfig({
                  ...config,
                  transport: {
                    ...(config.transport as Extract<Transport, { type: "stdio" }>),
                    command: e.target.value,
                  },
                })
              }
            />
          </label>
          <label>
            配置启动参数（JSON 字符串数组）
            <input value={args} onChange={(e) => setArgs(e.target.value)} />
          </label>
          <label>
            工作目录（执行根内相对路径）
            <input
              value={config.transport.cwd ?? ""}
              onChange={(e) =>
                setConfig({
                  ...config,
                  transport: {
                    ...(config.transport as Extract<Transport, { type: "stdio" }>),
                    cwd: e.target.value || null,
                  },
                })
              }
            />
          </label>
        </>
      ) : (
        <label>
          配置服务器地址
          <input
            required
            type="url"
            value={config.transport.url}
            onChange={(e) =>
              setConfig({
                ...config,
                transport: {
                  ...(config.transport as Extract<Transport, { type: "streamableHttp" }>),
                  url: e.target.value,
                },
              })
            }
          />
        </label>
      )}
      <fieldset>
        <legend>{config.transport.type === "stdio" ? "环境变量引用" : "请求头引用"}</legend>
        <p className="muted">仅填写环境变量名或系统凭据引用，不输入明文密钥。</p>
        {rows.map((row, index) => (
          <div key={index}>
            <label>
              名称
              <input
                value={row.key}
                onChange={(e) =>
                  setRows(rows.map((r, i) => (i === index ? { ...r, key: e.target.value } : r)))
                }
              />
            </label>
            <label>
              引用类型
              <select
                value={row.source}
                onChange={(e) =>
                  setRows(
                    rows.map((r, i) =>
                      i === index ? { ...r, source: e.target.value as Ref["source"] } : r,
                    ),
                  )
                }
              >
                <option value="environment">环境变量</option>
                <option value="credential">系统凭据</option>
              </select>
            </label>
            <label>
              引用名称
              <input
                value={row.name}
                onChange={(e) =>
                  setRows(rows.map((r, i) => (i === index ? { ...r, name: e.target.value } : r)))
                }
              />
            </label>
            <button type="button" onClick={() => setRows(rows.filter((_, i) => i !== index))}>
              移除引用
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={() => setRows([...rows, { key: "", source: "environment", name: "" }])}
        >
          添加引用
        </button>
      </fieldset>
      <label>
        启动超时（秒）
        <input
          type="number"
          required
          min={1}
          max={3600}
          value={config.startupTimeoutSecs ?? 30}
          onChange={(e) => setConfig({ ...config, startupTimeoutSecs: Number(e.target.value) })}
        />
      </label>
      <label>
        工具调用超时（秒）
        <input
          type="number"
          required
          min={1}
          max={3600}
          value={config.toolTimeoutSecs ?? 60}
          onChange={(e) => setConfig({ ...config, toolTimeoutSecs: Number(e.target.value) })}
        />
      </label>
      <label>
        允许工具（JSON 数组，null 表示全部）
        <input value={allow} onChange={(e) => setAllow(e.target.value)} />
      </label>
      <label>
        禁止工具（JSON 数组）
        <input value={deny} onChange={(e) => setDeny(e.target.value)} />
      </label>
      <label className="mcp-enabled">
        <input
          type="checkbox"
          checked={config.enabled}
          onChange={(e) => setConfig({ ...config, enabled: e.target.checked })}
        />
        启用服务器
      </label>
      <button className="primary" disabled={busy || !desktop()}>
        保存服务器配置
      </button>
      <button type="button" disabled={busy} onClick={onCancel}>
        取消
      </button>
    </form>
  );
}
