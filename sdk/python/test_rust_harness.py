"""Exercise the public SDK against a real headless process, without network/API keys."""
import os
from pathlib import Path
import tempfile
import unittest
from rust_harness import Harness

EXE = Path(os.environ.get("HARNESS_EXE", Path(__file__).resolve().parents[2] / "target/debug/harness.exe"))

@unittest.skipUnless(EXE.exists(), "Build harness-cli first")
class ProviderApiTest(unittest.TestCase):
    def test_close_releases_stdout_pipe(self):
        with tempfile.TemporaryDirectory() as data:
            client = Harness(str(EXE), data)
            client.close()
            self.assertTrue(client.process.stdout.closed)

    def test_supplier_protocol_model_crud_and_builtin_protection(self):
        with tempfile.TemporaryDirectory() as data, Harness(str(EXE), data) as client:
            providers = client.list_providers()
            # PR #36 起内置供应商含 chatgpt（OAuth 账号）。
            self.assertEqual(
                [p["id"] for p in providers], ["deepseek", "bigmodel", "chatgpt"]
            )
            self.assertEqual(len(providers[1]["endpoints"]), 3)
            changed = dict(providers[0], name="overridden")
            with self.assertRaises(RuntimeError):
                client.save_provider(changed)
            copy = client.copy_provider("deepseek")
            self.assertEqual(copy["compatibility"], "deepSeek")
            self.assertNotEqual(copy["credentialRef"], providers[0]["credentialRef"])
            self.assertFalse(copy["builtin"])
            copy["name"] = "Local custom"
            copy["endpoints"] = [{"apiMode": "responses", "baseUrl": "http://127.0.0.1:1/v1"}]
            copy = client.save_provider(copy)
            model = client.save_model({
                "id": "", "name": "fixture", "providerId": copy["id"], "apiMode": "responses",
                "model": "fixture", "maxOutputTokens": 128, "baseUrl": "", "credentialRef": "", "proxy": None,
            })
            self.assertEqual(model["baseUrl"], copy["endpoints"][0]["baseUrl"])
            self.assertEqual(model["compatibility"], "deepSeek")
            models = client.list_models()
            # model_list 含合成的内置目录条目；自定义模型排在其中且可被识别。
            self.assertTrue(all(m["builtin"] for m in models if m["id"] != model["id"]))
            self.assertIn(model["id"], [m["id"] for m in models])
            self.assertIn(model["id"], [m["id"] for m in models if m["providerId"] == copy["id"]])
            with self.assertRaises(RuntimeError):
                client.remove_provider(copy["id"])
            client.remove_model(model["id"])
            client.remove_provider(copy["id"])
            self.assertEqual(len(client.list_providers()), 3)
            self.assertEqual([m for m in client.list_models() if not m["builtin"]], [])
            self.assertTrue(all(m["enabled"] for m in client.list_models()))

if __name__ == "__main__":
    unittest.main()

