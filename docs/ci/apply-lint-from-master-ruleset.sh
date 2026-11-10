#!/usr/bin/env bash
# Apply or update the lint-from-master repository ruleset.
#
# Do not run this with enforcement: active until
# .github/workflows/lint-from-master.yml is on refs/heads/master. A ruleset
# pointing at a missing file would block merges on every branch (~ALL).
#
# After the first lint-from-master check is green on a PR, keep the existing
# branch-protection "lint" context until you are ready to switch. The command
# to add the new context (and only later drop lint) is in
# docs/ci/lint-from-master.md.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
payload=$root/docs/ci/ruleset-lint-from-master.json
repo=AnthonyPoschen/genos-engine

if ! command -v gh >/dev/null; then
  echo "apply-lint-from-master-ruleset: gh is required" >&2
  exit 2
fi

existing_id=$(gh api "repos/${repo}/rulesets" --jq '.[] | select(.name=="lint-from-master") | .id' | head -n1)
if [[ -n "${existing_id}" ]]; then
  echo "updating ruleset ${existing_id}"
  gh api --method PUT "repos/${repo}/rulesets/${existing_id}" --input "$payload"
else
  echo "creating ruleset lint-from-master"
  gh api --method POST "repos/${repo}/rulesets" --input "$payload"
fi
