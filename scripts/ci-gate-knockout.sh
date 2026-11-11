#!/usr/bin/env bash
# Run the presented-frame gates normally (they must pass), then again with
# each production guard knocked out (that gate must fail).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

tests=(
  "the_direct_light_is_in_the_presented_frame:${DIRECT_GUARD:-direct_in_compose}"
  "the_bounce_adds_light_to_the_presented_frame:${BOUNCE_GUARD:-bounce_in_compose}"
)

echo "==> presented-frame gates, guards on"
cargo test -p genos-render --test gate_presented -- --test-threads=1 --nocapture

for entry in "${tests[@]}"; do
  name=${entry%%:*}
  guard=${entry##*:}
  echo "==> knockout ${guard} on ${name} (must go red)"
  set +e
  GENOS_KNOCKOUT="$guard" cargo test -p genos-render --features knockout --test gate_presented \
    "$name" -- --test-threads=1 --exact --nocapture
  status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    echo "FAIL: ${name} still passed with GENOS_KNOCKOUT=${guard}" >&2
    exit 1
  fi
  echo "PASS: ${name} went red under knockout ${guard} (cargo status ${status})"
done

echo "gate-knockout: all listed gates pass with guards on and fail under knockout"
