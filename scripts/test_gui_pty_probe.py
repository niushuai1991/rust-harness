import os
from pathlib import Path
import subprocess
import tempfile
import unittest


PROBE = Path(os.environ.get("GUI_PTY_PROBE", "target/debug/examples/gui_pty.exe")).resolve()
OUTPUT = "Write-Output ('gui-pty-' + 'probe-marker')"
MARKER = "[IO.File]::WriteAllText('marker.txt','ok')"


class GuiPtyProbeTests(unittest.TestCase):
    def run_probe(self, command, timeout_ms=5000):
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.txt"
            result = subprocess.run(
                [str(PROBE), "--report", str(report), "--work-dir", directory, "--command", command,
                 "--timeout-ms", str(timeout_ms)],
                timeout=20,
                check=False,
            )
            return result.returncode, report.read_text(encoding="utf-8") if report.exists() else ""

    def test_missing_output_fails_even_if_command_was_echoed(self):
        code, report = self.run_probe(f"# {OUTPUT}\n{MARKER}")
        self.assertNotEqual(code, 0, report)
        self.assertIn("output", report.lower())

    def test_missing_marker_fails(self):
        code, report = self.run_probe(OUTPUT)
        self.assertNotEqual(code, 0, report)
        self.assertIn("marker", report.lower())

    def test_wrong_marker_content_fails(self):
        code, report = self.run_probe(f"{OUTPUT}; [IO.File]::WriteAllText('marker.txt','wrong')")
        self.assertNotEqual(code, 0, report)
        self.assertIn("wrong", report)

    def test_timeout_fails(self):
        code, report = self.run_probe("Start-Sleep -Seconds 5", timeout_ms=300)
        self.assertNotEqual(code, 0, report)
        self.assertIn("timeout", report.lower())

    def test_terminal_exit_before_result_fails(self):
        code, report = self.run_probe("exit")
        self.assertNotEqual(code, 0, report)
        self.assertIn("exited early", report.lower())

    def test_real_output_and_marker_pass(self):
        code, report = self.run_probe(f"{OUTPUT}; {MARKER}")
        self.assertEqual(code, 0, report)
        self.assertIn("source=", report)


if __name__ == "__main__":
    unittest.main()
