#!/usr/bin/env bash
# Exit 0 if the quit-gap job should run. Exit 1 if it can be skipped. The
# 100 ms budget is unchanged; this script only decides whether to run the job.
#
# The job builds and quits genos-camera, genos-stress and genos-lighting and
# runs the genos-window lifecycle_quit test. Anything those four packages
# compile can change the quit path (the scene event loop, the input and mcp
# threads, the renderer teardown), so the skip list is the complement of
# their workspace dependency closure, read from the Cargo.toml files of the
# checked-out tree. A path this script does not recognise runs the job.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: ci-quit-gap-needed.sh BASE HEAD" >&2
  exit 2
fi

base=$1
head=$2

if ! git cat-file -e "${base}^{commit}" 2>/dev/null; then
  echo "ci-quit-gap-needed: cannot see ${base}; running quit-gap" >&2
  exit 0
fi
if ! git cat-file -e "${head}^{commit}" 2>/dev/null; then
  echo "ci-quit-gap-needed: cannot see ${head}; running quit-gap" >&2
  exit 0
fi

changed=$(git diff --name-only "$base" "$head")
if [[ -z "$changed" ]]; then
  echo "ci-quit-gap-needed: no changed files in ${base}..${head}; skip"
  exit 1
fi

# A failure of the classifier itself must run the job.
# The script comes from the heredoc, so the file list goes in the environment.
if verdict=$(CHANGED="$changed" python3 - <<'PY'
import glob
import os
import sys
import tomllib

ROOTS = {"genos-window", "genos-camera", "genos-stress", "genos-lighting"}
# Never reach the windowed binaries: prose, assets, lint config, tooling.
INERT_PREFIXES = ("docs/", "assets/", "lint/", ".cursor/", "tools/")
INERT_SUFFIXES = (".md",)
INERT_FILES = {"LICENSE", ".gitignore", ".gitattributes"}
# Always matter: the dependency graph, toolchain, and this job itself.
ALWAYS = {
    "Cargo.toml", "Cargo.lock", "clippy.toml", "deny.toml", "Makefile",
    ".github/workflows/lint.yml", "scripts/ci-apt.sh",
    "scripts/ci-quit-gap-needed.sh",
}

members = {}
for manifest in glob.glob("**/Cargo.toml", recursive=True):
    if manifest.startswith(("target/", "tools/")) or manifest == "Cargo.toml":
        continue
    with open(manifest, "rb") as handle:
        doc = tomllib.load(handle)
    pkg = doc.get("package", {}).get("name")
    if pkg:
        members[pkg] = (os.path.dirname(manifest), doc)

def deps_of(doc):
    tables = [doc]
    tables.extend(doc.get("target", {}).values())
    for table in tables:
        for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
            yield from table.get(kind, {}).items()

closure_dirs = set()
todo = [name for name in ROOTS if name in members]
missing = ROOTS - set(todo)
if missing:
    print(f"run: root package missing: {sorted(missing)}")
    sys.exit(0)
seen = set()
while todo:
    name = todo.pop()
    if name in seen:
        continue
    seen.add(name)
    directory, doc = members[name]
    closure_dirs.add(directory)
    for dep_name, spec in deps_of(doc):
        if isinstance(spec, dict) and "path" in spec:
            target = os.path.normpath(os.path.join(directory, spec["path"]))
            for other, (other_dir, _) in members.items():
                if other_dir == target and other not in seen:
                    todo.append(other)

paths = [p.strip() for p in os.environ.get("CHANGED", "").splitlines() if p.strip()]
if not paths:
    print("run: classifier saw no paths")
    sys.exit(0)
for path in paths:
    if path in ALWAYS:
        print(f"run: {path} (graph, toolchain or this job)")
        sys.exit(0)
    if any(path == d or path.startswith(d + "/") for d in closure_dirs):
        print(f"run: {path} (in the quit-gap binaries' dependency closure)")
        sys.exit(0)
    if path.startswith(INERT_PREFIXES) or path.endswith(INERT_SUFFIXES) or path in INERT_FILES:
        continue
    if any(path.startswith(d + "/") for d, _ in members.values()):
        continue  # a workspace crate outside the closure
    if path.startswith(".github/workflows/") and path != ".github/workflows/lint.yml":
        continue
    if path.startswith("scripts/"):
        continue
    print(f"run: {path} (unrecognised path)")
    sys.exit(0)
print("skip")
PY
); then
  :
else
  echo "ci-quit-gap-needed: classifier failed; running quit-gap" >&2
  exit 0
fi

if [[ "$verdict" == skip ]]; then
  echo "ci-quit-gap-needed: nothing in ${base}..${head} reaches the windowed binaries; skip"
  exit 1
fi
echo "ci-quit-gap-needed: ${verdict}"
exit 0
