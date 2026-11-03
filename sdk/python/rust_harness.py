"""Standard-library JSON-RPC client for the local Rust Harness process."""
import concurrent.futures
import json
import subprocess
import threading


class Harness:
    def __init__(self, executable="harness", data_dir=None, on_event=None):
        args = [executable] + (["--data-dir", str(data_dir)] if data_dir else []) + ["rpc"]
        self.process = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        text=True, encoding="utf-8", bufsize=1)
        self.on_event = on_event
        self._lock = threading.Lock()
        self._pending = {}
        self._next = 0
        self._reader = threading.Thread(target=self._read, daemon=True)
        self._reader.start()

    def _read(self):
        try:
            for line in self.process.stdout:
                value = json.loads(line)
                if "id" in value:
                    with self._lock:
                        future = self._pending.pop(value["id"], None)
                    if future is not None:
                        if "error" in value:
                            future.set_exception(RuntimeError(value["error"]))
                        else:
                            future.set_result(value.get("result"))
                elif self.on_event:
                    # Keep the reader available for responses, even when a callback issues a call.
                    threading.Thread(target=self.on_event, args=(value,), daemon=True).start()
        finally:
            with self._lock:
                for future in self._pending.values():
                    future.set_exception(EOFError("Rust Harness process disconnected"))
                self._pending.clear()

    def request(self, method, params=None):
        future = concurrent.futures.Future()
        with self._lock:
            self._next += 1
            key = self._next
            self._pending[key] = future
            try:
                self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": key,
                    "method": method, "params": params or {}}, ensure_ascii=False) + "\n")
                self.process.stdin.flush()
            except Exception as error:
                self._pending.pop(key, None)
                future.set_exception(error)
        return future

    def call(self, method, params=None, timeout=None):
        return self.request(method, params).result(timeout)

    def list_providers(self):
        return self.call("provider.list")

    def save_provider(self, config, secret=None):
        return self.call("provider.save", {"config": config, "secret": secret})

    def copy_provider(self, provider_id):
        return self.call("provider.copy", {"id": provider_id})

    def remove_provider(self, provider_id):
        return self.call("provider.remove", {"id": provider_id})

    def list_models(self):
        return self.call("model.list")

    def save_model(self, config, secret=None):
        return self.call("model.save", {"config": config, "secret": secret})

    def remove_model(self, model_id):
        return self.call("model.remove", {"modelId": model_id})

    def test_model(self, model_id):
        return self.call("model.test", {"modelId": model_id})

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait()
        self._reader.join(timeout=2)
        self.process.stdout.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
