# ReSTIR DI — stage C

[ADR 0022](../../adr/0022-lighting-v3-compute-architecture.md) stage C replaces brute direct with ReSTIR DI. This note is the CPU reference and the GPU stages that follow it. C5 is the live flag. It stays off.

## Reference

`genos-restir-ref` (binary `restir-ref`) is the checker. It does not open a window or a Vulkan device, and it does not depend on `genos-render`. The pairwise weight is the same threshold as `lamp_pick::pairwise_merge`, copied so the crate can build without a device.

Initial candidates are resampled importance sampling (Talbot, Cline, and Egbert, EGSR 2005). Temporal and spatial reuse follow Bitterli et al., TOG 2020. The weight is the balance heuristic (Bitterli et al., SIGGRAPH 2022, GRIS equation 22). Two reservoirs use the shader's history test. A longer list uses one prefix-sum pick, then

`W = area * p̂_src / (p̂_q(y) * Σ_i M_i * p̂_i(y))`

with `area = Σ M_i * W_i * p̂_q(y_i)`. The shade is unshadowed radiance times visibility times `W`. `p̂` is Rec. 709 luminance of the unshadowed Lambertian radiance. Visibility is not in `p̂`. One shadow ray runs at shade time.

`make restir-ref` writes `./genos-debug/restir/metrics.json` and `reference.json`. `--out` overrides that directory. `GENOS_RESTIR_OUT` applies only when `--out` was omitted. `restir-ref diff --reference FILE --candidate FILE` writes `diff.json` for a later GPU dump in the same schema.

## Departures

Named in `crates/restir-ref/src/lib.rs` and repeated here.

- Proposal is uniform, an exact discrete target CDF, or a light-tree bound (C4). The tree is not the flat target CDF.
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

No MCP entry on the checker. The checker is offline, the same way `reflections-check` is. The live flag is a lighting setting (`restir_di`), so `set_key` and the debug-menu switch reach it. It stays off.

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
| C4 | Light-tree proposal. `--proposal tree` (off by default). One leaf per candidate from a BVH whose node importance is the Conty and Kulla / Estevez and Kulla bound. `--tree-fault ignore-distance` drops the inverse-square term and must fail the reservoir diff. The live direct pass is unchanged | Bias of the multi-seed mean against the brute sum, ceiling `ref_err_near` 0.05. Not a bit match to the flat target CDF. CPU and GPU reservoirs for the same seed: 0 mismatches |
| C5 | A flag replaces brute direct | One 4070 session, below |

C4 measured on this crate's tests (debug). Bias is `ref_err_near` of the multi-seed mean, 128 seeds, 8 frames, 8 candidates, temporal on, spatial off. Variance is the frame-1 sample variance at the brightest receiver, 48 seeds, 8 candidates, temporal off. The flat column is the exact target CDF. Brute error is the denominator of `ref_err_near` (zero by construction).

| Scene | Tree bias | Flat bias | Uniform bias | Tree var | Flat var | Uniform var |
| --- | --- | --- | --- | --- | --- | --- |
| gate | 0.0114 | 0.0000 | 0.0068 | 0.191 | 0 | 10.84 |
| shadow | 0.0357 | 0.0286 | 0.0267 | 0.634 | 0.227 | 11.38 |
| open | 0.0018 | 0.0000 | 0.0058 | 0.028 | 0 | 0.288 |
| row | 0.0021 | 0.0000 | 0.0109 | 0.0049 | 0 | 0.109 |
| domain | 0.0000 | 0.0000 | 0.0037 | 0 | 0 | 6601 |
| variance | 0.0117 | 0.0000 | 0.0075 | 0.0605 | 0 | 0.0487 |
| many (256 lamps) | 0.0052 | 0.0000 | 0.0764 | 38.17 | 0 | 71475 |

On `variance` (20 lamps at similar range) the tree's one-frame variance sits just above uniform. On `many` it is about 1800× lower. `--tree-fault ignore-distance` on `many` stays inside the 0.05 bias ceiling (0.021 at 96 seeds, 8 frames) and disagrees with the distance-aware reservoirs (14 lamp mismatches, seed 1, one frame). Lavapipe `restir-gpu --initial --proposal tree --scene many`: dispatch 186 ms, reservoir mismatches 0 against the CPU dump. The same fault on the GPU matches the CPU fault and fails the diff against the distance-aware tree.

C5 wires this walk into the live direct pass behind `restir_di` (`--restir-di` on `genos-stress`, mirror `GENOS_RESTIR_DI`). The flag stays off, so a default frame still shades the brute or pairwise sum. When it is on, the pass uploads this tree with the lamp list, runs the same initial, temporal, and spatial shader includes, and shades `unshadowed * visibility * W`. A lamp-list change or a geometry change record clears history the way `Session::reset_temporal` does. A camera turn reprojects; a depth or normal break drops the neighbour. Spatial reuse stays on. Each pixel draws 4 tree candidates a frame and history `M` is capped at 80 (20 frames of 4). On the 4070 at 1440p, 4 candidates instead of 8 took the initial pass from 5.96 to 3.09 ms at near-camera error within 0.002 of 8 on spin, sun, and turn. Screen pixels only: hits stay on the existing shade. Lavapipe checks the flag-on direct pass against brute direct on `Scene::gate` and `Scene::shadow` (`ref_err_near` 0.05). The grade is still the owner's 4070 session below. The stress package is `genos-stress`. Reservoirs are about 64 bytes per pixel plus the prefix, about 236 MB at 1440p, and they are allocated only while the flag is on.

C5, on the owner's 4070, once:

```sh
make lighting-oracle PROFILE=full
GENOS_RESTIR_DI=on make lighting-oracle PROFILE=full OUT=genos-debug/oracle-restir
cargo run --release -p genos-stress -- --scene stress --size 2560x1440 --gi-metrics on --gpu-times 30 --frames 120 --out genos-debug/restir-c5
cargo run --release -p genos-stress -- --scene stress --size 2560x1440 --gi-metrics on --gpu-times 30 --frames 120 --restir-di on --out genos-debug/restir-c5-on
```

`ref_err_near` is no worse than master. `grid_worst` ≤ 0.15. The direct pass is ≤ 3 ms at 1440p. Those are the ADR 0022 stage C gates. A miss drops or reworks the stage. It does not land on a promise. `ref_err_near` and `grid_worst` are fields in `metrics.json` under the oracle directory (`genos-debug/oracle` with the flag off, `genos-debug/oracle-restir` with `GENOS_RESTIR_DI=on`). The direct-pass milliseconds are `light=` on the stderr `LIGHTING_MS` line of the stress run. With the flag on, that line also prints `restir_initial`, `restir_temporal`, and `restir_spatial`, and `light=` includes their sum.

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
