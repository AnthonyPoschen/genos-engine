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

# The knockout feature must stay test-only: no manifest but genos-render's
# own may name it, and it is never a default feature.
if git grep -n knockout -- '*.toml' ':!crates/render/Cargo.toml' | grep .; then
  echo "FAIL: a manifest outside crates/render enables the knockout feature" >&2
  exit 1
fi
if grep -nE '^default *=.*knockout' crates/render/Cargo.toml; then
  echo "FAIL: knockout is a default feature of genos-render" >&2
  exit 1
fi

echo "==> presented-frame gates, guards on"
cargo test -p genos-render --test gate_presented -- --test-threads=1 --nocapture

for entry in "${tests[@]}"; do
  name=${entry%%:*}
  guard=${entry##*:}
  echo "==> knockout ${guard} on ${name} (must go red)"
  log=$(mktemp)
  set +e
  GENOS_KNOCKOUT="$guard" cargo test -p genos-render --features knockout --test gate_presented \
    "$name" -- --test-threads=1 --exact 2>&1 | tee "$log"
  status=${PIPESTATUS[0]}
  set -e
  if [[ "$status" -eq 0 ]]; then
    echo "FAIL: ${name} still passed with GENOS_KNOCKOUT=${guard}" >&2
    exit 1
  fi
  # A build error or a missing test also exits non-zero; only the gate's own
  # assertion failing counts as red.
  if ! grep -qE "^test ${name} \.\.\. FAILED" "$log" || ! grep -q "(guard ${guard})" "$log"; then
    echo "FAIL: ${name} exited ${status} under knockout ${guard}, but not from its own assertion" >&2
    exit 1
  fi
  rm -f "$log"
  echo "PASS: ${name} went red under knockout ${guard} (cargo status ${status})"
done

echo "gate-knockout: all listed gates pass with guards on and fail under knockout"
