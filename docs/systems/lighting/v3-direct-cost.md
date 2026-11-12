# Direct cost and off-screen fill (still hall)

Branch `cursor/still-converge-642e`. Lavapipe, hall pose of `still_converge` (eye `(14.5, 1.7, 23.5)`, yaw `-0.3554`, pitch `0.08`), preset High, 320×180. Absolute milliseconds here are lavapipe. They are not the RTX 4070. Anthony's 4070 numbers, quoted for comparison and not remeasured on this machine: direct light 9.6 ms on v3 against 1.3 ms on master `e0717bdd`; basic GI on 44 fps against 114; stress with GI off 93 against 162.

The 32-frame exact hold is already gone on this branch. `RESTIR_MEAN_CAP` and `LAMP_PICK_MEAN_CAP` are 4096. There is no `LIGHTING_DIRECT_HOLD`. A restir product pixel shades the reservoir on a quiet frame. Nothing in this note puts a hold back.

## ReSTIR split, lavapipe, 320×180, 24 still frames

`GENOS_GPU_TIMES=1`. `Renderer::lighting_restir_ms` is the mean of the collected frames. `lighting_pass_ms` is the last collected frame. Shade is the direct dispatch, and that dispatch's one visibility ray is inside the shade number. Spatial does not trace.

| Pass | ms |
|---|---:|
| restir initial (candidates; temporal is fused, so this is small) | 0.346 |
| restir temporal (the split stamp; the fuse leaves it nearly empty) | 0.327 |
| restir spatial | 409.093 |
| restir prepare | 3.644 |
| restir shade (direct dispatch, visibility ray included) | 5.099 |
| bucket direct (shade dispatches plus the three restir stamps) | 414.865 |
| bucket hit_light (cache-hit shading) | 1544.176 |
| bucket cache_rays | 2995.820 |
| bucket cache | 12356.905 |
| bucket compact | 472.377 |
| bucket atrous | 10.453 |
| bucket compose | 0.528 |

Place, trace, gather, and filter were 0. Screen probes are not scheduled on this preset.

On lavapipe the spatial stamp is almost the whole ReSTIR time, and the screen shade is 5 ms. Cache-hit shading is 1.5 s and the cache round is 12 s. A 4070's ray-to-compute ratio is the other way up, so this split does not say the 9.6 ms is the spatial pass. A 1440p lavapipe frame was not run: the cache round is already 12 s at 320×180, and the texels are world-sized, so a bigger picture does not change the cell budget this note is about.

No cut in this change. Dropping spatial neighbours to 1 missed the shadow row (0.063). Resolution scale 2 missed the picture gate (0.073). `REF_ERR` stays 0.05. Fewer candidates, a coarser reservoir, or pulling the cache-hit lamp walk out of the shade dispatch change variance or the GI cost and stay proposals for a 4070 measurement. The safe skip that is already on the branch (the near-lamp load on a ReSTIR pixel) was not touched.

## Off-screen cells before a turn

Anthony: once a cell has popped in, turning away and back shows it correct, so persistence works. The question was whether nearby cells are created and filled before they are seen, or whether the budget only reaches on-screen cells.

24 still frames, no turn. 32 surfaces behind and beside the camera, all off-screen (`viewport_uv` misses), all within 6 m (`LIGHTING_WRC_NEAR`). The 1 m band is empty because the hall has no surface that close to this eye, not because a cell was dropped. `shown_frame` on every 25 cm cell below is 0: a pixel has not shown it.

| Band | Behind exist | Behind filled (weight ≥ 1) | Behind missing | Beside exist | Beside filled | Beside missing |
|---|---:|---:|---:|---:|---:|---:|
| 1 m | 0 | 0 | 0 | 0 | 0 | 0 |
| 3 m | 16 | 16 | 0 | 10 | 10 | 0 |
| 6 m | 20 | 20 | 0 | 12 | 12 | 0 |

Weights were 10 to 12. `updated` was frame 23 or 24. Scouts ran on 3 of the 24 frames (4096 rays when due, which is the still interval of 8 plus the opening frames). The world-round scheduler threshold sat at 63 from frame 1, the top of the 64 buckets, so the ranking was saturated, and these cells still took samples.

