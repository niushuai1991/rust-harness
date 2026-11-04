"""Opt-in live P1 agent acceptance. Writes only isolated .cache fixtures; never exports secrets."""
from pathlib import Path
import argparse, concurrent.futures, datetime, json, os, shutil, sqlite3, sys, threading, time
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "sdk/python"))
from rust_harness import Harness

MODES = ("chatCompletions", "responses", "anthropic")
SCRIPT_COMMAND = "& ./verify.ps1"
PROMPT = """执行这个小型验收任务，只使用 read、write 和 shell 工具，严格按顺序：
1. read greet.txt，确认内容为 old。
2. 用 write 把 greet.txt 内容改为 new。不要用 edit，不要修改其他文件。
3. 调用 shell，command 字段必须完全等于 & ./verify.ps1（不加引号、前后空格、额外参数或其他命令）。
4. 工具输出 PASS 后，简短回答已完成。不要创建子代理、计划、目标或使用其他工具。不读取环境变量或凭据，不自行同步原项目。
"""

def allowed(request):
    args = request.get("arguments", {})
    return ((request.get("tool") == "write" and args.get("path") == "greet.txt"
             and isinstance(args.get("content"), str) and args["content"].strip() == "new")
            or (request.get("tool") == "shell" and args.get("command") == SCRIPT_COMMAND))


