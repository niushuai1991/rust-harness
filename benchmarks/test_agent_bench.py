import importlib.util
import json
import pathlib
import sys
import sqlite3
import subprocess
from contextlib import closing
import tempfile
import time
import unittest

MODULE = pathlib.Path(__file__).with_name("agent_bench.py")
spec = importlib.util.spec_from_file_location("agent_bench", MODULE)
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


class BenchmarkTests(unittest.TestCase):
    def test_parse_trace_separates_sessions_and_sums_phases(self):
        stderr = '\n'.join([
            'warning',
            'BENCHMARK {"session_id":"a","phase":"history_read","duration_ms":3.0}',
            'BENCHMARK {"session_id":"a","phase":"history_read","duration_ms":2.0}',
            'BENCHMARK {"session_id":"b","phase":"model_request","duration_ms":12.0}',
        ])
        self.assertEqual(bench.parse_trace(stderr), {
            "a": {"history_read": [3.0, 2.0]},
            "b": {"model_request": [12.0]},
        })

    def test_parse_trace_ignores_malformed_foreign_stderr(self):
        self.assertEqual(bench.parse_trace('BENCHMARK not-json\nBENCHMARK {"phase":"broken"}'), {})

    def test_summary_does_not_hide_failed_runs(self):
        rows = [
            {"ok": True, "elapsed_ms": 100.0},
            {"ok": True, "elapsed_ms": 300.0},
            {"ok": False, "elapsed_ms": 1.0},
        ]
        self.assertEqual(bench.summarize(rows), {"runs": 3, "successes": 2, "failures": 1, "median_ms": 200.0})

    def test_commands_preserve_literal_json_and_validate_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            fixture = root / "fixture"
            fixture.mkdir()
            (fixture / "sample.txt").write_text("original", encoding="utf-8")
            config = {"fixture": str(fixture), "prompt": "inspect", "repeats": 2, "variants": [
                {"name": "pass", "command": [sys.executable, "-c", "print('{\"type\":\"result\",\"status\":\"completed\"}')"], "result_format": "harness"},
                {"name": "fail", "command": [sys.executable, "-c", "print('{\"type\":\"result\",\"status\":\"error\"}')"], "result_format": "harness"},
            ]}
            result = bench.run(config, root / "out")
            self.assertEqual(result["pass"]["successes"], 2)
            self.assertEqual(result["fail"]["failures"], 2)
            self.assertEqual(len((root / "out" / "samples.jsonl").read_text(encoding="utf-8").splitlines()), 4)

    def test_variant_name_cannot_escape_output_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            fixture = root / "fixture"
            fixture.mkdir()
            config = {"fixture": str(fixture), "prompt": "", "repeats": 1,
                      "variants": [{"name": "../outside", "command": [sys.executable, "-c", "pass"]}]}
            with self.assertRaises(ValueError):
                bench.run(config, root / "out")

    def test_unknown_verification_directory_is_rejected(self):
        with bench.bounded_tempdir() as directory:
            root = pathlib.Path(directory)
            fixture = root / "fixture"
            fixture.mkdir()
            config = {"fixture": str(fixture), "prompt": "", "repeats": 1,
                      "variants": [{"name": "v", "command": [sys.executable, "-c", "pass"],
                                    "verify": [sys.executable, "-c", "pass"], "verify_cwd": "replca"}]}
            with self.assertRaises(ValueError):
                bench.run(config, root / "out")

    def test_seed_copies_only_model_configuration_and_applies_switches(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            seed = root / "seed.db"
            with closing(sqlite3.connect(seed)) as db:
                db.execute("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
                db.executemany("INSERT INTO settings VALUES (?,?)", [("models", "[]"), ("providers", "[]"), ("private", '"secret"')])
                db.commit()
            target = root / "data"
            bench.seed_harness(seed, target, {"app.tool_concurrency": False})
            with closing(sqlite3.connect(target / "harness.db")) as db:
                values = dict(db.execute("SELECT key,value FROM settings"))
            self.assertEqual(values, {"models": "[]", "providers": "[]", "app.tool_concurrency": "false"})

    def test_quality_markers_use_final_message_only(self):
        output = '\n'.join(['{"type":"event","event":{"kind":"user.message","data":{"content":"alpha"}}}',
                            '{"type":"result","status":"completed","finalMessage":"beta"}'])
        parsed = bench.harness_output(output)
        self.assertFalse(bench.quality_ok(parsed, ["alpha"]))
        self.assertTrue(bench.quality_ok(parsed, ["beta"]))

    def test_zcode_quality_ignores_projection_contents(self):
        parsed = bench.zcode_output('{"response":"beta","projection":{"content":"alpha"},"usage":{}}')
        self.assertFalse(bench.quality_ok(parsed, ["alpha"]))
        self.assertTrue(bench.quality_ok(parsed, ["beta"]))

    def test_zcode_stream_json_uses_final_result(self):
        output = '{"type":"turn.completed","turnId":"t","payload":{"resultType":"success"}}\n{"type":"result","turnId":"t","response":"right","usage":{}}'
        parsed = bench.zcode_output(output)
        self.assertTrue(bench.quality_ok(parsed, ["right"]))
        self.assertEqual(parsed["turn_result"], "success")

    def test_zcode_missing_or_cancelled_turn_is_not_success(self):
        for finish in ("cancelled", "error_max_budget"):
            out = f'{{"type":"turn.completed","turnId":"t","payload":{{"resultType":"{finish}"}}}}\n{{"type":"result","turnId":"t","response":"right"}}'
            self.assertNotEqual(bench.zcode_output(out)["turn_result"], "success")
        self.assertIsNone(bench.zcode_output('{"response":"right"}')["turn_result"])

    def test_zcode_selection_requires_observed_request_model(self):
        output = '{"payload":{"providerId":"p","modelId":"m","reasoningLevel":"high"}}\n{"type":"result","response":"ok"}'
        parsed = bench.zcode_output(output)
        self.assertTrue(bench.zcode_selection_ok(parsed, {"providerId":"p", "modelId":"m", "reasoningLevel":"high"}))
        self.assertFalse(bench.zcode_selection_ok(parsed, {"providerId":"p", "modelId":"other", "reasoningLevel":"high"}))
        self.assertFalse(bench.zcode_selection_ok(parsed, {"providerId":"p", "modelId":"m", "reasoningLevel":"low"}))

    def test_zcode_error_status_is_not_success(self):
        with bench.bounded_tempdir() as directory:
            root = pathlib.Path(directory)
            fixture = root / "fixture"
            fixture.mkdir()
            config = {"fixture": str(fixture), "prompt": "test", "repeats": 1, "variants": [
                {"name": "z", "result_format": "zcode", "command": [sys.executable, "-c", "print('{\"response\":\"done\",\"projection\":{\"status\":\"error\"}}')"]}]}
            self.assertEqual(bench.run(config, root / "out")["z"]["failures"], 1)

    def test_history_fixture_adds_ordered_message_pairs(self):
        with tempfile.TemporaryDirectory() as directory:
            db_path = pathlib.Path(directory) / "harness.db"
            with closing(sqlite3.connect(db_path)) as db:
                db.execute("CREATE TABLE events(session_id TEXT,seq INTEGER,id TEXT,kind TEXT,data TEXT,created_at TEXT)")
                db.commit()
            bench.insert_history(db_path, "session", {"turns": 3, "content": "固定历史"})
            with closing(sqlite3.connect(db_path)) as db:
                rows = list(db.execute("SELECT seq,kind,data FROM events ORDER BY seq"))
            self.assertEqual([r[1] for r in rows], ["user.message", "assistant.message"] * 3)
            self.assertEqual([r[0] for r in rows], list(range(1, 7)))

    def test_phase_totals_use_each_sample_and_exclude_failures(self):
        rows = [
            {"ok": True, "trace": {"a": {"model_request": [2, 3], "tool_batch": [4]}}},
            {"ok": True, "trace": {"b": {"model_request": [7], "tool_batch": [8]}}},
            {"ok": False, "trace": {"c": {"model_request": [500]}}},
        ]
        self.assertEqual(bench.phase_summary(rows), {"model_request": {"samples": 2, "median_total_ms": 6},
                                                     "tool_batch": {"samples": 2, "median_total_ms": 6}})

    def test_explicit_empty_quality_check_is_unverified(self):
        self.assertIsNone(bench.quality_ok({"final_message": "answer"}, []))

    def test_timeout_terminates_child_that_inherits_output(self):
        if sys.platform != "win32":
            self.skipTest("Windows process tree test")
        with bench.bounded_tempdir() as directory:
            root = pathlib.Path(directory)
            marker = root / "child-survived"
            child = f"import time,pathlib; time.sleep(1); pathlib.Path({str(marker)!r}).write_text('alive')"
            script = f"import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',{child!r}]); time.sleep(8)"
            start = time.monotonic()
            result = bench.run_process([sys.executable, "-c", script], root, {}, 0.3)
            self.assertTrue(result[3])
            self.assertLess(time.monotonic() - start, 3)
            time.sleep(1.1)
            self.assertFalse(marker.exists())

    def test_metadata_records_executable_and_config_fingerprints(self):
        with bench.bounded_tempdir() as directory:
            root = pathlib.Path(directory)
            fixture = root / "fixture"
            fixture.mkdir()
            config = {"fixture": str(fixture), "prompt": "test", "repeats": 1, "variants": [
                {"name": "v", "command": [sys.executable, "-c", "print('ok')"], "fingerprint_files": [str(__file__)]}]}
            bench.run(config, root / "out")
            metadata = json.loads((root / "out" / "metadata.json").read_text(encoding="utf-8"))
            self.assertEqual(len(metadata["variants"][0]["executable_sha256"]), 64)
            self.assertEqual(len(metadata["variants"][0]["command_sha256"]), 64)
            self.assertEqual(len(metadata["variants"][0]["fingerprint_files_sha256"][0]), 64)

    def test_verifier_runs_after_command_and_records_failure(self):
        with bench.bounded_tempdir() as directory:
            root = pathlib.Path(directory)
            fixture = root / "fixture"
            fixture.mkdir()
            config = {"fixture": str(fixture), "prompt": "test", "repeats": 1, "variants": [
                {"name": "v", "command": [sys.executable, "-c", "open('file.txt','w').write('bad')"],
                 "verify": [sys.executable, "-c", "import pathlib,sys; sys.exit(pathlib.Path('file.txt').read_text() != 'good')"]}]}
            result = bench.run(config, root / "out")
            self.assertEqual(result["v"]["failures"], 1)
            sample = json.loads((root / "out" / "samples.jsonl").read_text(encoding="utf-8"))
            self.assertFalse(sample["verification_ok"])

    def test_zcode_seed_is_isolated_and_selects_requested_model(self):
        with bench.bounded_tempdir() as directory:
            root = pathlib.Path(directory)
            source = root / "source" / ".zcode" / "v2"
            source.mkdir(parents=True)
            (source / "credentials.json").write_text('{"key":"encrypted"}', encoding="utf-8")
            (source / "provider_config.json").write_text('{"config":{}}', encoding="utf-8")
            builtin = root / "builtin.json"
            builtin.write_text("{}", encoding="utf-8")
            script = pathlib.Path(__file__).with_name("prepare_zcode.py")
            subprocess.run([sys.executable, str(script), "--source", str(root / "source"), "--output", str(root / "output"), "--builtin", str(builtin)], check=True)
            config = json.loads((root / "output/.zcode/v2/provider_config.json").read_text(encoding="utf-8"))
            self.assertEqual(config["config"]["defaultModelSelection"]["modelId"], "GLM-5.3-Flash")
            self.assertNotIn("defaultModelSelection", json.loads((source / "provider_config.json").read_text(encoding="utf-8"))["config"])


if __name__ == "__main__":
    unittest.main()
