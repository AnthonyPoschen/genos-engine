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

The fine band does that. The fill visit claims four 12.5 cm texels from each 25 cm cell whose hit is inside `surface_cache.fine_lod_create` (High default 6 m), in every direction, and does not probe that cell again until it leaves the drop distance. `surface_cache.fine_lod_drop` (8 m) is farther: an off-screen fine texel past it is freed, and one inside it keeps its rays. `surface_cache.fine_lod_lookahead` (2 m) stretches the band along motion and a turn. A still view does not. Those children are candidates in the tile-fill histogram under `surface_cache.fill_budget` when the frustum cannot see them. They do not take the cold-start rank, and they contend for one off-screen bin in eight. There is no second list and no reserved share of a second room. A full probe chain counts a miss only when the retry also fails, and it evicts the farthest texel past the drop distance, then a settled one, then a young off-screen one. A texel a pixel showed in the last 60 frames is not a victim. There is no second store. The headless checks are `a_yaw_turn_finds_nearby_surfaces_already_filled` and `a_move_stretches_the_fine_band_and_a_still_view_does_not`. The numbers in the next paragraph are this histogram on lavapipe.

A newly spawned child does not wait for four of its own sets. It is seeded from the on-plane 25 cm cells: bilinear when they agree, the nearest cell when they disagree by the sparse luminance ratio, off-plane corners left out, radiance not clamped. The birth frame often has no parent sample yet, so the prior is refreshed while the child is off-screen and short of four own sets, and it freezes when the frustum can see the child. The prior counts as at most four sets, and that weight fades as the child gains sets. At four own sets that pass stores the child's own field and clears the prior mark, on screen or off. Later off-screen samples keep that store on the current own mean until a pixel shows the child. The screen filter may walk the field closer to that mean; a step that leaves it is not stored. Compose trust for that child is 1 until the mark clears, so the pixel shows the blend from the first visible frame and the child's own field once the prior has retired. The prior's sample count is not written into the child's own count. Readiness in the yaw check is that stored prior, not four own sets. The 0.6 entry-change assert is unchanged.

On this histogram, lavapipe, the same yaw (24 still frames, then the turn, 320×180, High), one run after the off-screen store. Band off, pixel delta 0.1650, filled 0/9, rays 100000, missed 0, freed 3241. Band on, pixel delta 0.0466 (ratio 0.282), filled 9/9, mean own weight 0.22, rays 100000, missed 0, freed 3191. The 0.6 assert was not changed. A child at five own sets whose parent seed was 2.5039 against an own mean of 0.2739 held field 0.2735 while it was off screen. The three-set blend of that pair is 0.8314, a quarter of the parent gap. The off-screen read is the child's own luma. Aimed at that child for 32 frames, the field stayed 0.2735 and the own mean stayed 0.2739. The same bound rejects the previous head's off-screen read (field 2.1720, own 0.4215, seed 7.4908): the leftover 1.7505 is above 0.02 of the larger luma. Fill knockout on the same shader: early 0.6047, late 0.2025 (ceiling 0.213), mean abs 58.905 codes. Still hall, `MIN_DROP` 0.0008 unchanged: error 0.1177 at frame 192, 0.0940 at frame 384, drop 0.0237, min rays 100000. A temporary 262,144-slot table (both shader defaults and the host slot count, then reverted; the product default stays 1,048,576): band off missed 0 freed 3304 delta 0.0954, band on missed 2 freed 3226 delta 0.0486 (ratio 0.509), filled 9/9, rays 100000 both. The product assert `missed == 0` failed on that table and was not loosened. This head's compose module is 247924 bytes, under the 248000 ceiling. Specialization constant 1 on that module was 252688 bytes, 5540 above the folded lighting-v3 module (247148), because every slot mask stayed dynamic. The ceiling was not raised. The earlier ray-share measurements, on the shader before this prior, stay as the reason the child takes one bin in eight: every bin without the cold-start rank was late 0.2200, empty children with the cold-start rank were late 0.2188, and a parent-must-hold-four-sets gate was late 0.2251.