The 12.5 cm texel at the same 32 points existed for 1 of them. Fine texels are created when a pixel or a ray hits them. A turn creates the ones that were off-screen. They seed from the 25 cm cell (ADR 0039 stage 3) and then take their own samples. That handover is the data that feeds in. The 25 cm cell under it was already filled, which is why turning away and back stays correct once the texel exists.

No shader change. Nearby off-screen world cells already get a share of a saturated budget. Routing them off the ranking onto the hash-order background list would give the same cells the list every round (the S72N patch fill) and was not done.

## Shadow and penumbra, 16 still frames

Same pose. The shade ray starts at `position + normal * 0.02`. CPU `sees` from the surface (1 mm along the normal) and from that offset agreed on every facing point lamp: `disagree` 0. The visibility mean is the product mean (`mean_kind` 3). CPU Y is the point-lamp sum only (`LAMP_UNIT` 72, radius 0.1, visibility included). The sun is not in either number. The reservoir `W` is `w_sum / (M * target)` from the spatial scratch the shade reads.

| Pixel | Class | Facing | Visible | Blocked | Mean M | Mean vis | Mean Y | CPU Y | Reservoir M | W |
|---|---|---:|---:|---:|---:|---|---:|---:|---:|---:|
| 25848 | lit | 3 | 3 | 0 | 16 | true | 0.2658 | 0.2329 | 64 | 2.437 |
| 29720 | full shadow | 3 | 0 | 3 | 16 | false | 0.0000 | 0.0000 | 256 | 1.897 |
| 2568 | penumbra | 5 | 3 | 2 | 16 | true | 3.7765 | 3.6749 | 320 | 1.148 |

The mean counted every frame (M = 16 after 16 frames). It did not reset. Full shadow matches the CPU sum at 0. Penumbra is 2.8% high. The lit pixel is 14% high at 16 samples. `W` is finite and positive. Reservoir M is 64, 256, and 320, so temporal reuse is holding history. Spatial reuse does not trace; the shade retraces the chosen sample, and the mean above is that retraced sample. Nothing here is an origin inside the solid, a NaN weight, a mean stuck at one sample, or a penumbra mean that has left the CPU sum.

A full shadow's direct light is stably black, so grain that remains there is the cache, not this mean. No direct-light change. `REF_ERR` 0.05 and the knockout late ceiling 0.213 are unchanged.

## Probes

`Renderer::read_cache_cells` is the cell table. `Renderer::read_cache_round` is live, claimed, the scheduler threshold, rays, due, waiting, and scouts. `Renderer::read_direct_samples` is the scratch reservoir and the visibility mean for up to eight pixels. `Renderer::lighting_restir_ms` is the five-pass mean when `GENOS_GPU_TIMES` is on. `Renderer::lighting_restir_last_ms` is the last collected frame of that split. `StoredCell::seed` and `seed_weight` are the texel's seed, apart from its own mean. Replicated state: none (the copies stay on the machine that shades). Authority: that machine. Prediction / rollback: none. Determinism: the same pose and the same frame count produce the same cells. Headless server: a server that does not shade leaves them uncomputed.

Tests: `nearby_offscreen_world_cells_hold_samples_before_they_are_seen` and `shadow_and_lit_pixels_keep_a_visibility_mean` in `examples/stress/tests/offscreen_cells.rs`. `shadow_and_penumbra_pixels_trace_the_cache` and `a_turn_seeds_fine_texels_from_the_world_cell` in `examples/stress/tests/cache_grain.rs`.

## Cache grain on a still hall

Same pose, High, 320×180, 36 still frames. The presented picture is the measurement. `read_scene` returned a 320×180 buffer of zeros on every frame; the presented image did not, so the zeros are that readback and not the shade.

Full shadow is pixel 31000, the wall at `(19.29, 2.19, 8.15)` facing +Z. Three lamps face it and all three are blocked. Penumbra is pixel 2568, the ceiling at `(11.81, 3.0, 22.29)` facing down. Five lamps face it, three visible and two blocked. The CPU path tracer (`radiance`, six bounces) is 0.176 at the shadow pixel and 1.536 at the penumbra pixel.

