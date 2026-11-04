"""Explicit live-provider P1 smoke, using credential references without exporting API keys."""
from pathlib import Path
import argparse, datetime, json, os, shutil, sqlite3, sys, time
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "sdk/python"))
from rust_harness import Harness

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--live", action="store_true", required=True)
    parser.add_argument("--provider", action="append", choices=["deepseek", "bigmodel"])
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    if not output.is_relative_to((repo / ".cache").resolve()):
        raise ValueError("Acceptance output must be under this repository's .cache")
    output.mkdir(parents=True, exist_ok=True)
    source = Path(os.environ["LOCALAPPDATA"]) / "RustHarness/harness.db"
    con = sqlite3.connect(source.as_uri() + "?mode=ro", uri=True, timeout=5)
    settings = {}
    for name in ["providers", "models"]:
        row = con.execute("SELECT value FROM settings WHERE key=?", (name,)).fetchone()
        settings[name] = json.loads(row[0]) if row else []
    con.close()
    executable = output / "harness-live.exe"
    shutil.copyfile(repo / "target/debug/harness.exe", executable)
    data = output / "host"
    with Harness(str(executable), data) as client:
        client.call("initialize", timeout=30)
    con = sqlite3.connect(data / "harness.db")
    for name, value in settings.items():
        con.execute("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    (name, json.dumps(value, ensure_ascii=False)))
    con.commit()
    con.close()
    results = []
    report = output / "live-model-results.json"
    for provider in args.provider or ["deepseek", "bigmodel"]:
        configured = [m for m in settings["models"] if m.get("providerId") == provider]
        if not configured:
            results.append({"provider": provider, "status": "missing-configuration"})
            report.write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
            print(json.dumps(results[-1], ensure_ascii=True), flush=True)
            continue
        for mode in ["chatCompletions", "responses", "anthropic"]:
            config = dict(configured[0])
            config.update(id="p1-" + provider + "-" + mode, apiMode=mode, baseUrl="", maxOutputTokens=512)
            item = {"provider": provider, "apiMode": mode, "model": config["model"],
                    "maxOutputTokens": 512, "startedAt": datetime.datetime.now(datetime.timezone.utc).isoformat()}
            start = time.monotonic()
            try:
                with Harness(str(executable), data) as client:
                    client.save_model(config)
                    client.call("model.test", {"modelId": config["id"]}, timeout=205)
                item["status"] = "passed"
            except Exception as error:
                item.update(status="failed", error=str(error))
            item["elapsedSeconds"] = round(time.monotonic() - start, 2)
            results.append(item)
            report.write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
            print(json.dumps(item, ensure_ascii=True), flush=True)
    if len(results) != 3 * len(args.provider or ["deepseek", "bigmodel"]) or any(r["status"] != "passed" for r in results):
        return 1
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
