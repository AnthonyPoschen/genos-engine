# 0034: Contacts apply angular impulse at the contact point

Status: **Proposed** (Engine bot, 2026-10-31). Needs Anthony Poschen's approval before any code lands.

## Context

`solve_contact(a, b, normal, penetration)` in `crates/physics/src/contact.rs` is linear only. It takes a normal and a depth, moves linear velocity, and writes no torque ([physics.md](../systems/physics.md): "Contact does not write a torque"). Bodies carry `angular_velocity` and integrate orientation from it, and `rotational_energy` uses a diagonal inertia (`inertia_diagonal`: box `m/3 (h²+h²)`, sphere `2/5 m r²`, mesh = bounding-sphere moment). The [physics plan](../systems/physics/plan.md) lists no inertia tensor. ADR 0016 (CCD and destruction) leaves `step` unchanged. No accepted ADR or design covers rotation from contact.

So a box pushed over an edge slides off flat instead of tipping, a struck ship never yaws, and an edge contact (`hull_hull_contact` already returns a `point`, ADR 0016 work and PR #136) has no effect beyond the normal push. Friction acts on the centre, not where the surfaces touch.

## Decision

1. **Contact carries a point.** Every narrow-phase pair returns a manifold of 1 to 4 world-space points with one normal and a depth per point. Sphere and capsule pairs return one point; box/hull face contacts clip to at most 4. `HullHit.point` is the 1-point case.
2. **Inertia is a tensor in body space.** Each shape stores `inverse_inertia_local: Mat3` (diagonal for box, sphere, capsule; for hulls, the volume integral of the hull, computed once at `Shape::from_triangles`). Static and kinematic bodies have zero. World inverse inertia is `R · I⁻¹ · Rᵀ` per step.
3. **Sequential impulses at the point.** Per point, `r = p − x`, relative velocity `v + ω × r`, effective mass `1 / (m⁻¹ₐ + m⁻¹ᵦ + n·((Iₐ⁻¹(rₐ×n))×rₐ + (Iᵦ⁻¹(rᵦ×n))×rᵦ))`. The accumulated normal impulse is clamped at ≥ 0 and warm-started from last step's matched points. Fixed iteration count (8 default, a tunable), fixed pair order (body index), so the result is bitwise deterministic.
4. **Friction at the point.** Two tangent directions per point, a Coulomb box clamp `|λt| ≤ μ λn` with `μ = sqrt(μa μb)`. Rolling and torsional friction are out of scope.
5. **Penetration recovery stays positional** (split impulse / pseudo-velocity), so position correction adds no kinetic energy. Restitution applies only above a 1 m/s closing threshold.
6. **Gyroscopic term.** Integrate `ω` with the implicit gyroscopic step (Catto 2015) so a free spinning body does not gain energy.
7. **CCD stays linear in time of impact.** ADR 0016 conservative advancement already bounds rotation by `|ω| · radius`; the TOI contact feeds the same point solver.

## Tests that gate it

- Box tipping off an edge: a cube pushed past a ledge until its centre is beyond the edge rotates and falls; one whose centre stays over the ledge stays put.
- Spinning top: a body spinning about its major axis on a plane holds its axis within 5° for 10 s; spin about the intermediate axis flips (Dzhanibekov) rather than stalling.
- Energy never grows: over 600 steps of a resting pile, a bouncing box (restitution 0) and the free spinner, total kinetic + potential energy is non-increasing within 1e-4 relative.
- Determinism: two runs hash bit-identical every step (the `debug/physics` `--replay` probe and a unit test).
- Regression: ship lanes, hull edges and resting pile tests keep passing unchanged in their asserted numbers.

## Consequences

- `physics.md` drops "Contact does not write a torque" and gains failure modes and probes: per-point manifold dump, effective mass, accumulated impulses, and energy per step in `debug/physics`.
- `solve_contact` signature changes; destruction's chunk-inheritance velocity code must pass `angular_velocity` through (it already stores it).
- Cost: up to 4 points × 3 rows per pair, 8 iterations. The resting pile and 10k static grid timings in `debug/physics` are the budget check; regressions over 2× need a note.
- Not decided here: joints, rolling friction, islands and sleep (ADR 0033), soft contact.
