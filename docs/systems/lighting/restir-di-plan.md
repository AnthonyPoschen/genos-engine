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

## Addendum: world-space light lists (ReGIR), Proposed (5 Nov 2026)

Status: Proposed. No renderer code until Anthony accepts it. The numbers here are estimates unless they say measured.

### Why

C5 turn error is 0.539 against 0.403 with the flag off. Spin and switch are close to the flag-off numbers. When a pixel loses its history it has 4 fresh tree candidates and one shaded sample. That shows as noise until `M` grows back. Anthony asked (5 Nov) whether light-sampling data should live on surfaces instead. The answer is a hybrid:

- World-space cells hold a short list of the lights that matter there. This is ReGIR (Boksansky, Jukarainen, and Wyman, "Rendering Many Lights with Grid-Based Reservoirs", Ray Tracing Gems II, 2021, and RTXDI's ReGIR mode). A cell survives camera moves and is shared by every pixel that lands in it.
- Per-pixel reservoirs stay. A cell is too coarse for shadow edges and for how the lighting changes with surface direction.
- A pixel that has history keeps it, including through camera moves. A pixel that is newly visible starts from its cell's list instead of from nothing.

Constraints that do not change: ReSTIR replaces the old per-light pass. There is no exact-sum fallback at any lamp count. The ADR 0022 stage C gates stay as written above.

### Cells

- The cells reuse the light cache's keying: the same world hash, the cell size doubling with camera distance, and the point taken half a cell in from the surface along the normal (`crates/render/shaders/lighting_cache.glsl`). They also key on the same six facing axes, because a Lambertian receiver ignores lamps behind it, so a cell that mixes floor and ceiling wastes its list.
- They are coarser than the cache: about 1 m near the eye (4× the 25 cm WRC cell), doubling with distance. Light lists vary slowly in space, and the build cost scales with the number of cells.
- They go in a separate small table, not in the 368 B cache entries. The granularity, the lifetime, and the eviction rules all differ, and one table must not starve the other. Only the hash and the keying code are shared.
- Only active cells are built: those a pixel read this frame or last frame, compacted into a list. Cells nobody reads are not built. A cold cell is built the first frame a pixel asks for it, so a disoccluded pixel never reads an empty list.

### Building and updating the lists

- Each active cell holds K slots: K = 16 on High and Ultra, 8 on Medium and Low. Each slot is a light index plus that slot's contribution weight `W`.
- Every slot is a fresh RIS draw each frame. The candidates come from the C4 light tree, and the target is the cell's conservative bound (below). Nothing is carried from frame to frame, so the lists hold no history of their own.
- The lists are built after the lamp upload in the same frame. A lamp that turns on, off, or changes power is in the next list with no lag. A moving lamp is placed at this frame's position. A removed lamp cannot leave a dangling index, because no list outlives its frame.
- The cell lists add nothing to the lamp-change rules. Pixel history still follows the existing C5 change record and the 80 `M` cap. Moving lamps still stale by at most the `M` cap, as `a_moved_lamp_stops_being_stale_inside_the_m_clamp` checks.

### Unbiased resampling from cells

This is the part that has to be right first.

- **Cell target.** For each lamp, it is the largest unshadowed contribution over the cell's bounds and over the facing axis's hemisphere of normals. It is never a value at the cell centre. Any lamp that can light any point in the cell has a nonzero chance of being in the list. Dropping a lamp this way would bias the image with no sign of it in the noise, and that is the failure this design must not have.
- **Slot weight.** Each slot's `W` is the RIS contribution weight of its draw, so it is an unbiased estimate of 1/p for that light. Feeding the slot into a pixel's reservoir with `W` in place of 1/p is GRIS resampling from samples with contribution weights (Lin et al., SIGGRAPH 2022). Neighbouring pixels share slots, which correlates their noise but does not bias them.
- **Mixing with the tree.** Each pixel's initial candidates are a mix of cell draws (a uniform pick from the K slots) and tree draws. They are combined with constant MIS weights in proportion to each strategy's candidate count. Constant weights are unbiased only when both strategies cover the pixel target's support. The tree covers it by construction, and the conservative cell target covers it by the rule above. At least one tree candidate per pixel stays as the defensive strategy.
- **Choosing the mix.** The split may depend on the pixel's history: thin history (low `M`, a disocclusion) means more cell candidates and more spatial neighbours, which is the paper's answer for thin history. The split depends only on state from before this frame's draws, so it does not bias the estimate.
- Temporal and spatial merges are unchanged: the balance weight and the C3 history rule.

### Temporal reprojection

- On a camera move, a pixel looks up where its surface was last frame and keeps that reservoir if depth and normal agree, under the same thresholds the spatial pass uses. History is dropped only on disocclusion, a depth or normal break, or a change record. Agreed with Anthony on 5 Nov. That screen-space fix is in the #120 work now and does not depend on cells.
- Cells cover what reprojection cannot: a newly uncovered band starts from a list that is already close, and from spatial neighbours. It no longer starts from 4 tree candidates.
- The CPU reference's "history is the same index" departure ends at stage R0 below. The checker gains a camera shift and a disocclusion band.

### VRAM

| Item | High / Ultra (1440p) | Low (1080p, GTX 1060 6 GB, 4 GB floor) |
|---|---|---|
| Cell table, capped active cells | 32K cells × (4 B key + 16 × 8 B) ≈ 4.3 MB | 8K × (4 + 8 × 8) ≈ 0.55 MB |
| Active-cell list | ≈ 0.25 MB | ≈ 0.06 MB |

The cell lists fit inside the ~200 MB lighting soft budget and the 4 GB floor. The reservoirs already there do not. At about 64 B per pixel plus the prefix, C5 is about 236 MB at 1440p (above) and roughly 130 MB at Low's 1080p. That is beside this addendum, but it is the larger VRAM risk for ReSTIR as a whole (open question 3). The active-cell cap scales with resolution like ADR 0036. When it overflows, the farthest cells fall back to tree-only candidates. That is still unbiased, only noisier.

### Cost (estimates, unmeasured)

- **4070, 1440p.** The build is about 32K cells × 16 slots × a few tree candidates. That is well under the 3.7M px × 4 tree walks the initial pass does now (measured 3.09 ms), so the target is ≤ 0.5 ms. On the pixel side, one hash lookup plus a slot read replaces some tree walks, so the initial pass should get cheaper. The ADR 0022 gate is the whole direct pass ≤ 3 ms. Today it is about 7 ms over. Cells are justified by error first, and they must not add to that overrun.
- **GTX 1060, Low, 1080p.** That is 2.07M px, ~192 GB/s, 8K cells, 8 slots, and 1–2 cell candidates plus 1 tree candidate per pixel. Low's direct-light share of the 60 fps frame is the cap. It is unmeasured, and no 1060 run is planned in this addendum (open question 4).

### Test plan (reference first)

1. **R0, CPU (`crates/restir-ref`).** Add cells to the receiver grid, the conservative cell target, the K-slot RIS build, and `Proposal::Cells` mixed with the tree. Gates: multi-seed `ref_err_near` ≤ 0.05 against the brute sum on gate, shadow, open, many, and a new turn scene (camera shift plus disocclusion band). The turn scene must beat tree-only at the same candidate count on frame-1 variance in the disoccluded band.
   - Faults that must fail: a centre-point cell target (bias beyond 0.05 on a scene built to expose it) and dropping the defensive tree candidate on that scene.
   - Before grading anything new, re-run the flag-off path and the existing gates so the reference itself is confirmed first.
2. **R1, GPU cell build on lavapipe.** For a fixed seed, CPU and GPU cell slots match: 0 index mismatches and `W` within 1e-3. A perturbed seed fails.
3. **R2, pixel candidates from cells.** These sit behind a sub-switch of `restir_di`, off by default. Lavapipe reservoir diff against R0, and the C5 lavapipe check (`Scene::gate`, `Scene::shadow`, `ref_err_near` 0.05).
4. **R3, product grade on the 4070.** This is one batched session: `make lighting-oracle` with and without the switch, and the stress run as in C5. The pass condition is the turn error at or below the flag-off 0.403, with spin and switch no worse, `grid_worst` ≤ 0.15, and the direct-pass milliseconds against the 3 ms gate.

### Stages

R0 → R1 → R2 → R3, each a small PR into the #120 branch. The 4070 is used only in R3. Reprojection that keeps history is already in progress and lands first, so R3 measures what cells add on top of it. If R3 misses, the cells are dropped or reworked, and ReSTIR does not regain an exact sum.

### Open questions for Anthony

1. Cell size near the eye: 1 m (proposed), or 0.5 m at twice the build cost.
2. Should cells key on the facing axis (proposed) or on position only, as RTXDI does? Position only means fewer cells and a weaker list on thin geometry.
3. The existing reservoirs are about 236 MB at 1440p, above the 200 MB soft budget. Should they get a separate memory pass (for example, packing to 32 B), or is Ultra/High allowed over it as gi-design permits for speed?
4. GTX 1060 numbers: is there a Low-floor card available, or does Low stay estimated until one is?

References added: Boksansky, Jukarainen, and Wyman, "Rendering Many Lights with Grid-Based Reservoirs", Ray Tracing Gems II, 2021. NVIDIA RTXDI, ReGIR mode. Lin et al., "Generalized Resampled Importance Sampling", SIGGRAPH 2022.

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
