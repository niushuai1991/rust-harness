#!/usr/bin/env bash
# require-worktree.sh — PreToolUse hook (matcher: Edit|Write|NotebookEdit)
#
# 强制 AGENTS.md 的 worktree 工作流：主工作区禁止直接修改仓库文件，
# 必须先从 main 检出 fix/<任务描述> 分支并在 .worktrees/ 下创建
# worktree，在 worktree 中修改后走 PR。
#
# 放行：session 已在 worktree 中 / 目标文件在 .worktrees/ 检出内 /
#       目标在仓库外 / 目标是 .claude/settings.local.json（本地配置白名单，
#       不进 git）/ 无文件路径 / cwd 不在 git 仓库内
# 拒绝：其余主仓库内的文件写操作（输出 permissionDecision=deny）

input=$(cat)

file_path=$(printf '%s' "$input" | jq -r '.tool_input.file_path // .tool_input.notebook_path // empty')
cwd=$(printf '%s' "$input" | jq -r '.cwd // empty')

# Windows 反斜杠统一为正斜杠；cwd 缺失时退回 hook 进程工作目录（= session cwd）
file_path="${file_path//\\//}"
cwd="${cwd//\\//}"
cwd="${cwd:-$PWD}"

# 拿不到目标文件路径 → 不归此 hook 管
[ -z "$file_path" ] && exit 0

# session 已在 worktree 中（AGENTS.md/EnterWorktree 约定 worktree 位于 .worktrees/ 下）
case "$cwd" in
  */.worktrees/*) exit 0 ;;
esac

# 目标文件本身位于某个 worktree 检出内 → 允许（改的是 worktree 分支的文件）
case "$file_path" in
  */.worktrees/*) exit 0 ;;
esac

# 主仓库根；cwd 不在 git 仓库内则放行
repo_root=$(git -C "$cwd" rev-parse --show-toplevel 2>/dev/null) || exit 0
[ -z "$repo_root" ] && exit 0
repo_root="${repo_root//\\//}"

# 白名单：settings.local.json 是纯本地配置（被全局 gitignore），走 worktree 无意义
[ "$file_path" = "$repo_root/.claude/settings.local.json" ] && exit 0

# 目标不在主仓库内 → 放行
case "$file_path" in
  "$repo_root"/*) ;;
  *) exit 0 ;;
esac

# 主工作区内的仓库文件 → 拒绝
reason="主工作区禁止直接修改仓库文件（AGENTS.md worktree 工作流）：$file_path 。请先从 main 检出 fix/<任务描述> 分支，在 .worktrees/ 下创建 worktree（EnterWorktree 或 git worktree add），切换到 worktree 中修改，经 PR 合并回 main。"
jq -n --arg r "$reason" \
  '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
exit 0
