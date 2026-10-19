# S78Y: Quiet probe filter (rounds kept when their inputs did not change)

- **Date:** 2026-10-12
- **Result:** pushed (rough numbers under gaming load; idle-GPU A/B queued)
- **Commit / patch:** this commit

## What was implemented
The spatial filter's three rounds each have their own output (graw -> table 1 -> table 0 -> SH), so a round's output from last frame is still valid when none of its inputs changed. Per probe and round, the filter checks its 3 x 3 neighbourhood at that round's step (self included): round 0 re-runs if any of those probes' gather was redone this frame (the gather now stamps gd.x when it writes raw cells) or its depth from the eye differs from the value stored at last frame's last round (the depth weights; -1 for inactive probes); rounds 1-2 re-run if a neighbour's previous round re-ran (stamps gd.z / gd.w). Otherwise the round returns before reading anything else. The check reads input stamps, not a blanket "nothing changed" flag. On only with the gather hold (S78R3) and outside world mode 2.

## Numbers (rough, measured under gaming load; RTX 4070, stress, 1080p, alternating runs)
| | S78R3 | S78Y |
|---|---|---|
| filter (3 dispatches) | 1.53 / 1.58 ms | 0.475 / 0.490 ms |
| round 0 / 1 / 2 | 0.89 / 0.39 / 0.25 | 0.16 / 0.22 / 0.105 |
| still lighting total | 8.54 / 9.08 ms | 7.05 / 7.30 ms |
Smoke a 0.348; pop 0.098 % / 0.139 %.

## Screenshots
shad-S78Y/b.png viewed (moving box; mean abs diff vs S78R3 0.71/255).

## Also tried tonight (not pushed): a-trous dirty pages + noise-aware level 0
Per-64-slot page stamps per level, so a level's dispatch skips clean workgroups, plus a level-0 rewrite threshold of 2 SE of the texel. No gain on the a-trous dispatches (each still 0.25-0.6 ms): ~740 texel writes a frame, each scattering 5 x 5 at steps up to 16, dirty nearly every page. Patch g78/S78X-atrous-pages-noise.patch.
