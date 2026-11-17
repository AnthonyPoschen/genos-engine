#!/usr/bin/env bash
# Fetch pull-request objects for the baseline-growth gate.
#
# The workflow checks out github.sha (the pull_request_target default-branch
# commit) and then runs `fetch`, then `guard`, then the gate script. This
# file does not check out another commit and does not fetch a URL from the
# event. A bad ref, a missing gate script, or a fetched head that is not the
# event head fails closed.
set -euo pipefail

# A caller with GIT_DIR pointed at another repository would make `git`
# operate on that repository while the shell still runs this file. Stay on
# the worktree that was checked out.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_NAMESPACE GIT_PREFIX

fail() {
  echo "baseline-growth: $1. Fail closed." >&2
  exit 2
}

lower() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

sha_ok() {
  local re='^[0-9a-fA-F]{40}([0-9a-fA-F]{24})?$'
  [[ "$1" =~ $re ]]
}

pr_ok() {
  local re='^[1-9][0-9]{0,9}$'
  [[ "$1" =~ $re ]]
}

# Branch names only. Reject refspec destinations, options, and traversal
# before the value is placed in a fetch argument.
ref_ok() {
  local name=$1
  local re='^[A-Za-z0-9][A-Za-z0-9._+/-]*$'
  if [[ ! "$name" =~ $re ]]; then
    return 1
  fi
  if [[ "$name" == *".."* || "$name" == *"//"* || "$name" == *"@"* || "$name" == *".lock" || "$name" == */ || "$name" == refs/* ]]; then
    return 1
  fi
  return 0
}

require_syntax() {
  local mode=$1
  sha_ok "${TRUSTED_SHA:-}" || fail "trusted sha is not a commit sha"
  sha_ok "${BASE_SHA:-}" || fail "base sha is not a commit sha"
  sha_ok "${HEAD_SHA:-}" || fail "head sha is not a commit sha"
  pr_ok "${PR_NUMBER:-}" || fail "pull number is not an integer"
  if [ "$mode" = "fetch" ]; then
    ref_ok "${BASE_REF:-}" || fail "base ref is not a branch name"
  fi
}

# The gate script that will run is the one in this worktree. It has to be a
# regular file inside the trusted checkout, not a symlink to somewhere else.
require_trusted_worktree() {
  local worktree root script
  worktree=$(git rev-parse HEAD) || fail "cannot read the worktree sha"
  if [ "$(lower "$worktree")" != "$(lower "$TRUSTED_SHA")" ]; then
    fail "worktree is ${worktree}, trusted sha is ${TRUSTED_SHA}"
  fi
  root=$(git rev-parse --show-toplevel) || fail "cannot find the worktree"
  cd "$root"
  script=$root/scripts/lint-baseline-growth.py
  if [ ! -f "$script" ]; then
    fail "trusted checkout has no gate script"
  fi
  python3 -c 'import os, sys
root, path = sys.argv[1:]
root = os.path.realpath(root)
real = os.path.realpath(path)
if os.path.commonpath([root, real]) != root:
    raise SystemExit(1)
' "$root" "$script" || fail "gate script resolves outside the worktree"
  if [ ! -f "$root/scripts/lint-baseline-growth-ci.sh" ]; then
    fail "trusted checkout has no fetch guard"
  fi
  if [ -n "$(git status --porcelain)" ]; then
    fail "worktree is dirty"
  fi
}

fetch_origin() {
  local spec=$1
  local what=$2
  # `--` keeps a value from being read as a fetch option. The refspec has no
  # destination, so this updates FETCH_HEAD and does not move a local branch.
  git fetch --no-tags --no-recurse-submodules origin -- "$spec" || fail "cannot fetch ${what}"
}

object_type() {
  local sha=$1
  local kind
  kind=$(git cat-file -t "$sha" 2>/dev/null) || fail "$2 is not a commit"
  if [ "$kind" != "commit" ]; then
    fail "$2 is not a commit"
  fi
}

cmd=${1:-}
case "$cmd" in
  fetch)
    require_syntax fetch
    require_trusted_worktree
    git remote get-url origin >/dev/null 2>&1 || fail "origin is missing"
    before=$(git rev-parse HEAD)
    # The pull head is last so FETCH_HEAD is the event head, not the base.
    fetch_origin "$BASE_SHA" "base sha"
    fetch_origin "refs/heads/${BASE_REF}" "base ref"
    fetch_origin "refs/pull/${PR_NUMBER}/head" "pull/${PR_NUMBER}/head"
    fetched=$(git rev-parse FETCH_HEAD) || fail "fetch did not record FETCH_HEAD"
    if [ "$(lower "$fetched")" != "$(lower "$HEAD_SHA")" ]; then
      fail "fetched pull/${PR_NUMBER}/head is ${fetched}, event head is ${HEAD_SHA}"
    fi
    object_type "$BASE_SHA" "base sha"
    object_type "$HEAD_SHA" "head sha"
    after=$(git rev-parse HEAD)
    if [ "$after" != "$before" ]; then
      fail "fetch moved the worktree to ${after}"
    fi
    require_trusted_worktree
    ;;
  guard)
    require_syntax guard
    require_trusted_worktree
    ;;
  *)
    fail "usage: lint-baseline-growth-ci.sh fetch|guard"
    ;;
esac
