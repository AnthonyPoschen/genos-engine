# 0034: Contacts apply angular impulse at the contact point

Status: **Accepted** (implemented from the proposal in PR #139).

## Context

`solve_contact` used to be linear only. It took a normal and a depth, moved linear velocity, and wrote no torque. Bodies already carried `angular_velocity`. A box pushed past an edge slid off flat, and friction acted through the centre.

## Decision

1. **Contact carries a point.** Every narrow-phase pair builds a manifold of 1 to 4 world-space points with one normal and a depth. Sphere and capsule pairs use one point on the near side of the round surface. Box and hull pairs keep vertices that sit in the overlap, plus the centre projected onto the facing plane when that point lies in both shapes.
2. **Inertia is diagonal in body space for primitives.** World inverse inertia is `R · I⁻¹ · Rᵀ` applied to a vector. A box uses `Ixx = m/3 (hy² + hz²)`. A sphere uses `2/5 m r²`. A capsule uses the existing cylinder-and-hemispheres diagonal. A mesh stores the inverse of the unit-mass inertia from the pieces' volume (tetrahedra for a hull, analytic box plus parallel axis for a cuboid piece). Static and sleeping bodies use zero.
3. **Sequential impulses at the point.** Per point, `r = p − x`, relative velocity `v + ω × r`, effective mass `1 / (m⁻¹ₐ + m⁻¹ᵦ + (r×n)·I⁻¹(r×n) for each body)`. The accumulated normal impulse stays ≥ 0. The first of the four narrow-phase passes warm-starts from the previous step's points within 5 cm. Eight velocity iterations, fixed pair order, so the result is deterministic.
4. **Friction at the point.** Two tangent directions, a Coulomb box `|λt| ≤ μ λn`. Rolling and torsional friction are out of scope.
5. **Penetration recovery stays positional.** It moves the centres and does not change velocity, so it adds no kinetic energy. Restitution applies only above a 1 m/s closing speed, as a bias captured before the iterations.
6. **Gyroscopic term.** `ω` in the body frame takes the implicit Euler step (Catto 2015) before the orientation integrates.
7. **CCD stays linear in time of impact.** The time-of-impact contact feeds the same point solver.

## Departures from the proposal

- Friction uses the larger of the two coefficients, which is what `physics.md` already specified. The geometric mean would be zero against a default plane (friction 0) and would drop every existing ground-friction test.
- Position correction is still linear. A rotational split impulse launched resting stacks, so the penetration pass moves centres only. The face manifold (incident face clipped to the reference face, at most four points) keeps the normal forces on the support instead of on one corner.
- A symmetric face leaves a small residual spin under sequential impulses. The CCD contact runs 12 iterations so a head-on hull pair does not spin into the linear time-of-impact sweep. Discrete contacts stay at eight.
- Sleep is per body, not an island graph. The proposal left sleep to a later decision. A settled pile needs it: 0.5 s under 0.12 m/s and 0.25 rad/s, woken by a moving contact. Springs and motors do not sleep.

## Tests

- `a_box_past_the_edge_tips_and_one_over_the_ledge_stays`
- `a_spinning_box_on_the_floor_slows_down`
- `a_hull_pile_settles_and_sleeps_without_jitter`
- `mechanical_energy_does_not_grow_over_six_hundred_steps`
- `a_resting_stack_stays_put`
- `two_runs_of_the_overhang_match_bit_for_bit`
- `two_boxes_colliding_off_centre_exchange_angular_momentum`
- `a_yawed_hull_pile_sleeps_without_walking`
- `a_cube_hull_matches_the_box_moment`

## Consequences

`physics.md` records the torque, the sleep rule, and the probes. `step_stats.contact_points` and `step_stats.sleeping_bodies` are on the `genos-physics` debug step line. Joints, rolling friction, and a full island solver remain out of scope.
