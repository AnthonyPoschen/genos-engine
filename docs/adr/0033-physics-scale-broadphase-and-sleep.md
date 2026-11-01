# 0033: Physics scale is a layered AABB tree plus island sleep

Status: **Accepted** (2026-11-01). Source: [physics plan](../systems/physics/plan.md) steps P4, P5, P6 and P12. Proposed on PR 130. This change is the implementation of that proposal.

## Context

`step` used to sort every bounded body by low X and sweep once per step. Statics were re-sorted every step, and every dynamic body was integrated even when it had not moved. Measured on the box CPU (8 cores, release, 1/60 s, mean of 120 steps after 120 warm-up steps), before this work:

| Scene | Bodies | µs / step |
|---|---|---|
| Resting spheres on a plane | 101 | 33 |
| Resting spheres on a plane | 401 | 155 |
| Resting spheres on a plane | 1 025 | 448 |
| 100×100 static boxes, one dynamic sphere | 10 001 | 9 235 |

A still world cost time linear in bodies. The plan's goal (an asleep world does no narrowphase and no solver work) is not reachable on that sweep.

## Decision

1. **P4 layers.** `ObjectLayer` + `BroadPhaseLayer` (0 static, 1 dynamic, 2 debris), a world pair table; 2–2 rejected. Defaults leave existing scenes unchanged.
2. **P5 trees.** One AABB tree for statics, one for dynamics (debris in the dynamic tree). Static-static pairs never emitted. Pair order stays `(min id, max id)`.
3. **P6 maintenance.** `insert_many` batch build, dirty-expand on move, synchronous `refit` before `step` returns.
4. **P12 islands and sleep.** Union-find over contacts each step. An island sleeps when every member stays under the sleep speeds for 0.5 s. Wake is whole-island (contact with an awake or displaced body, force, teleport, `wake(id)`). Asleep bodies are not integrated, not in narrowphase, not solved.

Settings (`sleep_linear`, `sleep_angular`, `sleep_time`, layer table) live on `World`. Per-step counters extend `StepStats` (`pair_tests`, `broadphase_builds`, `tree_nodes`, `islands`, `awake_bodies`, `solver_bodies`) and the `debug/physics` JSON line.

Targets on the box CPU: 10k statics + 1 dynamic under 200 µs; a fully asleep 1 024-body grid under 30 µs. Cloud-agent numbers are not that machine. The release print is `cargo test -p genos-physics --release --test scale_bench -- --nocapture`.

## Rejected

- **Uniform grid / spatial hash.** Cheap for equal sizes, poor for ships next to debris; the plan and Jolt use trees.
- **Sweep-and-prune kept with a persistent sorted list.** Still touches every static each step.
- **Per-body sleep.** A body sleeping under an awake neighbour jitters on wake; Box2D, Jolt and Bepu sleep islands. The rotational solver had already stored a per-body sleep bit. This step keeps that bit and gates it on the island.
- **Constraint coloring and threads (P22–P24) in the same slice.** Separate step after a profile.

## Consequences

- The existing tests (`scenarios.rs`, `colliders.rs`, `bench.rs`, CCD and destruction) stay green. Swept CCD pairs still use the direct bounds test in `ccd.rs`. A swept ship wakes an island when the discrete broadphase sees the contact after the sweep, not by querying the dynamic tree with the swept AABB.
- Networking: the sleep bit is replicated state (plan, replication table); island ids and tree nodes stay local. Determinism: tree build order is by minimum X then body id, so two runs match bitwise. Headless server runs the same path.
- Vehicles, IK hooks and the bond-graph destruction follow-ups are not decided here.

## Departures in the implementation

- The batch tree is a median split on minimum X (van den Bergen's AABB tree, without the surface-area rotations Box2D applies on insert). A single insert walks by surface area and does not rotate. `refit`, and any step with an awake dynamic or a displaced body, rebuilds the dynamic tree with the median split. A step where every dynamic body is asleep and nothing was displaced keeps the previous dynamic tree and does not query it. The static tree is left in place until a static body is removed or `refit` is called.
- Sleep is per body, 0.5 s under 0.12 m/s and 0.25 rad/s, same numbers as #155. The island id is who `wake` and `set_position` wake together. A contact above 0.25 m/s or 0.4 rad/s wakes only the sleeper it touches. Keeping every neighbour awake until the fastest member is calm reproduces sleep disabled. On a frictionless yawed pile that is 3.587 m/s at 4 s: the columns overlap, the normal impulse is horizontal, and friction 0 stores it as speed. Waking the whole island from that restless cube does it too. The resting-pile fixture uses friction 0.6 so Coulomb friction removes that speed and the pile stays under 0.5 m/s with sleep off. An aligned frictionless stack on the same solver stays at rest, so this is not a velocity clamp.
- The resting edge settles near 1.6 cm/s, under 0.1 m/s. The term that blocks a 0.1 rad/s sleep is angular: about every fifth step the block normal solver writes one frame near 0.169 rad/s and the next step returns it to zero. Linear on that frame stays near 0.04 m/s. Resting hull piles also sit between 0.10 and 0.12 m/s, so a 0.1 m/s gate never accumulates 0.5 s. Both residuals are inside 0.12 m/s and 0.25 rad/s. Replacing the block solve with sequential impulses removes the angular spike and the pile then sinks or runs away, so the block solve stays.
- Swept pairs are not read from the dynamic tree. The ship-lane scenes hold eight bodies, and the swept test needs both expanded bounds. The direct loop stays.
