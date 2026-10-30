# 0033: Physics scale is a layered AABB tree plus island sleep

Status: **Proposed** (2026-10-31). Needs Anthony's review before code. Source: [physics plan](../systems/physics/plan.md) steps P4, P5, P6 and P12. [ADR 0016](0016-physics-ccd-destruction.md) accepted only the CCD and destruction slice; the plan itself is unreviewed, so this ADR asks for the scale slice to be accepted.

## Context

`step` (`crates/physics/src/contact.rs`) sorts every bounded body by low X and sweeps once per step. Statics are re-sorted and re-swept every step, and every dynamic body is integrated and solved even when it has not moved. Measured on the box CPU (8 cores, release, 1/60 s, mean of 120 steps after 120 warm-up steps, scratch probe not committed):

| Scene | Bodies | µs / step |
|---|---|---|
| Resting spheres on a plane | 101 | 33 |
| Resting spheres on a plane | 401 | 155 |
| Resting spheres on a plane | 1 025 | 448 |
| 100×100 static boxes, one dynamic sphere | 10 001 | 9 235 |

A still world costs time linear in bodies, and 10k statics with one moving body costs more than half a 60 Hz frame. The plan's goal ("an asleep world does no narrowphase and no solver work", thousands of active bodies) is not reachable on the sweep.

## Decision

Implement in this order, one commit per step, each with the plan's headless test:

1. **P4 layers.** `ObjectLayer` + `BroadPhaseLayer` (0 static, 1 dynamic, 2 debris), a world pair table; 2–2 rejected. Defaults leave existing scenes unchanged.
2. **P5 trees.** One AABB tree for statics, one for dynamics (debris in the dynamic tree). Static-static pairs never emitted. Pair order stays `(min id, max id)` so the solver order and determinism do not change.
3. **P6 maintenance.** `insert_many` batch build, dirty-expand on move, synchronous `refit` before `step` returns.
4. **P12 islands and sleep.** Union-find over contacts each step. An island sleeps when every member stays under 0.1 m/s and 0.1 rad/s for 0.5 s. Wake is whole-island (contact with an awake body, force, teleport, `wake(id)`). Asleep bodies are not integrated, not in narrowphase, not solved.

Settings (`sleep_linear`, `sleep_angular`, `sleep_time`, layer table) live on the data-driven world settings; per-step counters extend `StepStats` (`pair_tests`, `broadphase_builds`, `tree_nodes`, `islands`, `awake_bodies`, `solver_bodies`) and are exposed through the shared stats helper and the `debug/physics` binary.

A deterministic benchmark (`crates/physics/tests/scale_bench.rs`, the four scenes above plus a 4k-body pile) prints bodies and µs/step before and after each step. Targets on the box CPU: 10k statics + 1 dynamic under 200 µs; a fully asleep 1 024-body grid under 30 µs.

## Rejected

- **Uniform grid / spatial hash.** Cheap for equal sizes, poor for ships next to debris; the plan and Jolt use trees.
- **Sweep-and-prune kept with a persistent sorted list.** Still touches every static each step.
- **Per-body sleep.** A body sleeping under an awake neighbour jitters on wake; Box2D, Jolt and Bepu sleep islands.
- **Constraint coloring and threads (P22–P24) in the same slice.** Separate step after a profile.

## Consequences

- The existing tests (`scenarios.rs`, `colliders.rs`, `bench.rs`, CCD and destruction) must stay bitwise green; swept CCD pairs query the dynamic tree with swept AABBs.
- A swept ship wakes any island its swept AABB touches.
- Networking: the sleep bit becomes replicated state (plan, replication table); island ids and tree nodes stay local. Determinism: tree build order is by body id, so two runs match bitwise. Headless server runs the same path.
- Vehicles, IK hooks and the bond-graph destruction follow-ups are not decided here.
