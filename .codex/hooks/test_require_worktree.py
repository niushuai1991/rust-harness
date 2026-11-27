import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HOOK = Path(__file__).with_name('require_worktree.py')


class WorktreeHookTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.root = Path(cls.temp.name) / 'repo'
        cls.root.mkdir()
        subprocess.run(['git', 'init', '-q', str(cls.root)], check=True)
        subprocess.run(['git', '-C', str(cls.root), '-c', 'user.name=Test', '-c', 'user.email=test@example.com', 'commit', '--allow-empty', '-qm', 'init'], check=True)
        cls.worktree = cls.root / '.worktrees' / 'task'
        subprocess.run(['git', '-C', str(cls.root), 'worktree', 'add', '-q', '-b', 'task', str(cls.worktree)], check=True)

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def decision(self, cwd, patch):
        result = subprocess.run([sys.executable, str(HOOK)], input=json.dumps({'cwd': str(cwd), 'tool_name': 'apply_patch', 'tool_input': {'command': patch}}), text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)['hookSpecificOutput']['permissionDecision'] if result.stdout else 'allow'

    def test_main_repo_relative_path_denied(self):
        self.assertEqual(self.decision(self.root, '*** Begin Patch\n*** Add File: src/a.txt\n+x\n*** End Patch'), 'deny')

    def test_worktree_target_allowed(self):
        self.assertEqual(self.decision(self.root, f'*** Begin Patch\n*** Add File: {self.worktree}/a.txt\n+x\n*** End Patch'), 'allow')

    def test_worktree_session_allowed(self):
        self.assertEqual(self.decision(self.worktree, '*** Begin Patch\n*** Update File: a.txt\n@@\n-a\n+b\n*** End Patch'), 'allow')

    def test_outside_file_allowed(self):
        outside = Path(self.temp.name) / 'outside.txt'
        self.assertEqual(self.decision(self.root, f'*** Begin Patch\n*** Add File: {outside}\n+x\n*** End Patch'), 'allow')

    def test_move_into_repo_denied(self):
        outside = Path(self.temp.name) / 'outside.txt'
        self.assertEqual(self.decision(self.root, f'*** Begin Patch\n*** Update File: {outside}\n*** Move to: src/a.txt\n@@\n-a\n+b\n*** End Patch'), 'deny')

    def test_local_claude_settings_allowed(self):
        self.assertEqual(self.decision(self.root, '*** Begin Patch\n*** Add File: .claude/settings.local.json\n+{}\n*** End Patch'), 'allow')


if __name__ == '__main__':
    unittest.main()
