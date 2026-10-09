# GI v2 baseline

The numbers GI v2 is judged against: the current lighting (GI v1) on master at
`0874229d` ("probes tool: values reads each probe's stored light back from the GPU"),
tagged `gi-v2-baseline`. Run on zanven-pc (Linux, RTX 4070) from 06:00 AEDT on
Saturday 10 October 2026, one run at a time under `~/genos-bench.lock`, by
`target/gi2/baseline.sh` in the `genos-engine-gi2` worktree. Raw logs, reports and a
1 s VRAM log stay in that worktree's `target/gi2/baseline/`.

## Reference error (320x180, CPU path trace reference)

`reference.rhai` and `reference_views.rhai`. mean_rel is the mean relative error of
the light; bias is the overall exposure error.

| View | mean_rel | p95_rel | bias |
|---|---|---|---|
| noon (reference.rhai) | 0.055 | 0.133 | -0.026 |
| night (reference.rhai) | 0.058 | 0.172 | +0.049 |
| hall | 0.043 | 0.126 | +0.001 |
| room-a | 0.198 | 0.587 | +0.107 |
| doorway | 0.040 | 0.124 | +0.016 |
| outside | 0.058 | 0.429 | -0.007 |
| corner | 0.272 | 0.398 | +0.272 |

## Flicker (`temporal_flicker.rhai`, 320x180): FAIL

| Pose | frozen: settled | frozen max_delta | moving mean_delta | moving popped_share |
|---|---|---|---|---|
| hall | no (900 frames) | 0.0463 | 0.00018 | 0.246 |
| room-a | yes (751) | 0.0041 | 0.00001 | 0.006 |
| doorway | no (900) | 0.0084 | 0.00041 | 0.309 |
| outside | no (900) | 0.0089 | 0 | 0 |

Gate: worst frozen max_delta under 0.004 (one display code). The baseline fails it at
0.046.

## Settle

- `light_settle.rhai`: settles after a lamp colour change in 92 frames.
- `camera-bench` at 2560x1440: Rest settles in 95 ms, Switch in 83 ms; drag flicker
  mean 0.000, max 0.001.
- `reference_views` poses all report settled before the compare.

## Frame time and fps (RTX 4070)

GPU ms per frame from `GENOS_GPU_TIMES` (`raster+near` is the picture; light is the
probe builds, amortised per frame), 600 frames each.

| Run | picture ms | light ms/frame | VRAM (process) |
|---|---|---|---|
| stress idle 1280x720 | 1.46-1.55 | 0.65-0.72 | 601 MiB |
| stress moving 1280x720 | 2.92-3.14 | 1.44-1.53 | 601 MiB |
| stress idle 2560x1440 | 3.84-3.92 | 1.93-2.03 | 960 MiB |
| stress moving 2560x1440 | 9.15-9.80 | 3.82-5.50 | 960 MiB |
| many_lights_fps (128 lights) | 418 fps, frame 2.39 ms (worst 3.71) | 0.07 | |

Peak VRAM on the card during the whole run (all processes): 2469 MiB.

## Other scripts

Pass: smoke, reference (its bounds), reference_views, entrance_walk,
floor_edge_walk, light_settle, night_to_day, room_switch, spin_black_face,
box_drag_floor, many_lights_fps. Fail: `sun_fast_pop.rhai` (dusk: one frame moves the
mean light by 23 % of the whole change, limit 12 %) and `temporal_flicker.rhai`.
