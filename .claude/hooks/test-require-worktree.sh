#!/usr/bin/env bash
# test-require-worktree.sh — require-worktree.sh 的管道测试
# 用合成的 hook stdin JSON 逐一验证放行/拒绝逻辑。
# 用法: bash .claude/hooks/test-require-worktree.sh   (退出码 0 = 全部通过)
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK="$HERE/require-worktree.sh"

# 主仓库根：worktree 的 git-common-dir(.git) 的上一级
GIT_COMMON="$(git -C "$HERE" rev-parse --git-common-dir)"
MAIN_REPO="$(cd "$GIT_COMMON/.." && pwd -W 2>/dev/null || pwd)"
WORKTREE="$(git -C "$HERE" rev-parse --show-toplevel)"

pass=0
fail=0

run_case() { # $1=用例名 $2=期望(deny|allow) $3=stdin JSON
  local name="$1" expected="$2" json="$3" out rc
  out="$(printf '%s' "$json" | bash "$HOOK" 2>/dev/null)"
  rc=$?
  if [ "$expected" = "deny" ]; then
    if [ "$rc" -eq 0 ] && printf '%s' "$out" | jq -e '.hookSpecificOutput.permissionDecision == "deny"' >/dev/null 2>&1; then
      pass=$((pass + 1))
    else
      fail=$((fail + 1))
      echo "FAIL $name (期望 deny, rc=$rc, out=$out)"
    fi
  else
    if [ "$rc" -eq 0 ] && [ -z "$out" ]; then
      pass=$((pass + 1))
    else
      fail=$((fail + 1))
      echo "FAIL $name (期望放行, rc=$rc, out=$out)"
    fi
  fi
}

# --- 拒绝场景：主工作区 session 修改主仓库内文件 ---

# a. Edit 主仓库源码 → 拒绝
run_case "main-edit-repo-file" deny \
  "{\"tool_name\":\"Edit\",\"cwd\":\"$MAIN_REPO\",\"tool_input\":{\"file_path\":\"$MAIN_REPO/Common/HUtil32.pas\"}}"

# b. Write 主仓库新文件 → 拒绝
run_case "main-write-repo-file" deny \
  "{\"tool_name\":\"Write\",\"cwd\":\"$MAIN_REPO\",\"tool_input\":{\"file_path\":\"$MAIN_REPO/notes-temp.md\"}}"

# c. NotebookEdit 主仓库 notebook (notebook_path 字段) → 拒绝
run_case "main-notebook-repo-file" deny \
  "{\"tool_name\":\"NotebookEdit\",\"cwd\":\"$MAIN_REPO\",\"tool_input\":{\"notebook_path\":\"$MAIN_REPO/Tests/nb.ipynb\"}}"

# d. Windows 反斜杠路径 → 拒绝（路径归一化）
run_case "main-edit-backslash-path" deny \
  "{\"tool_name\":\"Edit\",\"cwd\":\"${MAIN_REPO//\\//\\\\}\",\"tool_input\":{\"file_path\":\"${MAIN_REPO//\\//\\\\}\\\\Common\\\\HUtil32.pas\"}}"

# --- 放行场景 ---

# e. session 在 worktree 中编辑 worktree 文件 → 放行
run_case "worktree-edit-own-file" allow \
  "{\"tool_name\":\"Edit\",\"cwd\":\"$WORKTREE\",\"tool_input\":{\"file_path\":\"$WORKTREE/Common/HUtil32.pas\"}}"

# f. 主工作区 session 编辑 worktree 检出内的文件 → 放行（目标本身在 worktree 里）
run_case "main-edit-worktree-file" allow \
  "{\"tool_name\":\"Edit\",\"cwd\":\"$MAIN_REPO\",\"tool_input\":{\"file_path\":\"$WORKTREE/Common/HUtil32.pas\"}}"

# g. 仓库外文件 → 放行
run_case "main-edit-outside-file" allow \
  "{\"tool_name\":\"Write\",\"cwd\":\"$MAIN_REPO\",\"tool_input\":{\"file_path\":\"C:/Users/guisu/tmp/notes.md\"}}"

# h. 白名单：.claude/settings.local.json → 放行
run_case "main-edit-local-settings" allow \
  "{\"tool_name\":\"Edit\",\"cwd\":\"$MAIN_REPO\",\"tool_input\":{\"file_path\":\"$MAIN_REPO/.claude/settings.local.json\"}}"

# i. 无 file_path/notebook_path → 放行
run_case "no-file-path" allow \
  "{\"tool_name\":\"Edit\",\"cwd\":\"$MAIN_REPO\",\"tool_input\":{}}"

# j. cwd 不在 git 仓库内 → 放行
run_case "non-repo-cwd" allow \
  "{\"tool_name\":\"Edit\",\"cwd\":\"C:/Windows/Temp\",\"tool_input\":{\"file_path\":\"C:/Windows/Temp/x.txt\"}}"

echo "pass=$pass fail=$fail"
[ "$fail" -eq 0 ]
