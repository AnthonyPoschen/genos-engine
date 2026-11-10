# Bounded quarters (2026-11-10)

`bounded_quarters` is off by default. With it on, a due fine texel takes one 16-ray quarter of its 64-ray set per round, and the round stops at `surface_cache.ray_budget`. Budget 0 keeps the base room and does not take the boost. Near texels under the 8-set minimum are listed before far ones, refinement, and a texel already at the cap. A lamp that moved still relights through the settle path, one quarter at a time, inside that budget. A partial quarter is stored and not shown. The mean steps when the fourth quarter lands, on the same strata the whole set would have traced.

The flag is head bit 22 (`4194304`). The stress knob is `lighting_bounded_quarters` ("Bounded quarters"). The argument is `--lighting-bounded-quarters on|off`. `GENOS_LIGHTING_BOUNDED_QUARTERS` applies only when the argument is omitted. The ray budget is the existing `surface_cache.ray_budget` (`--surface-cache-ray-budget`, `GENOS_LIGHTING_SURFACE_CACHE_RAY_BUDGET`).

## Root cause

The set cap limits how many sets a texel stores. It does not limit how many rays a due texel traces. A young texel, a texel the settle path marks due, and an unsettled texel each take `lighting_sc_entries(slot, true)`, up to four consecutive 16-ray entries, a whole 64-ray set. The boost (`LIGHTING_SC_BOOST_ROUNDS` on `LIGHTING_SC_BASE_BATCH`) raises the texel room to `lighting_sc_room()`: 87,808 entries after scouts, gradient checks, the world-cell refresh, and the trickle list. That room times 16 rays is 1,404,928. Through a yaw, new claims plus the texels the settle path marks due outnumber that room, so both cap 8 and cap 64 sit on 1,404,928 for the whole turn. With a moving lamp the settle path lists every cell and texel as a whole set and returns before the priority scheduler, so the cap does not cut that cost either (about 974k rays while lamp 3 moves).

`surface_cache.ray_budget` with the flag off only scales the lists after they are built, in slot order. It does not change a whole set into a quarter, and it does not prefer a near cell that is still under 8 sets. Quarters alone would still fill the room whenever the due texels exceed 87,808 entries. The hard budget is what cuts the spike. The rank is what spends that budget on the near cells first.

## What the flag changes

- Every due fine texel takes one quarter (`m = 1`), including on the settle path. The set still commits on the fourth quarter. A partial sum stays in the cache object and is not the shown colour.
- The dispatch stops at the ray budget. The boost is cleared. Flag off keeps the old room, the boost, and the proportional list cap.
- Admission walks 64 buckets from the top. Near and under the minimum is 48–63 (emptier, then closer). Far and under the minimum is 32–47. Refinement is 16–23. A texel already at the cap while a lamp is moving is 0–7. The crossing bucket cannot crowd out a higher one.
- World cells take the whole budget for the first 24 frames, so the 8 sets are not stored against an empty bounce. A moved lamp then gives that settle list the whole budget. Otherwise cells keep an eighth while the camera moves or a lamp note is hot, and a quiet frame spends nothing on cells.
- A ring entry whose two stored positions differ is a move. The spawn note stores the same point twice, so the still hall does not keep relighting for it. While that move note is up, an at-cap texel is not parked by the quiet return, the hold, or the resolved-history skip. Replacing the texel mean or the cell mean with the new sample for the whole note was tried and reverted: the direct highlights already matched, and the missing light was the bounce, which the settle path carries.

## Method

Lavapipe, 640×360, High, Anthony's pose (eye 14.5, 1.7, 23.5, yaw −0.3554, pitch 0.08), boxes still, lamps still except the lamp run, sun frozen, dynamic 0, time 0.12. Ray counts are `lightingsc` `rays`. Lighting milliseconds are `GPU_MS` `lighting=`, with frame 0–2 and any sample over 4,000 ms left out. Lavapipe times are ratios only. Runs marked paired shared the GPU with a second capture, so their milliseconds are high. Ray counts and pictures are the same either way. `near` / `full` are on-screen fine texels within 6 m, and how many of those are at the set cap. 95% means `full / near` at or above 0.95.

