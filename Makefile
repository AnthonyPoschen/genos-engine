.PHONY: run room stress stress-big stress-bench lighting verify reference lighting-ref-check lighting-oracle restir-ref lint-docs lint

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

# Pictures against the path trace. Reports land in ./genos-debug/refcheck/<timestamp>/.
# MODE=owner (default) or MODE=lavapipe-small. BASE=<sha> reads that commit's metrics.
# OUT=<dir> overrides the report directory.
lighting-ref-check:
	cargo run --release -p genos-stress -- lighting-ref check --mode $(if $(MODE),$(MODE),owner) $(if $(BASE),--base $(BASE),) $(if $(OUT),--out $(OUT),)

# Lighting harness and oracle (docs/systems/lighting/oracle-harness.md).
# PROFILE=quick (default), full, or lamps (many-lamp spin and switch). OUT=<dir> overrides ./genos-debug/oracle.
lighting-oracle:
	cargo build --release -p genos-stress
	scripts/lighting/oracle_run.sh $(if $(PROFILE),$(PROFILE),quick) $(if $(OUT),$(OUT),genos-debug/oracle)

# CPU ReSTIR DI reference (docs/systems/lighting/restir-di-plan.md).
# Writes ./genos-debug/restir/metrics.json and reference.json.
restir-ref:
	cargo run -p genos-restir-ref --bin restir-ref -- run --out genos-debug/restir

# docs/systems/*/plan.md: a References section, and local links that resolve.
# docs/systems/core-networking/plan.md is allowlisted until that plan lands.
lint-docs:
	python3 scripts/lint-plan-docs.py --self-test
	python3 scripts/lint-plan-docs.py

# Clippy, cargo-deny, semgrep, and the env-mirror registry.
lint:
	./scripts/lint.sh