The 12.5 cm texel is claimed on frame 0. Its stored point plus `0.02 * normal` is in front of the face (`z` 8.17 against the wall at 8.15) on every frame. It is not inside the wall, and neither is the stored point. The same is true of the ceiling texel. No ray deposit in this trace starts inside geometry. The still texels never store a seed: their own samples arrive first, so the seed record stays zero. The 25 cm cell is filled as well.

The picture does not go black. At frame 16 every one of the 57600 pixels is above 0.05, and none of them falls below a fifth of that value over the next 19 frames.

Settled picture, frames 12 through 35 (population variance):

| Pixel | Presented mean | Presented variance | Min | Max | Path tracer | Own-mean variance |
|---|---:|---:|---:|---:|---:|---:|
| shadow 31000 | 0.215 | 0.000137 | 0.184 | 0.227 | 0.176 | 0.0097 |
| penumbra 2568 | 1.542 | 0 | 1.542 | 1.542 | 1.536 | 0.024 |

The penumbra picture matches the path tracer and does not move. The shadow picture is 22% above the path tracer and still climbing a few thousandths a frame; that climb is the variance, not a pixel dropping out. The texel's own mean moves more than the picture (0.69 at frame 12, 1.02 at frame 32, 0.93 at frame 35). The filtered field is what the pixel shows, and it holds the picture still. Nothing in this trace is a black deposit, a flipped normal, or a 12.5 cm texel swapping to an empty 25 cm cell.

No shader change. A still frame on lavapipe does not contain the black flicker. The `|f640-f320|×8` speckle is this residual: the own mean is still moving after the picture has settled, and a later push of that mean is a small per-texel step.

## Seed on the turn

16 still frames, then the camera turns to the centroid of the 32 off-screen surfaces from the earlier fill check. 9 of the 32 land on screen. After 4 frames, 7 of those 9 have a 12.5 cm texel seeded from the parent (confidence −0.25, the detail mark). After 20 more frames those seven have weight 12. Two on-screen points never get a fine texel; their 25 cm cells stay at about 0.5.

Among the seven that filled, the largest gap between the seed and the texel's own mean is 0.255 of the larger of the two (seed 2.37, own 1.89). The largest gap between the 25 cm cell and that own mean is 0.439 (cell 3.71, seed 2.98, own 2.08). The bilinear seed is closer to the fine texel than the single parent cell is. Replacing the seed with the parent cell would make the pop larger, not smaller.

The 25 cm cells are already filled off-screen, and the ray budget is already saturated (threshold 63 in the earlier run). Creating 12.5 cm texels for every nearby surface in all directions, and tracing them before they are seen, is the way a turn would show no change. Those texels come out of the same ray ceiling.

The fine band does that. Compact claims four 12.5 cm texels from each 25 cm cell whose hit is inside `surface_cache.fine_lod_create` (High default 6 m), in every direction. `surface_cache.fine_lod_drop` (8 m) is farther: an off-screen fine texel past it is freed, and one inside it keeps its rays. `surface_cache.fine_lod_lookahead` (2 m) stretches the band in the direction of motion and a turn. `surface_cache.fine_budget_share` (0.10) reserves that fraction of the existing texel ray room for the off-screen texels that are not yet settled. The camera only raises priority (`surface_cache.fine_screen_boost`, `surface_cache.fine_close_boost`). There is no second store. The headless check is `a_yaw_turn_finds_nearby_surfaces_already_filled`.

On this lavapipe, that check (24 still frames, then the same yaw) brought 9 surfaces on screen. Two runs of the same distances: the entry-frame presented-luma change fell from 0.1042 to 0.0473, then from 0.1049 to 0.0535. The second run is the one this commit ships. With the band off, none of the 9 had a fine texel. With the band on, 7 of the 9 already had a fine texel of weight at least 4 (mean weight 6.00) before the turn. One more had weight 3. One existed at weight 0. The mean pixel change was 0.0535, 51% of the off figure 0.1049.

