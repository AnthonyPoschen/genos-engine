# Turn gate: the 3-bounce trace, not the SH path

The parked turn gate is near-camera error ≤ 0.403 against the oracle's path trace. On lavapipe at 160×90 that gate fails with `surface_cache.sh_texel_detail`, `sh_texel_ray_cut`, and `sh_probe_retire` on, and it fails the same way with them off. The miss is the hop count of the reference. It is not a scale error in the SH cells.

High's bounce count is unlimited. A cache ray adds the cell irradiance it hits, and the next relight reads that, so the picture is the equilibrium. The oracle's `reference` call passes `bounces: 3` (`examples/stress/scripts/lighting_oracle.rhai`). That cap is there so a frame finishes inside the 30 s trace budget. The cache-energy harness traces the same estimator at 64 bounces, and the closed box matches that reference.

## What was measured

Night hall, lamps on, sun frozen, eye `[12.5, 1.7, 20]`, yaw 0, pitch −0.05. The same pose as oracle turn frame 0. Flags on: SH cells, texel detail, texel ray cut, probe retire. Lavapipe, 160×90. Direct is the `direct` capture against a 0-bounce trace of that pose. The full picture is the live capture against traces of the same pose.

| Reference hops | Live energy / reference | Near error |
| --- | --- | --- |
| 1 | 4.82 | 4.82 |
| 3 (the oracle) | 2.02 | 1.25 |
| 8 | 1.08 | 0.22 |

Direct energy is 1.000. The 8-hop near error, 0.22, is under 0.403. Nothing in the engine changed between those three traces. A white surface with Lambert weight 0.8 (`SCENE_LAMBERT * π`) has an infinite sum about twice a 3-hop sum, which is the 2.02.

`diag[0]` / `diag[1]` on the settled frame were 146 covered pixels and 13380 probe evaluations. `lighting_sh_texel_detail_covers` is false once a fine texel holds its own samples, so compose still mixes the probes. The three flags move the turn picture by about 0.3 % of its energy. `cells_only`, which forces the SH read, is energy 2.24 against the same 3-hop trace. The cells are the same equilibrium. The closed box, graded at 64 hops, is bounce 1.0007 and blotch 0.011 with the flags on.

## What not to change

Do not multiply the cache, the probes, or the compose weight to land on the 3-hop picture. The 8-hop trace shows that light is in the scene. A darker cache would be a look hack, and it would miss the 64-hop closed-box gate that already passes.

Do not change the oracle's `bounces: 3` or the 0.403 number in this change. Both are frozen. Replacing the 3-hop pictures is a new reference, and Anthony has to accept that reference before a gate moves.

## The choice

Grade turn, slant, and spin against a trace at the cache depth (64, the same number `bounce_irradiance` uses in `examples/stress/tests/cache_energy.rs`), and keep 0.403 as the error limit on that picture. Or stop the cache at three hops and say so in the system record. The first keeps the physical solution the ADR asked for. The second makes the picture match a cost cap.

Coverage is a separate miss, and it is why the flags do not move the turn number. An axis-aligned texel with samples is not covered, so the ray cut has to keep those texels empty or the neighbour read never becomes the picture. On this hall it did not. That does not explain an error of 1.14 against a 3-hop trace: the forced cell picture is just as far from that trace.

Spin, same flags, near error 0.162 on and 0.161 off. The reference error does not show the cells falling behind the spinner. Probe retire did not take the probes off the turn view, so this run does not decide the mover-only helper.

## Networking

Replicated state is none. Authority is the machine that runs the trace. Prediction and rollback are none. The CPU trace is deterministic for a commit, a pose, a hop count, and a sample count. A headless server does not grade this.
