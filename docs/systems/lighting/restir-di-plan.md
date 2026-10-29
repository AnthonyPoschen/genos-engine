# ReSTIR DI — stage C

[ADR 0022](../../adr/0022-lighting-v3-compute-architecture.md) stage C replaces brute direct with ReSTIR DI. This note is the CPU reference and the GPU stages that follow it. No shader is in this change. The renderer is unchanged.

## Reference

`genos-restir-ref` (binary `restir-ref`) is the checker. It does not open a window or a Vulkan device, and it does not depend on `genos-render`. The pairwise weight is the same threshold as `lamp_pick::pairwise_merge`, copied so the crate can build without a device.

Initial candidates are resampled importance sampling (Talbot, Cline, and Egbert, EGSR 2005). Temporal and spatial reuse follow Bitterli et al., TOG 2020. The weight is the balance heuristic (Bitterli et al., SIGGRAPH 2022, GRIS equation 22). Two reservoirs use the shader's history test. A longer list uses one prefix-sum pick, then

`W = area * p̂_src / (p̂_q(y) * Σ_i M_i * p̂_i(y))`

with `area = Σ M_i * W_i * p̂_q(y_i)`. The shade is unshadowed radiance times visibility times `W`. `p̂` is Rec. 709 luminance of the unshadowed Lambertian radiance. Visibility is not in `p̂`. One shadow ray runs at shade time.

`make restir-ref` writes `./genos-debug/restir/metrics.json` and `reference.json`. `--out` overrides that directory. `GENOS_RESTIR_OUT` applies only when `--out` was omitted. `restir-ref diff --reference FILE --candidate FILE` writes `diff.json` for a later GPU dump in the same schema.

## Departures

Named in `crates/restir-ref/src/lib.rs` and repeated here.

- Proposal is uniform or an exact discrete target CDF, not a light tree.
- `p̂` is unshadowed Rec. 709 luminance. One shadow ray multiplies the shade.
- Point lamps and a Lambertian BRDF only. No sun, no BSDF sample.
- Receivers are a grid. Temporal history is the same index, not a motion vector.
- Occluders are boxes, not mesh SDFs.
- One spatial pass. History is the post-temporal reservoir, not the spatial one, so a neighbour's samples are not counted again next frame. `History::Final` stores the shaded reservoir instead.
- History `M` is clamped to `m_clamp` times the current frame's candidate count before the temporal merge. The default multiple is 20 (Bitterli et al., TOG 2020). `m_clamp = 0` leaves `M` unclamped. The merge output can sit one candidate above the cap, because the clamp runs on the history input. `--m-clamp` sets the multiple. It is not the GPU fault `--m-cap`, which clamps history `M` to an absolute count.
- An empty canonical sample returns the history record unchanged, matching `pairwise_merge`.
- The four CPU gates run on an unoccluded grid. `Scene::shadow` adds one box and is the bias check only.

Bitterli et al. 2020 section 3 (divide by the sum of `M`, ignore the source target) is implemented as `Weighting::Section3` so a dump can name it. It is not the gated weight. On one hand-built pair the balance estimate is 5 and the section-3 estimate is 2. Algorithm 6 shaded with `point_total / phat` stays out; that path is already gone from the live shader.

## What the CPU gates mean

The product gates stay `make lighting-oracle` and a 4070 timestamp. The CPU names are the same four keys, on a receiver grid.

| Key | CPU stand-in | Ceiling |
| --- | --- | --- |
| `ref_err_near` | Near-weighted relative Rec. 709 error of the multi-seed mean against the brute sum. Weight is `1 / max(distance, 0.5 m)` from the scene camera. | 0.05 |
| `grid_worst` | Checkerboard score from `scripts/lighting/grid_metric.py`, tile 1. A frame step under 0.002 is skipped. | 0.15 |
| `frames_to_ref` | First 1-based frame whose error stays within 1.1× the last frame. | 8 |
| `frame_ms` | One `Session::step` after a warmup, debug build. Not the 3 ms GPU budget. | 50 |

A target proposal with no occlusion makes each receiver's luminance the integral on every sample, so the picture holds still and `grid_worst` is 0. A fresh spatial sample moves the multi-seed mean by enough to score as a checkerboard (measured 1.09 with 4 neighbours, 64 seeds, 8 frames, while `ref_err_near` was 0.047). The gated config therefore leaves spatial off. Spatial reuse stays in the open-grid test. On the GPU one path locks and goes still, which is why the product `grid_worst` is still the oracle PNG metric.

Visibility is one sample per chain. Temporal `M` makes the weight exact and does not reduce the variance of a shadowed integral. That variance falls across seeds. The walled scene is not the checkerboard gate.

## Measured (debug, this crate's tests)

Gate scene, 8×8, 12 lamps, no occluder, target proposal, 8 candidates, temporal, spatial off, balance, 64 seeds, 8 frames:

- `ref_err_near` 2.4e-7
- `grid_worst` 0
- `frames_to_ref` 2, because frame 1 is 2.6e-7 and the last frame is 2.4e-7, just outside the 1.1× band. The picture is the integral from the first frame.
- `frame_ms` 0.20 (debug). Ceiling 50. Not a 4070 number.

`Scene::shadow`, same config, 128 seeds, 4 frames: `ref_err_near` 0.029 (frames 0.034, 0.027, 0.028, 0.029).

