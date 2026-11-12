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

`Renderer::read_cache_cells` is the cell table. `Renderer::read_cache_round` is live, claimed, the scheduler threshold, rays, due, waiting, and scouts. `Renderer::read_direct_samples` is the scratch reservoir and the visibility mean for up to eight pixels. `Renderer::lighting_restir_ms` is the five-pass split when `GENOS_GPU_TIMES` is on. Replicated state: none (the copies stay on the machine that shades). Authority: that machine. Prediction / rollback: none. Determinism: the same pose and the same frame count produce the same cells. Headless server: a server that does not shade leaves them uncomputed.

Tests: `nearby_offscreen_world_cells_hold_samples_before_they_are_seen` and `shadow_and_lit_pixels_keep_a_visibility_mean` in `examples/stress/tests/offscreen_cells.rs`.
