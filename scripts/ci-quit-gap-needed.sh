#!/usr/bin/env bash
# Exit 0 if the quit-gap job should run (a window or lifecycle path changed
# between BASE and HEAD). Exit 1 if it can be skipped. The 100 ms budget is
# unchanged; this script only decides whether to run the job.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: ci-quit-gap-needed.sh BASE HEAD" >&2
  exit 2
fi

base=$1
head=$2

# Paths that can change the quit-gap timing or the job itself.
# #253 touched crates/window, the renderer teardown in gpu.rs, and the
# three windowed binaries the job builds.
paths=(
  crates/window
  crates/render/src/gpu.rs
  crates/render/src/gpu
  debug/lighting/src/main.rs
  examples/camera/src/main.rs
  examples/stress/src/main.rs
  .github/workflows/lint.yml
  scripts/ci-apt.sh
  scripts/ci-quit-gap-needed.sh
)

if ! git cat-file -e "${base}^{commit}" 2>/dev/null; then
  echo "ci-quit-gap-needed: cannot see ${base}; running quit-gap" >&2
  exit 0
fi
if ! git cat-file -e "${head}^{commit}" 2>/dev/null; then
  echo "ci-quit-gap-needed: cannot see ${head}; running quit-gap" >&2
  exit 0
fi

changed=$(git diff --name-only "$base" "$head" -- "${paths[@]}")
if [[ -n "$changed" ]]; then
  printf '%s\n' "$changed"
  exit 0
fi
echo "ci-quit-gap-needed: no window/lifecycle paths in ${base}..${head}; skip"
exit 1