Earlier shader, before the parent prior. On this lavapipe, that check (24 still frames, then the same yaw) brought 9 surfaces on screen. Two runs of the same distances: the entry-frame presented-luma change fell from 0.1042 to 0.0473, then from 0.1049 to 0.0535. The second of those runs is the one that measurement shipped. With the band off, none of the 9 had a fine texel. With the band on, 7 of the 9 already had a fine texel of weight at least 4 (mean weight 6.00) before the turn. One more had weight 3. One existed at weight 0. The mean pixel change was 0.0535, 51% of the off figure 0.1049.

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
| sh prep, stop walking every pixel | Pass 9. Clear 16 distance bins, then the maintenance compile of compose, then a drain of 16,384 threads. On this opening frame `compose_full` is set, so maintenance is one 8×8 group per block: 320×180 groups, 3.69 million pixels. Each pixel loads the surface and runs `lighting_screen_probe_indirect` (four world-cache corner finds, coverage atomics, an enqueue of an empty 12.5 cm texel, a lattice request). The colour is thrown away. The drain then fills the lattice slots and stores detail seeds. The drain is per cell. The 10.4 ms is the per-pixel walk. Compose does the same walk again, after the drain, to store the pixel. The drain has to sit between those two, so they cannot be one dispatch. A still frame with `still > 0`, compose reuse on, and 57,600 blocks under the 65,535 cap should switch both to the dirty block list. If the new rows keep sh prep near 10 ms, the list is still covering the picture and the cut is to skip a block whose texels did not change. | Up to about 10 ms on a full frame, and little once the dirty list is actually small. The corner finds are most of the cost; dropping only the discarded blend saves less than the whole 10.4. | A skipped block that did change keeps the old shade (`lighting_compose_cover`). The steady-state hold is the next section. The opening frame stays a full walk. |
| compose 6.2 ms | The picture store. Same full grid on this opening frame, same world-cache read, then the shoulder and the write. It has to run after the drain, because it loads the lattice the drain just filled. | Same shape as sh prep: up to about 6 ms while the frame is full, less on a real dirty list. | Skipping a dirty block fails the picture. The steady-state hold is the next section. The opening frame stays a full walk. |
| direct, fewer candidates | `RESTIR_CANDIDATES` is 4. The 2.17 ms stamp is the initial pass with temporal fused into it. | About 1 ms if the candidate trace is most of that stamp. Temporal reuse does not shrink with the candidate count. | Variance goes up. Not measured against 0.05. Not implemented. |
| direct, half-res reservoirs | `restir_resolution_scale` 2. Shade and its visibility ray stay at full resolution (2.94 ms). | Candidates plus spatial are 4.54 ms. A quarter of the reservoir pixels would take that toward about 1 ms, about 3 ms saved, if the cost scales. | Already measured. Resolution scale 2 missed the picture gate at 0.073. Not implemented. |
| direct, cheaper spatial | Spatial is 2.37 ms and does not trace. One neighbour was the trial. | About 1 ms. Not the whole 2.37. | One neighbour missed the shadow row at 0.063. Not implemented. |
| direct, visibility reuse | The shade dispatch is 2.94 ms and traces one visibility ray per pixel. | Some of that 2.94. The ray and the shade math are one stamp, so the split is not known. | The reservoir sample changes as history grows, so a reused bit is not the same picture. Not implemented. |
| cache round once the cache is busy | The 52.6 ms spike and the 2–4 ms that remain in the steady band. In-frame, not async. | The steady band has about 3 ms over the opening span. The spike is larger and then goes away. The new CSV names the column. | Fewer rays or a smaller batch slows convergence and moves a still picture. Not implemented until the rows show which column grew. |

## Steady filter, prep, and compose

`ce55cd49` on the 4070, 2560×1440, High, still, p50 of frames 30–90. Lighting span 29.7 ms. The opening filter (frame 3 of the earlier run) was 0.68 ms, which is eight dispatches over an empty list.

| Pass | p50 ms |
|---|---:|
| filter | 8.891 |
| sh prep | 4.135 |
| shade + visibility | 3.839 |
| compose | 2.617 |
| candidates | 2.457 |
| cache-hit shading | 2.102 |
| spatial | 1.987 |
| cache rays | 1.266 |
| cache round | 1.210 |
| compact | 0.363 |

### Why the filter is 8.9 ms

It is not the iteration count, the format, or the resolution by itself. Preset high launches the screen wavelet: one gather, one commit, five iterations, one push. Once a texel has 16 sets, the old code set `rebuild` every frame. That disabled the dirty early-out, so the 7×7 variance and the five 5×5 iterations ran on the whole screen list every frame. A new sample still moves the mean by more than one percent, so the dirty test would not have idled those texels anyway. Frame 3 is cheap because the list is empty. After the cells fill, the list is the picture.