def live_case(executable, data, output, provider, mode, model):
    started = time.monotonic()
    item = {"provider": provider, "apiMode": mode, "maxOutputTokens": 4096,
            "startedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(), "checks": {}}
    source = output / (provider + "-" + mode) / "source"
    source.mkdir(parents=True, exist_ok=False)
    (source / "greet.txt").write_text("old", encoding="utf-8")
    verify = "$ErrorActionPreference = 'Stop'\nif ((Get-Content -LiteralPath './greet.txt' -Raw).Trim() -ne 'new') { throw 'unexpected greeting' }\nWrite-Output 'PASS'\n"
    (source / "verify.ps1").write_text(verify, encoding="utf-8")
    config = dict(model)
    config.update(id="p1-agent-" + provider + "-" + mode, apiMode=mode, baseUrl="", maxOutputTokens=4096)
    events = []
    seen = set()
    lock = threading.Lock()
    requests = 0
    denied = 0
    session_id = None
    budget_cancelled = threading.Event()
    client = None

    def event(notification):
        nonlocal requests, denied
        payload = notification.get("params", {})
        durable = payload.get("event")
        if not isinstance(durable, dict) or durable.get("sessionId") != session_id:
            return
        with lock:
            if durable.get("id") in seen:
                return
            seen.add(durable.get("id"))
            events.append(durable)
            kind = durable.get("kind")
            if kind == "request.snapshot":
                requests += 1
                # Cancel the eighth request immediately: at most eight starts, no ninth iteration.
                if requests >= 8:
                    budget_cancelled.set()
                    client.request("session.cancel", {"sessionId": session_id})
            if kind == "approval.request":
                request = durable.get("data", {})
                allow = allowed(request)
                if not allow:
                    denied += 1
                client.request("approval.resolve", {"id": request["id"], "allow": allow})

    try:
        with Harness(str(executable), data, on_event=event) as client:
            client.call("initialize", timeout=30)
            client.save_model(config)
            workspace = client.call("workspace.open", {"path": str(source)}, timeout=30)
            session = client.call("session.create", {"workspaceId": workspace["id"], "title": "P1 live agent"}, timeout=30)
            session_id = session["id"]
            future = client.request("session.send", {"sessionId": session_id, "modelId": config["id"], "text": PROMPT})
            deadline = started + 240
            while not future.done() and time.monotonic() < deadline:
                time.sleep(0.1)
            if not future.done():
                budget_cancelled.set()
                client.call("session.cancel", {"sessionId": session_id}, timeout=5)
            future.result(timeout=20)
            recorded = client.call("session.events", {"sessionId": session_id}, timeout=15)
            results = [e for e in recorded if e["kind"] == "tool.result"]
            request_count = sum(e["kind"] == "request.snapshot" for e in recorded)
            names = [e["data"].get("name") for e in results]
            successful = [e for e in results if not e["data"].get("isError")]
            shell_pass = any(e["data"].get("name") == "shell" and "PASS" in e["data"].get("content", "") and not e["data"].get("isError") for e in results)
            needed = [e["data"].get("name") for e in successful if e["data"].get("name") in ("read", "write", "shell")]
            ordered = all(name in needed for name in ("read", "write", "shell")) and needed.index("read") < needed.index("write") < needed.index("shell")
            checks = item["checks"]
            checks.update(realToolResults=bool(results), readWriteShellOrder=ordered, shellPass=shell_pass,
                          sourceUnchangedBeforeSync=(source / "greet.txt").read_text(encoding="utf-8") == "old",
                          replicaChanged=(Path(workspace["replicaPath"]) / "greet.txt").read_text(encoding="utf-8").strip() == "new",
                          onlyAllowedTools=all(n in ("read", "write", "shell") for n in names),
                          noToolErrors=len(successful) == len(results), finalAnswer=any(e["kind"] == "assistant.message" and bool(e["data"].get("content")) for e in recorded),
                          withinBudget=request_count <= 8 and not budget_cancelled.is_set())
            item.update(modelRequests=request_count, toolResults=len(results), deniedApprovals=denied,
                        tools={name: names.count(name) for name in sorted(set(names))})
            if not all(checks.values()):
                raise AssertionError("agent acceptance checks failed")
            reviewed = client.call("workspace.diff", {"workspaceId": workspace["id"]}, timeout=15)
            checks["reviewedOnlyGreeting"] = len(reviewed) == 1 and reviewed[0]["path"] == "greet.txt" and not reviewed[0].get("conflict") and reviewed[0].get("beforeText", "").strip() == "old" and reviewed[0].get("afterText", "").strip() == "new"
            if not checks["reviewedOnlyGreeting"]:
                raise AssertionError("unexpected replica changes")
            client.call("workspace.apply", {"workspaceId": workspace["id"], "reviewed": reviewed}, timeout=20)
            checks["reviewedSourceSync"] = (source / "greet.txt").read_text(encoding="utf-8").strip() == "new"
            checks["verificationScriptUnchanged"] = (source / "verify.ps1").read_text(encoding="utf-8") == verify
            prior_ids = [e["id"] for e in recorded]
        with Harness(str(executable), data) as restarted:
            after = restarted.call("session.events", {"sessionId": session_id}, timeout=30)
            checks["historyAfterRestart"] = [e["id"] for e in after] == prior_ids
            checks["workspaceAfterRestart"] = any(w["id"] == workspace["id"] for w in restarted.call("workspace.list", timeout=15))
        item["status"] = "passed" if all(checks.values()) else "failed"
    except Exception as error:
        # Never persist model responses, request snapshots, API exception bodies or credentials.
        item.update(status="failed", errorType=type(error).__name__)
        with lock:
            item.setdefault("modelRequests", requests)
            item.setdefault("deniedApprovals", denied)
    item["elapsedSeconds"] = round(time.monotonic() - started, 2)
    return item


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--live", action="store_true", required=True)
    parser.add_argument("--provider", action="append", choices=["deepseek", "bigmodel"])
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    if not output.is_relative_to((repo / ".cache").resolve()):
        raise ValueError("Acceptance output must be under repository .cache")
    output.mkdir(parents=True, exist_ok=False)
    source_db = Path(os.environ["LOCALAPPDATA"]) / "RustHarness/harness.db"
    with sqlite3.connect(source_db.as_uri() + "?mode=ro", uri=True, timeout=5) as con:
        settings = {}
        for name in ("providers", "models"):
            row = con.execute("SELECT value FROM settings WHERE key=?", (name,)).fetchone()
            settings[name] = json.loads(row[0]) if row else []
    executable = output / "harness-agent-live.exe"
    shutil.copyfile(repo / "target/debug/harness.exe", executable)
    data = output / "host"
    with Harness(str(executable), data) as client:
        client.call("initialize", timeout=30)
    with sqlite3.connect(data / "harness.db") as con:
        for name, value in settings.items():
            con.execute("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (name, json.dumps(value, ensure_ascii=False)))
    results = []
    for provider in args.provider or ("deepseek", "bigmodel"):
        models = [m for m in settings["models"] if m.get("providerId") == provider]
        for mode in MODES:
            item = live_case(executable, data, output, provider, mode, models[0]) if models else {"provider": provider, "apiMode": mode, "status": "missing-configuration"}
            results.append(item)
            (output / "live-agent-results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
            print(json.dumps(item, ensure_ascii=True), flush=True)
    return 0 if all(item["status"] == "passed" for item in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