The path reference is `img/hall-path-reference.png` (72 spp). Error is near-weighted relative luminance, sky `(28, 36, 48)` masked, weight `1 / max(distance, 0.5 m)` from the depth view, on 2×2 blocks. The same measure as `scripts/lighting/oracle_metrics.py`. A frame after the yaw or after the lamp has moved is a different view, so those frames are also scored against the flag-off picture at the same mark. The engine frame of a script mark lags the script counter by the startup frames. The turn and the lamp move are the engine frames the script tagged, about frames 93–144, which is the 40-frame motion.

The yaw is 80 frames still, then 0.03 rad per frame for 40 frames, then the turned pose held. Lamp 3 starts at (12.5, 2.6, 21.5) and moves to (14.0, 2.6, 22.5) over those 40 frames. Flag-off budget 0 is the ceiling this note is about. The flag-off rows at 256k, 512k, and 1M are the old proportional list cap, not quarters.

## Still hall

| run | peak rays | mean rays while above 1k | lighting ms peak / mean | frames to 95% near at 8 | near error vs path, f80 | floor / wall luma |
|---|---:|---:|---:|---:|---:|---:|
| off, budget 0 | 1,404,928 | 924,869 | 1,614 / 820 | 27 | 0.0615 | 0.383 / 0.303 |
| off, 256k | 256,048 | 255,000 | 1,506 / 954 | not by 285 (945/2,688) | 0.1113 | |
| off, 512k | 512,048 | 510,079 | 1,521 / 933 | not by 280 (2,051/2,693) | 0.0630 | |
| off, 1M | 1,000,048 | 854,166 | 1,651 / 855 | 32 | 0.0614 | |
| on, 256k | 256,000 | 192,128 | 2,085 / 998, paired | 58 | 0.2614 | 0.323 / 0.178 |
| on, 512k | 512,000 | 267,493 | 2,166 / 870, paired | 57 | 0.1874 | 0.347 / 0.197 |
| on, 1M | 1,000,000 | 544,389 | 1,674 / 820, solo pre-turn | 56 | 0.0601 | 0.382 / 0.298 |

The path reference is floor 0.382 and wall 0.298. Flag off at budget 0 matches it (near error 0.061). Flag on at 1M matches it too (0.060, floor 0.382, wall 0.298) and is idle after the fill. Flag on at 256k and 512k reaches the 95% counter and then idles (0 rays by frame 126), but the 8 sets were stored before the bounce existed, so the wall stays dark (0.178 and 0.197 against 0.298). Flag off at 256k never finishes and never idles: it sits on 256k rays for the whole capture. Flag off at 512k also never reaches 95% at the cap, and its picture is already close to the reference, because those rays are whole sets rather than quarters of a set locked early.

The 1M still numbers are the pre-turn frames of the solo yaw (the hall is still until the turn). Peak during that fill is 1,000,000. No round in the on-runs went more than 48 rays over its budget. 48 rays is three 16-ray entries of dispatch rounding, not the old ceiling.

## Yaw

Window is the tagged turn, about 47 frames.

| run | peak rays | mean rays | lighting ms peak / mean | 95% near at 8 after the turn | near error vs flag-off, frame after the turn |
|---|---:|---:|---:|---:|---:|
| off, budget 0 | 1,404,928 | 1,235,842 | 1,573 / 1,124 | no, plateaus at 0.94 | |
| off, 256k | 256,048 | 252,745 | 1,513 / 1,239 | | |
| off, 512k | 512,032 | 502,705 | 1,547 / 1,295 | | |
| off, 1M | 1,000,000 | 973,097 | 1,674 / 1,343 | | |
| on, 256k | 256,048 | 180,858 | 2,428 / 1,685, paired | touches 0.95, picture still dark | 0.437 |
| on, 512k | 512,048 | 229,297 | 2,373 / 1,651, paired | no, best 0.94 | 0.355 |
| on, 1M | 1,000,048 | 382,162 | 1,860 / 1,402, solo | no, best 0.94 | 0.0376 |

