.PHONY: run room stress stress-big stress-bench lighting verify reference lint-docs

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

# Lighting debug binary. SCENE picks debug/scenes/<name> (default stress).
# This is the Sponza benchmark: make lighting SCENE=sponza
SCENE ?= stress
lighting:
	cargo run --release -p genos-lighting -- --scene $(SCENE) --gpu-times 30

# Debug scripts headless; reports land in target/debug-reports/<script>.
verify:
	cargo run --release -p genos-stress -- --proof --no-panel --size 320x180 --script examples/stress/scripts/smoke.rhai

# Live lighting against the path-traced ground truth (minutes on first run, cached after).
reference:
	cargo run --release -p genos-stress -- --proof --no-panel --size 320x180 --script examples/stress/scripts/reference.rhai

# docs/systems/*/plan.md: a References section, and local links that resolve.
# docs/systems/core-networking/plan.md is allowlisted until that plan lands.
lint-docs:
	python3 scripts/lint-plan-docs.py --self-test
	python3 scripts/lint-plan-docs.py