A held camera now skips that kernel for a texel that already has 16 sets and has been filtered, once the first 8 frames are over and the picture has not changed. Frame 32 runs it again. A host hash change after that, and a mover still on the keep list, run it for the whole picture for 16 frames. A moving camera does not take the hold. Young texels still run. The first 8 frames do not take the hold. Walking the fill through the frames `screen_hall` scores made still flicker 3.39 against a 1.5 bar.

`examples/stress/tests/still_idle.rs`, 320×180, draws 40–47: `filter_heavy` 24–77 of 54,922 screen pixels. The picture moved 87 codes from draw 2 to draw 47. The list was not rebuilt (`list_rebuilt` 0).

### Why prep stayed at 4.1 ms, and compose at 2.6

The dirty list was already on. It did not get small. A world cell marks its whole screen rect when a coefficient bucket changes, and that happens every round. Maintenance then walks every pixel of those blocks (four cache finds, colour thrown away) and compose walks them again. Lavapipe compose at 480×270 was already 0.06 ms, so a compose millisecond on this machine cannot prove a 4070 cut. The walk count can.

A held camera does not mark a quiet world cell until frame 32. A host hash change, or a mover on the keep list, drops that hold for the picture. A direct sample is compared with the last marked value, per channel, so a ramp of sub-percent steps still marks and a dim channel cannot drop out on a bright pixel. Maintenance is not recorded on the quiet frames (`LightingSunStats::maint_ran` is false; the same test). Compose adds the new direct to the bounce it stored on the last full walk, and does not call `lighting_screen_probe_indirect`, except where a change reaches and during the fill. The same test, after the fill: `cache_walks` 0. Frame 32 marks the quiet cells, runs maintenance, and stores a new bounce. A camera move does not take the hold.

A narrow changed-texel dispatch, a stage-0 gather skip, and skipping compose marks outright were tried earlier. Each of them moved the screen wavelet off the hash wavelet (still p99 about 74–94 against a floor near 9). They are not in this tree. The hold is in the kernel both launches share. `screen_hall` on lavapipe: still closer 0.306/1.667 against floor 2.667, flicker 0.310/2.667. Moving closer 0.208/1.667 against floor 47.667.

### Lavapipe proxy, 480×270, High, still

Not a 4070 millisecond. Same command as below with `--size 480x270 --frames 48`. Median of the named rows.

| Pass | before, frames 12–16 | after, frames 40–48 |
|---|---:|---:|
| filter | 616 | 83 |
| sh prep | 36 | 1.7 |
| compose | 0.06 | 0.06 |
| lighting span | 1058 | 468 |

Frames 12–16 after are still in the fill (filter median 584). The drop is the settled frames. CSV row 32 is the backstop: filter 571, and one nearby prep row is 32, which is the maintenance walk coming back. One spike per 32 frames.

The filter's heavy kernels fell from about ten thousand texels to a few dozen. Lavapipe time fell 7×, not 200×, because an early-out thread is still scheduled. On the 4070 the 8.9 ms is that kernel, and the empty-list launch was 0.68 ms. Prep's dispatch is not recorded on a held frame, so the 4.1 ms walk should leave. Compose's cache walks are 0. Lavapipe compose was already at the noise floor, so the 2.6 ms has to be read on the 4070. The corner finds were the cost.

### Gates

`restir_di`: `restir_di_matches_brute_direct_on_the_gate_scenes`, `restir_di_matches_brute_with_eight_spatial_neighbours`, and `the_same_seed_renders_the_same_picture_twice` pass. `REF_ERR` stays 0.05. `restir_merge_oracle::fixed_reservoirs_match_balance_list_on_both_spatial_pipelines` still fails on this branch and on `lighting-v3`.

`fill_knockout`: early-to-late 86.8 codes, error 0.7259 → 0.0829. `MIN_ERROR_GAIN` stays 0.03. `MIN_DELTA_CODES` stays 4.

