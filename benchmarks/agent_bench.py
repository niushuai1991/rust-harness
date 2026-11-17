"""Repeat external agent commands with isolated fixture copies and JSONL results."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import shutil
import sqlite3
import statistics
import subprocess
import tempfile
import time
import uuid
from contextlib import closing, contextmanager


@contextmanager
def bounded_tempdir():
    directory = tempfile.TemporaryDirectory(prefix="agent-bench-")
    try:
        yield directory.name
    finally:
        for attempt in range(40):
            try:
                directory.cleanup()
                break
            except PermissionError:
                if attempt == 39:
                    raise
                time.sleep(0.05)


def run_process(command, cwd, env, timeout):
    """Bound the whole process tree, including descendants holding inherited output."""
    windows = os.name == "nt"
    job = None
    if windows:
        from ctypes import wintypes
        class Basic(ctypes.Structure):
            _fields_ = [("process_time", ctypes.c_longlong), ("job_time", ctypes.c_longlong),
                        ("flags", wintypes.DWORD), ("min_work", ctypes.c_size_t), ("max_work", ctypes.c_size_t),
                        ("active", wintypes.DWORD), ("affinity", ctypes.c_size_t),
                        ("priority", wintypes.DWORD), ("scheduling", wintypes.DWORD)]
        class Io(ctypes.Structure):
            _fields_ = [(name, ctypes.c_ulonglong) for name in ("read_ops", "write_ops", "other_ops", "read_bytes", "write_bytes", "other_bytes")]
        class Limits(ctypes.Structure):
            _fields_ = [("basic", Basic), ("io", Io), ("process_memory", ctypes.c_size_t),
                        ("job_memory", ctypes.c_size_t), ("peak_process", ctypes.c_size_t), ("peak_job", ctypes.c_size_t)]
        class Accounting(ctypes.Structure):
            _fields_ = [("total_user", ctypes.c_longlong), ("total_kernel", ctypes.c_longlong),
                        ("period_user", ctypes.c_longlong), ("period_kernel", ctypes.c_longlong),
                        ("faults", wintypes.DWORD), ("total", wintypes.DWORD),
                        ("active", wintypes.DWORD), ("terminated", wintypes.DWORD)]
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        kernel.CreateJobObjectW.restype = wintypes.HANDLE
        kernel.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        kernel.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        kernel.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
        kernel.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p]
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        job = kernel.CreateJobObjectW(None, None)
        if not job:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = Limits()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not kernel.SetInformationJobObject(job, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            error = ctypes.WinError(ctypes.get_last_error())
            kernel.CloseHandle(job)
            raise error
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        proc = None
        timed_out = False
        try:
            proc = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stdout, stderr=stderr,
                                    creationflags=(0x4 | subprocess.CREATE_NO_WINDOW) if windows else 0,
                                    start_new_session=not windows)
            if windows:
                if not kernel.AssignProcessToJobObject(job, int(proc._handle)):
                    raise ctypes.WinError(ctypes.get_last_error())
                ntdll = ctypes.WinDLL("ntdll")
                ntdll.NtResumeProcess.argtypes = [wintypes.HANDLE]
                if ntdll.NtResumeProcess(int(proc._handle)) != 0:
                    raise OSError("Unable to resume benchmark process")
            try:
                proc.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
        finally:
            cleanup_error = None
            if windows:
                try:
                    kernel.TerminateJobObject(job, 1)
                    accounting = Accounting()
                    deadline = time.monotonic() + 5
                    while True:
                        if not kernel.QueryInformationJobObject(job, 1, ctypes.byref(accounting), ctypes.sizeof(accounting), None):
                            raise ctypes.WinError(ctypes.get_last_error())
                        if not accounting.active:
                            break
                        if time.monotonic() >= deadline:
                            raise RuntimeError("benchmark process tree did not terminate")
                        time.sleep(0.01)
                except Exception as error:
                    cleanup_error = error
                finally:
                    kernel.CloseHandle(job)
            elif proc is not None:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            if proc is not None:
                if proc.poll() is None:
                    proc.kill()
                proc.wait()
            if cleanup_error is not None:
                raise cleanup_error
        stdout.seek(0)
        stderr.seek(0)
        return (stdout.read().decode("utf-8", errors="replace"), stderr.read().decode("utf-8", errors="replace"),
                proc.returncode, timed_out)


def parse_trace(stderr):
    sessions = {}
    for line in stderr.splitlines():
        if not line.startswith("BENCHMARK "):
            continue
        try:
            record = json.loads(line[len("BENCHMARK "):])
            session, phase, duration = record["session_id"], record["phase"], record["duration_ms"]
            if not isinstance(session, str) or not isinstance(phase, str) or not isinstance(duration, (int, float)) or duration < 0:
                continue
        except (json.JSONDecodeError, TypeError, KeyError):
            continue
        sessions.setdefault(session, {}).setdefault(phase, []).append(duration)
    return sessions


def summarize(rows):
    elapsed = [row["elapsed_ms"] for row in rows if row["ok"]]
    return {"runs": len(rows), "successes": len(elapsed), "failures": len(rows) - len(elapsed),
            "median_ms": statistics.median(elapsed) if elapsed else None}


def phase_summary(rows):
    phases = {}
    for row in rows:
        if not row["ok"]:
            continue
        totals = {}
        for session in row["trace"].values():
            for phase, durations in session.items():
                totals[phase] = totals.get(phase, 0) + sum(durations)
        for phase, total in totals.items():
            phases.setdefault(phase, []).append(total)
    return {phase: {"samples": len(values), "median_total_ms": statistics.median(values)}
            for phase, values in phases.items()}


def expand(items, variables):
    def substitute(item):
        for key, value in variables.items():
            item = item.replace("{" + key + "}", value)
        return item
    return [substitute(str(item)) for item in items]


def harness_output(stdout):
    result, calls, usage = None, 0, []
    for line in stdout.splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("type") == "result":
            result = record
        event = record.get("event", {})
        if event.get("kind") == "tool.call":
            calls += 1
        if event.get("kind") == "assistant.message" and event.get("data", {}).get("usage") is not None:
            usage.append(event["data"]["usage"])
    return {"status": result.get("status") if result else None,
            "session_id": result.get("sessionId") if result else None,
            "workspace_id": result.get("workspaceId") if result else None,
            "final_message": result.get("finalMessage", "") if result else "",
            "tool_calls": calls, "model_usages": usage}


def zcode_output(stdout):
    observed = []
    reasoning = []
    completed = {}
    def collect_reasoning(value):
        if isinstance(value, dict):
            for key, nested in value.items():
                if key == "reasoningLevel" and isinstance(nested, str):
                    reasoning.append(nested)
                collect_reasoning(nested)
        elif isinstance(value, list):
            for nested in value:
                collect_reasoning(nested)
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
            payload = event.get("payload", {}) if isinstance(event, dict) else {}
            if isinstance(event, dict) and event.get("type") == "turn.completed" and isinstance(payload, dict):
                completed[event.get("turnId")] = payload.get("resultType")
            if isinstance(payload, dict) and isinstance(payload.get("providerId"), str) and isinstance(payload.get("modelId"), str):
                observed.append({"providerId": payload["providerId"], "modelId": payload["modelId"]})
            collect_reasoning(event)
        except json.JSONDecodeError:
            continue
    try:
        result = json.loads(stdout)
    except json.JSONDecodeError:
        result = None
        for line in stdout.splitlines():
            try:
                candidate = json.loads(line)
                if isinstance(candidate, dict) and candidate.get("type") == "result":
                    result = candidate
            except json.JSONDecodeError:
                continue
    if not isinstance(result, dict) or not isinstance(result.get("response"), str):
        return None
    return {"final_message": result["response"], "usage": result.get("usage"),
            "event_count": result.get("eventCount"), "session_id": result.get("sessionId"),
            "status": result.get("projection", {}).get("status"), "observed_models": observed,
            "observed_reasoning_levels": reasoning, "turn_result": completed.get(result.get("turnId")) if result.get("turnId") else None}


def zcode_selection_ok(parsed, expected):
    if not parsed or not parsed["observed_models"]:
        return False
    model = {key: expected[key] for key in ("providerId", "modelId")}
    if not all(observed == model for observed in parsed["observed_models"]):
        return False
    if "reasoningLevel" in expected:
        return bool(parsed["observed_reasoning_levels"] and all(level == expected["reasoningLevel"] for level in parsed["observed_reasoning_levels"]))
    return True


def quality_ok(parsed, markers):
    if not markers:
        return None
    return all(marker.casefold() in parsed["final_message"].casefold() for marker in markers)


def fixture_hash(fixture):
    digest = hashlib.sha256()
    for path in sorted(fixture.rglob("*")):
        if path.is_symlink():
            raise ValueError("fixture must not contain symbolic links")
        if path.is_dir():
            digest.update(b"D")
            digest.update(path.relative_to(fixture).as_posix().encode("utf-8"))
            digest.update(b"\0")
        if path.is_file():
            digest.update(b"F")
            digest.update(path.relative_to(fixture).as_posix().encode("utf-8"))
            digest.update(b"\0")
            with path.open("rb") as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(block)
            digest.update(b"\0")
    return digest.hexdigest()


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def value_sha256(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True).encode("utf-8")).hexdigest()


def read_seed(seed):
    # Read only explicit model configuration; never copy session history or mutate the source.
    source_uri = Path(seed).resolve(strict=True).as_uri() + "?mode=ro"
    with closing(sqlite3.connect(source_uri, uri=True)) as source:
        return list(source.execute("SELECT key,value FROM settings WHERE key IN ('models','providers')"))


def seed_harness(seed, target, settings, rows=None):
    target.mkdir()
    if rows is None:
        rows = read_seed(seed)
    with closing(sqlite3.connect(target / "harness.db")) as dest:
        dest.execute("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
        dest.executemany("INSERT INTO settings(key,value) VALUES (?,?)", rows)
        dest.executemany("INSERT OR REPLACE INTO settings(key,value) VALUES (?,?)",
                         [(key, json.dumps(value)) for key, value in settings.items()])
        dest.commit()


def insert_history(db_path, session_id, history):
    turns = history["turns"]
    if not isinstance(turns, int) or turns < 1:
        raise ValueError("history turns must be positive")
    content = history["content"] * history.get("repeat_content", 1)
    records = []
    for index in range(turns):
        for kind, text in (("user.message", f"历史问题 {index + 1}：{content}"),
                           ("assistant.message", f"历史答复 {index + 1}：{content}")):
            records.append((session_id, len(records) + 1, str(uuid.uuid4()), kind,
                            json.dumps({"content": text}, ensure_ascii=False), "2026-01-01T00:00:00Z"))
    with closing(sqlite3.connect(db_path)) as db:
        db.executemany("INSERT INTO events(session_id,seq,id,kind,data,created_at) VALUES (?,?,?,?,?,?)", records)
        db.commit()


def prepare_history(executable, data_dir, workspace, history, env, timeout):
    def call(method, params):
        stdout, _, code, timed_out = run_process([executable, "--data-dir", str(data_dir), "call", method,
                                                  json.dumps(params, ensure_ascii=False)], workspace, env, timeout)
        if code != 0 or timed_out:
            raise RuntimeError(f"history preparation failed: {method}")
        return json.loads(stdout)
    workspace_id = call("workspace.open", {"path": str(workspace)})["id"]
    session_id = call("session.create", {"workspaceId": workspace_id, "title": "Benchmark fixture"})["id"]
    insert_history(data_dir / "harness.db", session_id, history)
    return session_id


def run(config, output):
    names = [v["name"] for v in config["variants"]]
    if not names or len(names) != len(set(names)) or any(not re.fullmatch(r"[A-Za-z0-9_-]+", n) for n in names):
        raise ValueError("variant names must be unique and contain only letters, numbers, underscore or hyphen")
    if not isinstance(config["repeats"], int) or config["repeats"] < 1:
        raise ValueError("repeats must be positive")
    for variant in config["variants"]:
        if variant.get("verify_cwd", "workspace") not in ("workspace", "replica"):
            raise ValueError("verify_cwd must be workspace or replica")
        if variant.get("verify_cwd") == "replica" and variant.get("result_format") != "harness":
            raise ValueError("replica verification requires harness output")
    fixture = Path(config["fixture"]).resolve(strict=True)
    if not fixture.is_dir():
        raise ValueError("fixture must be a directory")
    digest = fixture_hash(fixture)
    seeds = {v["harness_seed_database"]: read_seed(v["harness_seed_database"])
             for v in config["variants"] if v.get("harness_seed_database")}
    output.mkdir(parents=True, exist_ok=False)
    metadata = {"fixture_sha256": digest,
                "prompt_sha256": hashlib.sha256(config["prompt"].encode("utf-8")).hexdigest(),
                "metadata": config.get("metadata", {}),
                "variants": [{"name": v["name"], "harness_settings": v.get("harness_settings", {}),
                              "history_sha256": value_sha256(v.get("history")),
                              "command_sha256": value_sha256(v["command"]),
                              "executable_sha256": file_sha256(shutil.which(v["command"][0])) if shutil.which(v["command"][0]) else None,
                              "fingerprint_files_sha256": [file_sha256(path) for path in v.get("fingerprint_files", [])],
                              "provider_config_sha256": value_sha256(seeds[v["harness_seed_database"]]) if v.get("harness_seed_database") else None}
                             for v in config["variants"]]}
    (output / "metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2), encoding="utf-8")
    rows = []
    # Interleave variants to reduce correlation with changing network conditions.
    for repeat in range(config["repeats"]):
        offset = repeat % len(config["variants"])
        for variant in config["variants"][offset:] + config["variants"][:offset]:
            with bounded_tempdir() as temp:
                root = Path(temp)
                workspace = root / "workspace"
                shutil.copytree(fixture, workspace)
                if fixture_hash(workspace) != digest:
                    raise ValueError("fixture changed during benchmark")
                variables = {"workspace": str(workspace), "run_dir": str(root),
                             "data_dir": str(root / "data"), "prompt": config["prompt"], "repeat": str(repeat)}
                if variant.get("harness_seed_database"):
                    seed_harness(variant["harness_seed_database"], root / "data", variant.get("harness_settings", {}),
                                 seeds[variant["harness_seed_database"]])
                env = os.environ.copy()
                env.update({key: expand([value], variables)[0] for key, value in variant.get("env", {}).items()})
                if variant.get("history"):
                    if not variant.get("harness_seed_database"):
                        raise ValueError("history requires an isolated harness seed database")
                    variables["session_id"] = prepare_history(variant["command"][0], root / "data", workspace,
                                                              variant["history"], env,
                                                              config.get("timeout_seconds", 1800))
                for setup in variant.get("setup", []):
                    _, _, setup_code, setup_timeout = run_process(expand(setup, variables), workspace, env,
                                                                   config.get("timeout_seconds", 1800))
                    if setup_timeout or setup_code != 0:
                        raise RuntimeError("benchmark setup failed")
                started = time.perf_counter()
                stdout, stderr, code, timed_out = run_process(expand(variant["command"], variables), workspace,
                                                              env, config.get("timeout_seconds", 1800))
                elapsed = (time.perf_counter() - started) * 1000
                stem = f'{repeat:03d}-{variant["name"]}'
                (output / f"{stem}.stdout.jsonl").write_text(stdout, encoding="utf-8")
                (output / f"{stem}.stderr.log").write_text(stderr, encoding="utf-8")
                parsed = harness_output(stdout) if variant.get("result_format") == "harness" else None
                zcode = zcode_output(stdout) if variant.get("result_format") == "zcode" else None
                quality_source = parsed or zcode or {"final_message": stdout}
                quality = quality_ok(quality_source, config.get("required_markers", []))
                selection_ok = zcode_selection_ok(zcode, variant["expected_zcode_selection"]) if "expected_zcode_selection" in variant else None
                verification_ok = None
                verification_error = None
                verification_dir = workspace
                if variant.get("verify"):
                    if variant.get("verify_cwd") == "replica":
                        if parsed is None or not parsed["workspace_id"]:
                            verification_error = "harness workspace ID unavailable"
                        else:
                            listing, _, listing_code, listing_timeout = run_process(
                                [variant["command"][0], "--data-dir", str(root / "data"), "call", "workspace.list"],
                                workspace, env, config.get("timeout_seconds", 1800))
                            if listing_code or listing_timeout:
                                verification_error = "workspace lookup failed"
                            else:
                                candidates = [w for w in json.loads(listing) if w["id"] == parsed["workspace_id"]]
                                if not candidates:
                                    verification_error = "harness workspace not found"
                                else:
                                    verification_dir = Path(candidates[0]["replicaPath"])
                    variables["stdout_path"] = str((output / f"{stem}.stdout.jsonl").resolve())
                    variables["sample_output"] = str((output / stem).resolve())
                    if verification_error is None:
                        verify_out, verify_err, verify_code, verify_timeout = run_process(
                            expand(variant["verify"], variables), verification_dir, env,
                            config.get("verification_timeout_seconds", 300))
                        (output / f"{stem}.verify.stdout.log").write_text(verify_out, encoding="utf-8")
                        (output / f"{stem}.verify.stderr.log").write_text(verify_err, encoding="utf-8")
                        verification_ok = verify_code == 0 and not verify_timeout
                    else:
                        verification_ok = False
                ok = code == 0 and not timed_out and (parsed is None or parsed["status"] == "completed") and (variant.get("result_format") != "zcode" or (zcode is not None and zcode["status"] != "error" and zcode["turn_result"] == "success")) and quality is not False and verification_ok is not False and selection_ok is not False
                if not ok and variant.get("preserve_on_failure"):
                    shutil.copytree(verification_dir, output / f"{stem}.failed-workspace", symlinks=True)
                row = {"variant": variant["name"], "repeat": repeat, "elapsed_ms": elapsed,
                       "ok": ok, "exit_code": code, "timed_out": timed_out,
                       "quality_ok": quality,
                       "selection_ok": selection_ok,
                       "verification_ok": verification_ok, "verification_error": verification_error,
                       "harness": parsed,
                       "zcode": zcode,
                       "trace": parse_trace(stderr)}
                rows.append(row)
                with (output / "samples.jsonl").open("a", encoding="utf-8") as stream:
                    stream.write(json.dumps(row, ensure_ascii=False) + "\n")
    summary = {}
    for variant in config["variants"]:
        variant_rows = [r for r in rows if r["variant"] == variant["name"]]
        summary[variant["name"]] = {**summarize(variant_rows), "phases": phase_summary(variant_rows)}
    (output / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    config = json.loads(args.config.read_text(encoding="utf-8"))
    print(json.dumps(run(config, args.output), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
