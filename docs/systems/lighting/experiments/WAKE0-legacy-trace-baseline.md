# WAKE0: legacy probe traces, before ring wakes

- **Date:** 2026-10-13
- **Result:** measured (legacy path, unchanged)
- **Commit / patch:** this commit

## Idea
Record how many screen probes the current scheduler traces, still and moving, before ring wakes replace the always-on check ray.

## What the scheduler does today
Place marks a stratum held (`w = 2`) when the probe still stands where that stratum was traced and no hold-fail window is open. A held probe traces one check ray (`lighting_hold_check_k`) plus any ray whose segment crosses a geometry-change sphere. A probe that is not held traces every ray of the current stratum. Lamps are not in the change list: kept hits are shaded again on the stratum and halfway schedule, and a check ray does not rewrite stored light for surface-cache noise (S78U). After the scene has been still for the settle window, a frozen frame skips the probe passes entirely.

The new counters (`gstats` words 140–151, `lightingwake` when `GENOS_LIGHTING_DEBUG` is set, `Renderer::lighting_wake_stats`) count probes and rays that reach the primary probe-ray trace. Fills and change-crossing retraces are extra rays, not extra probes. Ring, split, wake, and verification counters stay at zero on this path.

## Numbers
Lavapipe, proof window 320×180, shipped hall (`examples/camera/scene.rhai`), opening camera, preset high. Tile is `height/45` = 4 px, so the grid is 80×45 = 3600 slots. The still sample is the 12th frame (strata held). The moving sample is the fourth frame after that, yaw stepped by 0.04, 0.08, 0.12, then 0.16 rad from the opening yaw.

| | probes placed | probes traced | rays traced | rays per traced probe |
|---|---:|---:|---:|---:|
| still, held | 1862 | 1862 | 1862 | 1 |
| moving camera | 2089 | 2089 | 133696 | 64 |

Still rays held (gather) were 953344; moving, 789790. Rings tested, groups split, probes woken, verification hits and misses were 0 on both frames.

A 1920×1080 picture uses the 16 px tile: 120×68 = 8160 slots. The same rules trace one check ray per placed probe while the camera is still, and 64 rays per placed probe when the hold drops. That 1080p count was not run here.

## Outcome
The legacy still frame already traces one ray per placed probe, not the full stratum. Ring wakes have to beat that check ray, not the 64-ray stratum, on a still picture. A moving camera still pays the full stratum.