Flag off at budget 0 reproduces the set-cap yaw: peak 1,404,928 for the whole turn. The set-cap note recorded mean 1,274,168 and 1,557 / 1,385 ms. This machine's mean is 1,235,842 and 1,573 / 1,124 ms. The peak is the same room. Flag off with a budget sits on that budget for the whole turn (mean within a few thousand of the cap). Flag on at 1M peaks at 1,000,048, never the room, and the mean through the turn is 382,162 because near cells finish and the list shrinks. The frame after the turn is within 0.038 of the flag-off picture (0.033 by the held pose). The pre-turn frame against the path reference is 0.0601, against 0.0611 flag off.

Neither flag returns the near cells to a steady 95% after the yaw. Flag off drops under 0.95 during the turn and plateaus near 0.94. Flag on at 1M peaks at 0.94 after the dip. The set-cap note's return at frame 225 was not reproduced here. The counter is not the picture: flag on at 256k does touch 0.95, and that frame is 0.44 away from flag off because the sets are dark.

Lavapipe does not show a time win at 1M. The turn's lighting peak is 1,860 ms against 1,573 ms flag off, with 3.2× fewer rays. The ray count is the bound. These times are ratios on lavapipe, and a paired run sits higher still.

Crops of the frame after the yaw, 2× nearest, flag off on the left and 1M on the right: `img/bounded-quarters-yaw-crops.png`, and the pieces `img/bounded-quarters-yaw-floor.png`, `img/bounded-quarters-yaw-back-wall.png`, `img/bounded-quarters-yaw-pillar.png`. The full frame is `img/bounded-quarters-yaw-1m.png`.

## Moving lamp 3

Window is the tagged move, about 46 frames. The note stays up for 120 frames after the move, and the settle path keeps relighting until it expires.

| run | peak rays | mean rays | lighting ms peak / mean | near error vs flag-off at f160 |
|---|---:|---:|---:|---:|
| off, budget 0 | 974,592 | 796,214 | 1,149 / 850 | |
| on, 256k | 256,048 | 251,742 | 1,854 / 1,015, paired | 0.356 |
| on, 512k | 512,032 | 449,910 | 2,200 / 1,314, paired | 0.295 |
| on, 1M | 1,000,048 | 807,015 | 1,945 / 1,042, solo | 0.033 |

The set-cap note recorded 974,560 peak and 799,457 mean. This run is 974,592 and 796,214. Flag off stays full through the move. A few frames after the lamp stops, the settle streak re-arms and flag off touches 1,470,464, which is the texel room plus the 4,096-entry trickle list. It is idle by frame 320 (tail under 100 rays).

Flag on at 1M stays inside the budget (peak 1,000,048, none over). The pre-move frame matches flag off (near error 0.030) and the path reference (0.061). Mid-move, f120, it lags (0.245 against flag off). By f160 it has caught up (0.033), and f240 and f360 stay at 0.034. That lag is the budget spreading the settle list. It is not a permanent pop: the direct highlights match during the move, and the fill arrives before the note expires. Rays stay near 800k while the note is hot, then fall, and the capture is idle by frame 333.

256k and 512k stay on their budgets and the pictures stay dark (0.36 and 0.29 at f160). Those captures end while the note is still hot, so they have not idled yet. The still-hall result already shows they idle on a dark mean once the note ends.

## Recommendation

1,000,000 rays per frame. Below that, the 8-set minimum locks before the bounce exists: the still wall is 0.178 at 256k and 0.197 at 512k, against 0.298 in the path reference and in the flag-off hall. At 1M the still hall matches the reference (near error 0.060), the frame after the yaw stays within 0.04 of flag off, and the lamp catches up by the frame after the move (0.033) with no leftover pop. The turn no longer sits on 1,404,928. Budget 0 with the flag on is the base room, not this setting. The flag stays off until that budget is chosen.