Other locks, same run:

- One receiver, target proposal, 6 frames: luminance error 6.9e-8.
- Eight coloured lamps, 96 seeds, 4 frames: channel relative error 0.050, 0.014, 0.063. RGB is unbiased in expectation. Luminance is exact per sample.
- Uniform proposal, 1 candidate, no spatial, 64 seeds: variance 0.361 at frame 1 and 0.015 after 24 frames (ratio 0.042).
- Open 4×4, uniform, 2 candidates, 3 spatial neighbours, temporal, 36 seeds, 10 frames: worst relative luminance 0.052.
- Two receivers, one candidate, one disagreeing neighbour, 2000 seeds: balance relative error 0.040.

## Probes

| Failure | Probe |
| --- | --- |
| Biased weight | `ref_err_near` in `metrics.json`, and `restir-ref diff` |
| Reuse not reducing variance | `variance_falls_with_temporal_reuse` (ratio above) |
| Temporal confidence surviving a change | `Session::reset_temporal`, then `history_m` (the test grows `M` past 2, then the next frame is 2) |
| A moved lamp staying stale because history `M` grew without a cap | `a_moved_lamp_stops_being_stale_inside_the_m_clamp`. Unclamped `M` stays above 40. After the near lamp moves away, the 20× cap is within 0.15 relative error by frame 24, and the unclamped shade is still above 0.5 at that frame |
| Visibility leak | `a_wall_blocks_one_lamp_and_not_the_other`, and the shadow `ref_err_near` |
| Tunables | CLI flags, echoed in the dump `config` |
| Snapshot | `reference.json`. `diff` is the restore check |

No MCP entry. The checker is offline, the same way `reflections-check` is. `debug/lighting` is unchanged because the renderer is unchanged.

## Networking

Replicated state: none. The reservoirs stay in the checker.
Authority: the checker process.
Prediction / rollback: none.
Determinism: one PCG32 stream and the draw order in `crates/restir-ref/src/restir.rs`.
Headless server: the checker is headless. A game server does not run it.

## GPU stages

Each stage is a small diff against this reference. Lavapipe is the device until C5. Do not book the 4070 between C1 and C4. One batched owner run covers C5.

| Stage | What | Check |
| --- | --- | --- |
| C0 | This CPU reference | `cargo test -p genos-restir-ref`. `make restir-ref` |
| C1 | Initial candidates only. Same PCG stream and the documented draw order. `restir-gpu --initial` (off unless the flag or `GENOS_RESTIR_INITIAL` is set) | `restir-ref run --no-temporal --spatial 0 --frames 1`, then `restir-ref diff` on the reservoirs. Lavapipe. A perturbed seed must fail |
| C2 | Temporal reuse. A change record clears history, as `reset_temporal` does. `--temporal` is off unless the flag or `GENOS_RESTIR_TEMPORAL` is set. History `M` is clamped to `m_clamp` times this frame's candidate count (default 20). `--m-cap` is still the absolute-count fault. | Diff the reservoirs after N frames against `restir-ref run --spatial 0`. A reset must drop `M` the way the CPU test does. `--m-cap 1` must fail the diff. A run where the 20× cap does not fire (8 candidates, 8 frames) stays an exact match |
| C3 | One spatial pass and the balance weight. `restir-gpu --spatial N` (off unless the flag or `GENOS_RESTIR_SPATIAL` is set; `N` defaults to 4, radius defaults to 2). History stays post-temporal. `--reverse-neighbours` is the order fault | `restir-ref run --spatial 4`. Diff reservoirs at 1 and 8 frames (lamp mismatches 0, `W` relative error at most 1e-3). Do not treat the CPU multi-seed `grid_worst` of that run as the product gate. `--reverse-neighbours` must fail the diff |
| C4 | Light-tree proposal | Bias against the brute sum. Not a bit match to the flat target CDF |
| C5 | A flag replaces brute direct | One 4070 session, below |

C5, on the owner's 4070, once:

```sh
make lighting-oracle PROFILE=full
cargo run --release -p genos-lighting -- --scene stress --size 2560x1440 --gi-metrics on --gpu-times 30 --frames 120 --out genos-debug/restir-c5
```

`ref_err_near` is no worse than master. `grid_worst` ≤ 0.15. The direct pass is ≤ 3 ms at 1440p. Those are the ADR 0022 stage C gates. A miss drops or reworks the stage. It does not land on a promise.

## References

- [ADR 0022](../../adr/0022-lighting-v3-compute-architecture.md)
- [Lighting record](README.md)
- [Debug standard](debug-standard.md)
- [Oracle harness](oracle-harness.md)
- [Glossary](../../lighting/GLOSSARY.md)
- [Pairwise merge](../../../crates/render/src/lamp_pick.rs)
- [Reference departures](../../../crates/restir-ref/src/lib.rs)
- Talbot, Cline, and Egbert, "Importance Resampling for Global Illumination", EGSR 2005.
- Bitterli, Wyman, et al., "Spatiotemporal reservoir resampling for real-time ray tracing with dynamic direct lighting", TOG 2020.
- Bitterli et al., "Generalized Resampled Importance Sampling", SIGGRAPH 2022.
- Wyman, SIGGRAPH 2023 course (pairwise MIS, Jacobian 1 for point lamps).