Cost, same binary, 320×180, High, 24 still frames, frozen sun, still boxes, dynamic 0. Mean of the 18 frames after the first four, from the timestamp line `--profile-passes` turns on. Lavapipe, not a 4070. Band off: lighting 666.5 ms, frame 670.8 ms. Band on: lighting 735.8 ms, frame 739.9 ms. The ray ceiling is the same. The extra 69 ms is the compact claims and the extra texels inside that ceiling. The pass CSV wrote only its first timestamp row (frame 3, still warming): shade 4.898 ms off and 4.603 ms on, cache round 11.272 ms off and 12.392 ms on.

## 4070 pass profile

Lavapipe milliseconds are not a 4070 result. `--profile-passes FILE` writes one CSV row per collected timestamp frame from the same GPU timestamps. A draw with no new sample adds nothing. The first two draws have no sample, because each flight slot is read on its second use. The row is one frame behind the draw. `shade_visibility_ms` is the direct shade dispatch and includes its visibility ray. `cache_hit_shading_ms` is the hit-light dispatch. `cache_round_ms` is pass 6, the relight, the same work as the `cache` column of `LIGHTING_MS`. `sh_prep_ms` is pass 9. The nine-wide `LIGHTING_MS` total leaves pass 9 out, so that line's total is not the lighting span. The cache bucket counts the prep with the relight. `cache_rays_ms` is the ray dispatch of the round. `filter_ms` is a-trous. `direct_ms` is the direct bucket, the `light_px` column (candidates, temporal, spatial, and shade). `compact_ms` is pass 0. `lighting_span_ms` is the `lighting` column of `GPU_MS` for that collected frame. Temporal is fused into the candidate stamp unless the passes are split, so that column stays near empty. An omitted `--gpu-times` turns timestamps on every frame. `GENOS_PROFILE_PASSES` is the path only when the argument was omitted.

A print every frame used to replace the timestamp record with a fresh one and drop `lighting_ids`. The other flight slot's pass list had not been read yet, so the next collect skipped the lighting queries. `fd9bb4de` on the 4070 wrote one CSV row, frame 3, out of 90 draws. The print now keeps that unread list. The same run's `GPU_MS` lines were already one per frame. An 8-frame lavapipe run at 320×180 wrote rows 3 through 8, and `lighting_span_ms` matched that frame's `GPU_MS` lighting column. A 90-frame run writes 88 rows. The first two draws have nothing to read, and the last submission of each slot is still in flight when the process exits.

```
cargo run --release -p genos-stress -- --proof --size 2560x1440 --preset high --frames 90 --still-boxes --freeze-sun --dynamic 0 --profile-passes ./genos-debug/passes.csv
```

### Frame 3 on the 4070, 2560×1440, High, still

`fd9bb4de`, the one collected row. It is the first frame's GPU work (the first time slot 0 was read). `compose_valid` starts false, so maintenance and compose cover the whole picture. The cache is empty: `cache_rays` 0.040 ms, relight 0.010 ms.

| Column | ms |
|---|---:|
| candidates (4 per pixel; temporal is fused into this stamp) | 2.170 |
| temporal | 0.000 |
| spatial | 2.370 |
| prepare | 0.486 |
| shade + visibility | 2.940 |
| direct (candidates + temporal + spatial + shade) | 7.480 |
| cache-hit shading | 0.432 |
| cache round (pass 6, relight) | 0.010 |
| sh prep (pass 9) | 10.385 |
| cache rays | 0.040 |
| compose | 6.201 |
| filter (a-trous) | 0.682 |
| compact (`LIGHTING_MS`, not in that CSV) | 0.035 |

`LIGHTING_MS` total on that print is 14.881. Pass 9 is not one of its nine slots, so that total leaves out the 10.385 ms of sh prep. 14.881 + 10.385 + prepare 0.486 = 25.752. The same print's `GPU_MS` lighting span is 26.646. The 0.9 ms left is the full-frame image copy into the lighting buffer, the copy back out, the compose-rect fills, and the barriers.

