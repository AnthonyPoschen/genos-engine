# 0016: Ships use time of impact, and destruction is a pre-fractured bond graph

Status: **proposed**. Design: [`docs/systems/physics-ccd-destruction-design.md`](../systems/physics-ccd-destruction-design.md).

The source is the physics plan of 12 Oct 2026 ([`docs/systems/physics/plan.md`](../systems/physics/plan.md)). This ADR decides the continuous-collision and destruction slice of that plan. It does not change `step`.

## Context

`step` is discrete. A body integrates `velocity * dt` and then resolves the overlap it already has. Two ships closing at 3 000 m/s travel 50 m each in one `1/60` s step, which is longer than the hull, so they exchange places with no contact. A mesh collider is at most six boxes or one 12-vertex hull. It cannot break.

Release multiplayer is server-authoritative. Both peers have to agree on the impact and on which chunks exist. [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md) keeps the step in this repository. [ADR 0013](0013-lighting-invalidation-is-local.md) invalidates lighting from local change records, with a per-preset work cap and no full-frame rebuild.

## Decision

1. The solver stays `genos-physics`. Rapier and Jolt are references. They are not dependencies.
2. Motion quality is per body. `Discrete` is the default. `Speculative` grows the contact distance by `min(speed * dt, speculative margin)` and lets the solver stop the body. `Swept` solves a time of impact for that pair by conservative advancement, including both linear and angular velocity, and sub-steps that pair only.
3. A ship lane is `Swept`. A modest bullet is `Speculative`. The ship scene is four head-on lanes at 30, 120, 600, and 3 000 m/s.
4. Destruction is an offline Voronoi pattern in a `.gfract` asset: a chunk hierarchy, interior faces, and a bond graph. The step hot-swaps a broken chunk for its child bodies. One `apply_damage` breaks a bond, whether the amount came from a contact impulse or from a game event such as health reaching 0.
5. A spawned chunk is a mover. The client emits one [change record](../lighting/GLOSSARY.md) per spawned, moved, slept, or culled chunk. Cards and SDFs are cooked per chunk. The break does not rebake.

## Rejected

- **Rapier** as the solver. It already has soft CCD and nonlinear CCD, and it is Rust, but it brings Parry and nalgebra and would own `step`. [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md) forbids that.
- **Jolt bindings.** `EMotionQuality::LinearCast` is a shipped sweep, and the body id and the layer split are the shapes the physics plan already copies. A binding would put the step in a C++ library. Linear cast also rotates first and then sweeps, so a fast spin can still tunnel.
- **Speculative contacts as the only CCD.** PhysX and Bepu stop modest speeds this way. At ship speed the margin is tens of metres, and the contact fires in empty space.
- **Time-of-impact sub-steps of the whole world.** Box2D advances every body to the first impact. One ship pair would re-simulate the room.
- **Runtime fracture from the impact point** (Müller, Chentanez, and Kim, SIGGRAPH 2013). The pattern would depend on the hit, so a content hash of one asset would not name the chunks.
- **Finite-element fracture.** The server tick cannot carry a deforming mesh, and the contact code tests convex pieces.

## Consequences

The metrics and the phase order are in the design. Until the swept step and the bond graph exist, the metrics harness reports `NOT-IMPLEMENTED` and does not invent a reading. The physics system record changes in the commit that changes `step`, not in this decision.
