#!/usr/bin/env bash
# Run the presented-frame gates normally (they must pass), then again with
# each production guard knocked out (that gate must fail).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# binary:test:guard
tests=(
  "gate_presented:the_direct_light_is_in_the_presented_frame:${DIRECT_GUARD:-direct_in_compose}"
  "gate_presented:the_bounce_adds_light_to_the_presented_frame:${BOUNCE_GUARD:-bounce_in_compose}"
  "restir_moving_grain:a_moving_point_lamp_does_not_grain_a_flat_wall:${WALL_GUARD:-moving_lamp_wall}"
  "penumbra:the_penumbra_is_in_the_presented_frame:${DISC_GUARD:-lamp_disc}"
  "cone_stability:the_cone_cascades_drop_nothing_and_turn_back_to_the_same_frame:${CONE_CAP_GUARD:-cone_table_cap}"
  "cone_stability:a_turn_of_the_head_does_not_move_a_cone_probe:${CONE_SEAT_GUARD:-cone_field_seat}"
  "cone_stability:a_moved_lamp_lights_from_its_new_place_in_the_next_frame:${CONE_LAMP_GUARD:-cone_lamp_follow}"
)

# The knockout feature must stay test-only, and it is never a default.
# genos-render owns it. genos-stress forwards it so the fill gate
# (examples/stress/tests/fill_knockout.rs) can compile the guard out.
# That forward is not a default. No other manifest may name it.
if git grep -n knockout -- '*.toml' \
  ':!crates/render/Cargo.toml' \
  ':!examples/stress/Cargo.toml' | grep .; then
  echo "FAIL: a manifest other than genos-render or the stress fill forward names knockout" >&2
  exit 1
fi
if grep -nE '^default *=.*knockout' crates/render/Cargo.toml examples/stress/Cargo.toml; then
  echo "FAIL: knockout is a default feature" >&2
  exit 1
fi
if ! grep -q '^knockout = \["genos-render/knockout"\]$' examples/stress/Cargo.toml; then
  echo "FAIL: genos-stress knockout forward is not genos-render/knockout" >&2
  exit 1
fi

echo "==> presented-frame gates, guards on"
cargo test -p genos-render --test gate_presented -- --test-threads=1 --nocapture
cargo test -p genos-render --test restir_moving_grain -- --test-threads=1 --nocapture
cargo test -p genos-render --test penumbra the_penumbra_is_in_the_presented_frame -- --test-threads=1 --exact --nocapture
cargo test -p genos-render --test cone_stability -- --test-threads=1 --nocapture

for entry in "${tests[@]}"; do
  binary=${entry%%:*}
  rest=${entry#*:}
  name=${rest%%:*}
  guard=${rest##*:}
  echo "==> knockout ${guard} on ${name} (must go red)"
  log=$(mktemp)
  set +e
  GENOS_KNOCKOUT="$guard" cargo test -p genos-render --features knockout --test "$binary" \
    "$name" -- --test-threads=1 --exact 2>&1 | tee "$log"
  status=${PIPESTATUS[0]}
  set -e
  if [[ "$status" -eq 0 ]]; then
    # A passing test hides its output. Run it once more for the numbers it grades.
    GENOS_KNOCKOUT="$guard" cargo test -p genos-render --features knockout --test "$binary" \
      "$name" -- --test-threads=1 --exact --nocapture 2>&1 | grep -E "gate|presented|^test " || true
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