Later `GPU_MS` lighting, same run, no per-pass rows: the second printed frame is 52.6 ms, then 41.8, 36.7, 37.8, 34.3, and the band after that is 28.3–30.7 ms with the frame at about 30–31 ms. The opening span is already 26.6. Steady state sits 2–4 ms above it. The early spike is the in-frame cache round once cells exist. High leaves `cache_async` off and `cache_hz` at 0, so a due round runs inside the frame. The batch is 98,304 entries by 16 rays (1,572,864 rays) plus the relight. The opening frame dispatched an empty list. Those milliseconds will show up in `cache_rays_ms`, `cache_hit_shading_ms`, and `cache_round_ms` on the rows this fix writes. Pre-v3 master direct was 1.3 ms and the basic scene held 114 fps. This still hall is about 33 fps, and the lighting span is the frame.

### Cuts, biggest first

No shader change in this fix. Every cut below changes the picture or the variance. `REF_ERR` stays 0.05. Half-res reservoirs and one spatial neighbour already missed that gate on lavapipe (0.073 and 0.063).

| Cut | What it is | Expected save on this 4070 frame | Quality risk against `REF_ERR` 0.05 |
|---|---|---|---|
| sh prep, stop walking every pixel | Pass 9. Clear 16 distance bins, then the maintenance compile of compose, then a drain of 16,384 threads. On this opening frame `compose_full` is set, so maintenance is one 8×8 group per block: 320×180 groups, 3.69 million pixels. Each pixel loads the surface and runs `lighting_screen_probe_indirect` (four world-cache corner finds, coverage atomics, an enqueue of an empty 12.5 cm texel, a lattice request). The colour is thrown away. The drain then fills the lattice slots and stores detail seeds. The drain is per cell. The 10.4 ms is the per-pixel walk. Compose does the same walk again, after the drain, to store the pixel. The drain has to sit between those two, so they cannot be one dispatch. A still frame with `still > 0`, compose reuse on, and 57,600 blocks under the 65,535 cap should switch both to the dirty block list. If the new rows keep sh prep near 10 ms, the list is still covering the picture and the cut is to skip a block whose texels did not change. | Up to about 10 ms on a full frame, and little once the dirty list is actually small. The corner finds are most of the cost; dropping only the discarded blend saves less than the whole 10.4. | A skipped block that did change keeps the old shade (`lighting_compose_cover`). That misses the picture gate. Not implemented. |
| compose 6.2 ms | The picture store. Same full grid on this opening frame, same world-cache read, then the shoulder and the write. It has to run after the drain, because it loads the lattice the drain just filled. | Same shape as sh prep: up to about 6 ms while the frame is full, less on a real dirty list. | Skipping a dirty block fails the picture. Not implemented. |
| direct, fewer candidates | `RESTIR_CANDIDATES` is 4. The 2.17 ms stamp is the initial pass with temporal fused into it. | About 1 ms if the candidate trace is most of that stamp. Temporal reuse does not shrink with the candidate count. | Variance goes up. Not measured against 0.05. Not implemented. |
| direct, half-res reservoirs | `restir_resolution_scale` 2. Shade and its visibility ray stay at full resolution (2.94 ms). | Candidates plus spatial are 4.54 ms. A quarter of the reservoir pixels would take that toward about 1 ms, about 3 ms saved, if the cost scales. | Already measured. Resolution scale 2 missed the picture gate at 0.073. Not implemented. |
| direct, cheaper spatial | Spatial is 2.37 ms and does not trace. One neighbour was the trial. | About 1 ms. Not the whole 2.37. | One neighbour missed the shadow row at 0.063. Not implemented. |
| direct, visibility reuse | The shade dispatch is 2.94 ms and traces one visibility ray per pixel. | Some of that 2.94. The ray and the shade math are one stamp, so the split is not known. | The reservoir sample changes as history grows, so a reused bit is not the same picture. Not implemented. |
| cache round once the cache is busy | The 52.6 ms spike and the 2–4 ms that remain in the steady band. In-frame, not async. | The steady band has about 3 ms over the opening span. The spike is larger and then goes away. The new CSV names the column. | Fewer rays or a smaller batch slows convergence and moves a still picture. Not implemented until the rows show which column grew. |
