.PHONY: run room stress stress-big stress-bench verify reference

run:
	cargo run -p genos-camera

room:
	cargo run -p genos-camera -- --scene room.rhai

stress:
	cargo run --release -p genos-stress

stress-big:
	cargo run --release -p genos-stress -- --scale big --lights 100 --dynamic 75

stress-bench:
	cargo run --release -p genos-stress -- --bench 10

# Headless smoke, sized for lavapipe. Reports land in target/debug-reports/<script>.
# GENOS_LIGHTING_SC_SETTLE is the still-frame count (the lighting also waits at
# least 32). smoke.rhai passes timeout_s so the command outlasts those frames,
# and it checks skip, UI scissor and verification invalidation.
GENOS_LIGHTING_SC_SETTLE ?= 32
verify:
	GENOS_LIGHTING_SC_SETTLE=$(GENOS_LIGHTING_SC_SETTLE) cargo run --release -p genos-stress -- --proof --no-panel --size 320x180 --script examples/stress/scripts/smoke.rhai

# Live lighting against the path-traced ground truth (minutes on first run, cached after).
reference:
	cargo run --release -p genos-stress -- --proof --no-panel --size 320x180 --script examples/stress/scripts/reference.rhai
