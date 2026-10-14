#!/usr/bin/env bash
# Workspace lint: clippy, cargo-deny, semgrep, and the env-mirror registry.
# The manifest is virtual, so clippy needs --workspace. `-- -D warnings`
# matches the workspace rust lint `warnings = "deny"`.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if [[ -d "${HOME}/.local/bin" ]]; then
  export PATH="${HOME}/.local/bin:${PATH}"
fi

python3 scripts/lint-env-mirrors.py --self-test
python3 scripts/lint-semgrep.py

cargo deny check
cargo clippy --workspace --all-targets -- -D warnings
