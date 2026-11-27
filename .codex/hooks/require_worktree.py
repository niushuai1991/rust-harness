#!/usr/bin/env python3
"""Block apply_patch edits in the main checkout; shell commands are not covered."""

import json
import re
import subprocess
import sys
from pathlib import Path


def main():
    event = json.load(sys.stdin)
    cwd = Path(event.get('cwd') or '.').resolve()
    if '.worktrees' in cwd.parts:
        return

    result = subprocess.run(
        ['git', '-C', str(cwd), 'rev-parse', '--show-toplevel'],
        text=True, capture_output=True,
    )
    if result.returncode:
        return
    root = Path(result.stdout.strip()).resolve()
    command = event.get('tool_input', {}).get('command', '')
    paths = re.findall(r'^\*\*\* (?:Add File|Update File|Delete File|Move to): (.+)$', command, re.MULTILINE)
    for name in paths:
        target = (cwd / name.strip()).resolve()
        if not target.is_relative_to(root):
            continue
        if target.is_relative_to(root / '.worktrees'):
            continue
        if target == root / '.claude' / 'settings.local.json':
            continue
        print(json.dumps({'hookSpecificOutput': {
            'hookEventName': 'PreToolUse',
            'permissionDecision': 'deny',
            'permissionDecisionReason': '主工作区禁止直接修改仓库文件，请先在 .worktrees/ 下创建 worktree，再进行修改。',
        }}))
        return


if __name__ == '__main__':
    main()
