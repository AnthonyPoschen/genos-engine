#!/usr/bin/env bash
# Install the pre-push hook that runs scripts/lint.sh.
# Safe to run again. An existing pre-push keeps its other commands.
# A symlink is left in place. A file with a NUL byte is left in place.
# scripts/hooks/pre-push is the hook the wrapper calls. Pointing
# core.hooksPath at scripts/hooks must not wrap that file, or the wrapper
# calls itself. A rejection is decided before any symlink is created.
# When the active hooks directory is a dispatcher that records
# .cursor-original-hooks-path and its other hooks are symlinks, the
# pre-push name becomes the same kind of symlink and the lint block is
# installed on that original path, which is the hook the dispatcher runs.
# A local hook is not enforcement. CI remains the authority.
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"

marker="# genos-engine: pre-push lint"
snippet=$(cat <<'EOF'
# genos-engine: pre-push lint
# Local hook only. CI remains the authority (docs/LINT_RULES.md).
repo_root=$(git rev-parse --show-toplevel) || exit 1
"$repo_root/scripts/hooks/pre-push" || exit $?
EOF
)

is_text() {
  python3 -c 'import sys; data=open(sys.argv[1],"rb").read(); raise SystemExit(0 if b"\0" not in data else 1)' "$1"
}

install_block() {
  local dest=$1
  local tmp first
  if [ -L "$dest" ]; then
    echo "pre-push: $dest is a symlink; not replacing it" >&2
    exit 1
  fi
  mkdir -p "$(dirname "$dest")"
  if [ -e "$dest" ]; then
    if ! is_text "$dest"; then
      echo "pre-push: $dest is not a text hook; not modifying it" >&2
      exit 1
    fi
    if grep -qF "$marker" "$dest"; then
      return 0
    fi
  fi
  tmp=$(mktemp)
  if [ ! -e "$dest" ]; then
    {
      printf '%s\n' '#!/bin/sh'
      printf '%s\n' "$snippet"
    } >"$tmp"
  else
    first=$(head -n 1 "$dest")
    case $first in
      '#!'*)
        {
          printf '%s\n' "$first"
          printf '%s\n' "$snippet"
          tail -n +2 "$dest"
        } >"$tmp"
        ;;
      *)
        {
          printf '%s\n' '#!/bin/sh'
          printf '%s\n' "$snippet"
          cat "$dest"
        } >"$tmp"
        ;;
    esac
  fi
  mv "$tmp" "$dest"
  chmod +x "$dest"
}

first_symlink() {
  local candidate base
  shopt -s nullglob
  for candidate in "$hook_dir"/*; do
    [ -L "$candidate" ] || continue
    base=$(basename "$candidate")
    [ "$base" = "pre-push" ] && continue
    readlink "$candidate"
    return 0
  done
  return 1
}

real_hook=$root/scripts/hooks/pre-push

canonical_path() {
  python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$1"
}

is_self_hook() {
  [ "$(canonical_path "$1")" = "$(canonical_path "$real_hook")" ]
}

# Reject before any install mutation. A symlink or a binary stays as it was.
reject_unless_safe_text() {
  local dest=$1
  if [ -L "$dest" ]; then
    echo "pre-push: $dest is a symlink; not replacing it" >&2
    return 1
  fi
  if [ -e "$dest" ] && ! is_text "$dest"; then
    echo "pre-push: $dest is not a text hook; not modifying it" >&2
    return 1
  fi
  return 0
}

hook_dir=$(git rev-parse --git-path hooks)
case $hook_dir in
  /*) ;;
  *) hook_dir=$root/$hook_dir ;;
esac
mkdir -p "$hook_dir"

original=""
if [ -f "$hook_dir/.cursor-original-hooks-path" ]; then
  original=$(tr -d '\r\n' <"$hook_dir/.cursor-original-hooks-path")
fi

target=$hook_dir/pre-push
if [ -L "$target" ]; then
  if [ -z "$original" ]; then
    echo "pre-push: $target is a symlink; not replacing it" >&2
    exit 1
  fi
  if is_self_hook "$original/pre-push"; then
    echo "pre-push: $original/pre-push is the lint hook; not wrapping it" >&2
    exit 0
  fi
  reject_unless_safe_text "$original/pre-push"
  install_block "$original/pre-push"
  exit 0
fi

if [ -e "$target" ]; then
  if is_self_hook "$target"; then
    echo "pre-push: $target is the lint hook; not wrapping it" >&2
    exit 0
  fi
  reject_unless_safe_text "$target"
  install_block "$target"
  exit 0
fi

sibling=""
if [ -n "$original" ]; then
  sibling=$(first_symlink || true)
fi
if [ -n "$original" ] && [ -n "$sibling" ]; then
  dest=$original/pre-push
  if is_self_hook "$dest"; then
    echo "pre-push: $dest is the lint hook; not wrapping it" >&2
    exit 0
  fi
  # Validate before creating the dispatcher symlink. A rejected original
  # hook must leave the hooks directory as it was.
  reject_unless_safe_text "$dest"
  ln -s "$sibling" "$target"
  install_block "$dest"
  exit 0
fi

if is_self_hook "$target"; then
  echo "pre-push: refusing to install a wrapper that calls itself" >&2
  exit 1
fi
reject_unless_safe_text "$target"
install_block "$target"