A lamp change on a still camera used to wait up to 32 frames, then pop (measured step 0.363 linear, 75.8 codes). An untouched still view popped at the first refresh (0.522 to 0.903, 68.6 codes). The host publishes `still == 0` on the change frame. After frame 32 that arms a 16-frame picture-wide unhold. A mover on the keep list unholds the picture while it is listed. Putting `lighting_sc_lamp_touched`, `lighting_near_change`, and `lighting_sc_local_change` inside the hash wavelet overflowed lavapipe's shader stack (SIGSEGV on `screen_hall`'s hash path). Those walks stay in the relight passes. The hold reads one stamp and the mover count.

Lavapipe, 320×180, High, sun frozen, camera fixed. Open fill, draws 1–47: the mean goes 44 → 68 → 74 → 84 → 106.8 and holds, then frame 32 steps to 131.1. Max step 24.28 codes (the old step was 68). Lamp power to zero after the settle: the toggle frame walks 56922, then 55834 for 16 frames, and the mean goes 130.95 → 110.08 → 76.22 → 52.48 and down to 22.38, then flat. A box sliding for 8 frames walks 56922 a frame (it was 0); after it stops the max step is 0.43 codes. A 0.4 % lamp ramp for 12 frames moves the mean 0.86 codes during the ramp and 0.31 in any later step. `screen_hall` still flicker is hash 0.309/2.667 and screen 0.310/2.667. `still_idle` draws 40–47 are heavy 20–68 of 54922, walks 0, frame move 57.021 codes. `fill_knockout` is 62.449 codes, error 0.5964 → 0.0825. The bounce gate is full 17670561 against direct 17220302. Knocking out `bounce_in_compose` is full 17220120 against direct 17220302.

Direct that moves by more than one percent of the last marked sample still redraws. A ramp under one percent is compared with that marked sample, not with last frame's already-committed word. Per channel, so a dim channel on a bright pixel cannot drop out. A 0.4 % ramp of a unit sample marks on frame 3 or 4 against the committed value, and never marks when each step replaces that value first.

A cache clear does not reset `still` or the picture. The next held frames were reusing the bounce stored on the clear frame, which is empty (`pack + 1`, so it counts as a finished black bounce), and unmarked pixels kept the previous view. `the_bounce_adds_light_to_the_presented_frame` reads the third draw after that clear: full 17220106 against direct 17220280, so the bounce added nothing. The three frames after a clear (`BOUNCE_FILL_FRAMES`) compose the whole picture and zero the slice, so that third draw walks the cache that those frames traced. The hold starts on the frame after that.

An empty walk packs to 0. Storing `packed + 1` made that 1 look like a colour, and a held camera kept the black bounce until frame 32. A walk stores a colour only when it produced one, when the read was full trust (a real zero), or on the drift pass when nothing is stored yet. A later miss does not replace a stored colour. A block that still has no colour stays on a retry list of 64, because an idle dirty list stopped visiting three path-lit texels and they stayed at exact zero. There is no colour floor and no blur.

Lavapipe, hall 960×540, High, boxes and lamps still, 502 direct-mask samples. Loop index 0 is lighting frame 5. Frames 8–15 (the climb) have median mean 0.256, exact-zero samples 0. Lighting frames 32–39 (loop indices 27–34) have median mean 0.351, variance 0, exact-zero samples 0, and no sample that was lit on indices 12–15 goes to exact zero. The largest median step from loop index 4 on is 0.077. The old hold stepped about 0.08 to 0.35 in one frame. Walking the bounce on every field change through frame 32 made that step 0.029 and moved the `screen_hall` still pair by 3.35 codes (the bar is 1.5). The read is every third frame after frame 8. `screen_hall` still flicker is hash 0.305/2.667 and screen 0.305/2.667. `still_idle` draws 40–47 are heavy 27–67 of 54922, walks 0, frame move 57.140 codes. The 320×180 open fill, after the first draw, steps at most 10.61 codes and reaches about 131 with no cliff at frame 32. Lamp off, a sliding box, and a 0.4% ramp still pass: walks 56922 on the change, box stopped max step 0.31, ramp move 0.96 and late step 0.31.

### Not cut

Direct is still the 4070's candidates 2.5, spatial 2.0, and shade 3.8. Half-res reservoirs (picture error 0.073) and one spatial neighbour (shadow row 0.063) stay rejected. Fewer candidates stays a variance proposal. `REF_ERR` was not loosened.

```
cargo run --release -p genos-stress -- --proof --size 2560x1440 --preset high --frames 90 --still-boxes --freeze-sun --dynamic 0 --profile-passes ./genos-debug/passes.csv
```
