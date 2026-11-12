# Screen-tile fill and neighbour sharing for the world cache (design, for review)

Status: **Proposed** (11 Nov 2026). Design only: no engine code, no setting, no shader changes in this PR. Nothing here is accepted until Anthony Poschen reviews it. On acceptance this becomes ADR 0044 (0042 and 0043 are taken on open branches, and 0019, 0040 and 0043 are already duplicated; check for a clash before the number is claimed).

Origin: Anthony, 11 Nov 2026: "if we did a sample per 8x8 then we could also handle neighbour sharing and every cell improves together". The weekly research note (`research/2026-11-11-weekly.md`, PR #264, not yet on this branch) found the Lumen World Cache fill as proposal (A).

Facts below were checked against: `lighting-v3` 34cc80cb (base of this PR), PR #252 head c34633f1 (the lighting integration that this design builds on: SH-L1 cells, texel detail, screen probes removed), PR #265 head 96588f01 (`bounded_quarters`, default off), and the 4070 runs of 11 Nov (`pc-cap8-1111`, `pc-quarters-1111`). Anything marked **est** is not measured. Anything marked **model** is a calculation from the two small scripts in the appendix, not a render.

## 1. Summary

Today every due cache entry traces its own rays from its own centre, and a scheduler decides which entries are due by scanning the whole table. The ray count is therefore a function of how many entries changed, and it hits a ceiling: 87,808 texel entries x 16 rays = 1,404,928 rays a frame.

The proposal turns the cost around. A fixed number of **origins** is chosen from the screen each frame, one pixel per 8x8 tile (57,600 at 2560x1440) with a need-weighted choice inside the tile. Each origin traces a path from the pixel's own surface point, with the same hit shading and the same cache read at the hit that cache rays use today. The result is deposited into the cell and texel under the pixel, and, when a strict set of gates pass, into the other cells of the same smooth surface in the tile (neighbour sharing). Cost follows screen tiles, not changed entries.

This is the Lumen World Cache fill (UE 5.9 Preview, 9 Nov 2026) with three departures that our own history forces:

1. **The pixel choice is need-weighted, not uniform.** A uniform pick starves distant floors (model, section 5.2: a floor cell at 20 m gets 0.32 paths a frame, so 128 paths take about 400 frames). Lumen avoids this with cells that grow with distance. Anthony removed distance-keyed sizes on 12 Oct (S57: rings and coarse grain while walking), so our cells stay one size.
2. **Sharing is a prior that expires, not a permanent filter.** S72B (neighbour reuse of hits without visibility) was reverted for correlated low-frequency blotches. Sharing here is gated by geometry, by hit distance, and by the receiving cell's own sample count, and it stores no per-cell sample (no reservoirs), so errors decorrelate frame to frame.
3. **A budget is a ceiling, and a settled scene spends almost nothing.** Anthony's rule (E19, rubric row 8) stands. A small fixed share of the picks is still spread out at random so a quiet area that should not be quiet is found (rule: always spend some budget on spread-out checks).

The expected win is not "fewer rays" by itself. RAYCEIL (19 Oct) cut moving rays by half and made the frame 1.35 ms slower, and the settled 4070 hall spends 15.2 ms in lighting with 0 cache rays. The win has to show up in milliseconds, in the 1.40M spike, and in the lines of scheduler that are deleted. If it does not show up in milliseconds it is not shipped.

## 2. Problem (measured)

4070, 2560x1440, High, hall checkpoint, same scripts and pose for every row. Sources: `/workspace/pc-cap8-1111/SUMMARY.md` (cap 8, #262) and `/workspace/pc-quarters-1111/SUMMARY.md` (#265).

| Situation | Flag off (cap 8) | #265 at 1M | What it says |
|---|---|---|---|
| Initial fill, rays peak / mean (frames 0-200) | 1,404,928 / 1,001,663 | 1,000,000 / 749,266 | The ceiling is the initial fill. About 200 M rays in the first 200 frames |
| Initial fill fps, median | 35 | 42.5 | A budget helps here |
| First frame under 1k rays / lasting idle | 116 / 286 | 193 / 286 | Budget stretches the fill, the end is the same |
| Lamp 3 move then hold, rays peak in the hold | **1,470,464** | 1,000,048 | The second ceiling: the settle path relists every cell and texel |
| Lamp move fps (frames 81-120) | 35.5 | 39.0 | |
| Settled, fps / lighting ms / rays | 65 / 15.17 / 0 | 67 / 14.74 / 0 | The settled floor is not a ray problem |
| Yaw from a fully settled hall, rays peak | 130,048 | 145,376 | **Not** at the ceiling. The 1.40M yaw peak seen on lavapipe was an 80-frame pre-turn with the cache still filling |
| Yaw fps | 51.8 | 48.3 | #265 is slightly slower here |

What the data says about the motivation:

- The ceiling appears in two places: **initial fill (and any camera cut that discards the cache)** and the **spike after a lamp change**. It does not appear in a yaw of a settled scene. This design is motivated by those two cases and must not regress the settled yaw.
- #265 at 1M keeps quality on the still hall (near error 0.060 against the path reference, 0 pixels at 20 codes or more against flag off at frame 640). It lags during a lamp move: about 6% of near pixels are 20 codes or more off flag off mid-move and at the frame the lamp stops (4070). 256k and 512k leave the wall dark (lavapipe: 0.178 and 0.197 against 0.298) because the 8-set minimum locks before the bounce exists.
- Ray count is not millisecond count. Lowering rays 3.2x on lavapipe did not lower time. On the 4070 the settled hall costs 15.17 ms of lighting with zero rays (ReSTIR direct, compose, hit light, the rest). Section 12 states what this design can and cannot move.

Why the ceiling exists (from the #265 note): `lighting_sc_room()` is the batch minus scouts, gradient checks, the world-cell refresh and the trickle list = 87,808 entries. A due fine texel takes up to four 16-ray entries (a whole 64-ray set). Under a lamp change, `lighting_lamp_settling()` lists every cell and texel as a whole set and returns before the priority scheduler. So work is driven by *how many entries are due*, and the cap on sets (cap 8, #262) does not cut it.

## 3. Goals and non-goals

Goals:

- Cost per frame bounded by the screen, independent of how many cache entries changed. Hard ray ceiling at or under #265's 1M, typical far lower.
- Fill speed for the cases in section 2: first 200 frames, camera cuts, lamp changes. Near-camera error first (weight `1 / max(distance, 0.5 m)`, the metric of `scripts/lighting/oracle_metrics.py`).
- Every cell near a fill improves together (Anthony), without blur and without light leaking through walls or across edges.
- One estimate, one data flow: **cells -> texels -> pixels, never back** (ADR 0039). The fill adds a writer; it does not add a reader of texels.
- Delete the superseded scheduler once the replacement passes (section 10).
- Hold the frozen gates (blotch 0.014, cell energy 0.9, linger 0.05 at 120 frames, lag 0.9 at 300, turn 0.403, contact/corner/thin-wall no worse than no-skip). No threshold moves.
- No look hacks, no blur to hide grain, no flicker or pops, same picture at the same sample count on every GPU.

Non-goals:

- Not a fix for the settled-frame floor (15 ms lighting on the 4070). The 500 fps (basic, moving light) and 100 fps (stress) targets are owned by direct light (ReSTIR), compose, and the denoiser. This design reports where it stands against them and claims nothing there.
- Not a change to direct light. Direct stays per pixel (t378 rule; ADR 0039).
- Not distance-keyed cell sizes (S57) and not a baked or trained field (ADR 0012).
- Not the off-screen pre-fill S-curve (S73A/B, parked until there is spare performance). Section 5.5 only defines the hook that pre-fill would use.
- Not Low. Low keeps probes-era behaviour until GTX 1060 numbers exist (ADR 0039 decision 2). Section 12 sketches the Low path.
- Not new look: a stage whose picture differs from the path reference beyond the frozen gates does not ship.

## 4. What exists, what the fill changes

```
today (c34633f1):
  compact pass scans live table ──> due lists (young, settle, WRC refresh, trickle, scouts, checks)
        │ priority histogram + threshold + boost, settle turns
        v
  trace pass: each listed entry traces 16-ray entries from its own centre
        v
  cache pass: MSME update per entry (texel mean, cell SH-L1) ──> compose reads (cells -> texels -> pixels)

proposed:
  G-buffer (position, normal, fine slot under each pixel: already stored by screen stage 0)
        │
  pick pass:   one workgroup per 8x8 tile; need-weighted reservoir over 64 pixels; m picks
        v
  path pass:   per pick, a path from the pixel's surface point (same hit shade, same cache read at the hit)
        v
  deposit:     fixed-point atomics into a per-frame deposit buffer
               ├─ origin cell (SH-L1) and origin texel
               └─ shared cells of the same smooth surface in the tile (gated, section 7)
        v
  resolve pass: one writer per touched entry folds the deposit into the existing MSME step (section 8)
        v
  compose reads (unchanged)
```

Kept as they are: the SH-L1 cell store and its true-normal read, texel detail seeded from cells, the stage-4 ray cut, MSME, lamp picking at hits, the world-cell key and hash table, the a-trous layer 2 (never feeds back), A-SVGF same-seed checks (they measure change, a different job), the lamp-change ring (`lighting_lamp_delta`).

## 5. Fill scheduling

### 5.1 Unit, budget and defaults

- **Tile:** `fill_tile` pixels square. High 8, Medium 16 (est), Low n/a (section 12). At 2560x1440 an 8x8 tile grid is 320x180 = 57,600 tiles; 16x16 is 14,400.
- **Base picks:** one per tile. **Max picks:** at most 4 per tile and at most `4 x tiles` in total (230,400 at 1440p). The pick count rises above 1 only where need is high and other tiles need nothing (water-filling, section 5.3).
- **Spread-out floor:** `tiles / 16` picks (3,600 at 1440p) are chosen uniformly at random from tiles regardless of need, every frame. This is the always-on check budget (Anthony's rule) and the guard against a quiet-but-wrong cell.
- **Ray ceiling:** `fill_ray_ceiling`, default 1,000,000 cache rays a frame, the value #265 showed holds quality. It is a ceiling, not a target. The pick pass cannot exceed it by construction: picks are assigned until the ceiling is reached, nearest and neediest first.
- **Directions:** each pick traces one path with one first-bounce direction. The direction index is the receiving cell's stratum: `atomicAdd` on the cell's deposit counter returns an index `i`, and the direction is stratum `(own_count + i) mod 64` of the existing 64-ray hemisphere set, jittered by a hash of (cell key, set). So successive samples into one cell walk the same stratified sets the cache uses today (rule: fixed ray patterns, S72O decorrelation kept). A path never uses the frame number as its only source of variation.
- **Determinism:** pick order and jitter are hashes of (tile, frame, cell key). No `rand()`, no thread-order dependence. The same inputs give the same picture on lavapipe, the 4070 and macOS (rule: the sample is `f(position, count)`). The deposit sum itself is order-independent because it is integer fixed point (section 8).

Counting unit. The existing counters (`lightingsc` `rays`, gi-metrics `rays`) count cache rays. The fill reports the same counter so every comparison with #262 and #265 is like for like, plus `fill_paths` and `fill_shadow_rays`. Lumen's "58k paths" is about 58k x 3 bounces x (1 + 1 shadow) = 350k segments. Our default is 1 bounce vertex per path (the cache read at the hit carries the later bounces, as `lighting_bounce_ok` does today), so 57.6k paths is about 115k segments. Stage F5 tests longer paths.

### 5.2 Which pixel: why need-weighted (model)

A tile contributes samples to cells in proportion to the screen area they cover. For a 25 cm cell at 2560x1440, vertical fov 60 degrees (the engine's), eye height 1.7 m, one uniform pick per tile (paths per frame = pixels covered / 64):

| Distance r (m) | Wall px per cell | Wall paths/frame | Floor px per cell | Floor paths/frame | Frames to 128 paths, wall | floor |
|---:|---:|---:|---:|---:|---:|---:|
| 2 | 24,300 | 380 | 20,655 | 323 | 0.3 | 0.4 |
| 4 | 6,075 | 95 | 2,582 | 40 | 1.3 | 3.2 |
| 6 | 2,700 | 42 | 765 | 12 | 3.0 | 10.7 |
| 10 | 972 | 15.2 | 165 | 2.6 | 8.4 | 50 |
| 20 | 243 | 3.8 | 21 | 0.32 | 34 | 397 |
| 40 | 61 | 0.95 | 3 | 0.04 | 135 | 3,173 |
| 80 | 15 | 0.24 | 0.2 | 0.003 | 540 | 25,383 |

(**model**, appendix A. 128 paths is a quarter of a cap-8 texel's 512 rays; the real target is the cap, section 8.)

Reading: near surfaces are over-sampled by two to three orders of magnitude and far floors starve. A uniform pick reproduces exactly the reported grain on flat far surfaces (Anthony 11 Nov 01:02) and would make it worse. Lumen's answer (cells that grow with distance, 25.6 cm at 10 m) is excluded by S57. Our answer is to spend the budget where the cells are short of samples:

`need(p) = prox(p) x max(under(p), stale(p), disagree(p))`

- `prox(p) = 1 / max(d(p), 0.5 m)`: the same weight as the error metric. Near errors matter most, so near cells win ties and are never starved by far work.
- `under(p) = max(0, 1 - own_count(texel under p) / fill_target)`. `fill_target` is the set cap in rays (cap 8 sets x 64 = 512 on High, the #262 value). A converged cell has `under = 0`.
- `stale(p)`: the cell's last update is older than a lamp-change ring entry that reaches it (section 9), or an object or sun change stamp. This is `lighting_lamp_delta(p, updated) >= 0`, a single read, no list.
- `disagree(p)`: an A-SVGF same-seed check ray on the cell disagreed beyond 2 sigma (the existing check machinery, unchanged).

Each workgroup owns one tile and holds the 64 `need` values in shared memory (`lighting_screen.comp` already does a 64-lane workgroup reduction per block). It draws `m` pixels without replacement by weighted reservoir sampling. A tile whose pixels all have `need = 0` draws none except through the spread-out floor.

Consequence for far cells: they get picks because they have need, and the budget is large once near cells converge. A far floor cell needs many origins, not many distinct pixels: several picks of the same pixel with different strata are valid. So far starvation is a budget-allocation problem, not a tile-size problem, and a 4x4 tile is not needed (section 17, question 2).

### 5.3 Budget allocation (water-filling, no list)

1. Every tile computes `need_tile = max need(p)` and its draw weight.
2. A global histogram of `need_tile` in 64 log buckets (the same shape as the existing scheduler histogram) is built with workgroup atomics.
3. One thread reads the histogram and chooses a threshold bucket so that `sum(picks) <= min(4 x tiles, fill_ray_ceiling / rays_per_pick)`. This is `LIGHTING_SC_THR_AT`'s job today, but over 57,600 tiles instead of the whole live table, and with no boost state machine.
4. Tiles above the threshold get `m = 1..4` by bucket, tiles at the threshold are admitted by hash, tiles below get none.

Two properties to test: (a) the threshold never lets the total exceed the ceiling (the #265 invariant: no round more than 48 rays over), (b) a crossing bucket cannot crowd out a higher one (the #265 admission rule, kept).

### 5.4 Near-camera priority

Near wins in three places: `prox` in the weight, the threshold taking buckets from the top, and the first 24 frames after a cache clear giving the whole ceiling to the nearest 6 m (`LIGHTING_WRC_NEAR`), as #265 does for world cells. Near cells never wait behind far work, and far work never waits behind nothing: the ceiling is shared by weight, not partitioned.

### 5.5 Off-screen and behind the camera

A pure screen fill leaves nothing behind the camera. Lumen accepts this (its cells expire after 64 unvisited frames). We cannot: a turn or walking backwards must not show an unfilled wall (Anthony 12 Oct; S73L counts unfilled pixels). Today scouts cover it: 256 entries x 16 rays a round, half over the sphere and half ahead of the motion, a quarter from up to 2 m beside the eye (E17). The proposal keeps the *idea* and removes the separate machinery:

- **Virtual tiles.** A coarse grid of directions around the eye (an octahedral map, 64x64 = 4,096 directions, a guard band that is the frustum widened by 40 degrees plus the whole sphere at low rate). For each virtual tile a primary ray from the eye (or from a point up to 2 m beside it) finds a surface point. That point plays the part of "the pixel" in sections 5.2 to 6: it has a position, a normal, a texel and a cell, so `need` applies unchanged. No G-buffer is needed for them.
- **Priority.** They run on whatever the on-screen need leaves (water-filling puts them last), with a floor of `tiles / 8` picks while the camera moves or turns and `tiles / 64` when still. This is the "lower priority, spare capacity" rule Anthony gave for pre-fill (S73A/S73B: "off-screen low-priority areas can only be filled once there is spare capacity").
- **Seeding.** An off-screen cell with samples is read exactly like an on-screen one when revealed, so there is no pop. A cell revealed with none starts from the seed the cells already use (low-confidence, never an own sample).

Whether virtual tiles should replace the scouts or only the on-screen fill in the first stages is open question 6 (answer: replace in F3, keep scouts until F3 passes the turn gate).

## 6. The path and what it deposits

A pick at pixel `p` (surface point `x`, normal `n`):

1. Trace one ray from `x` along the stratum direction. The ray, the hit shade (lamp pick, sun, emissive, the shadow cell cache), the cache read at the hit (`lighting_bounce_ok`) and the sky on a miss are the existing functions, called from a new, small pipeline. Nothing is re-derived.
2. The sample is `L(w)`, a radiance, with the cosine weighted hemisphere estimator the cache uses (`2 pi / C` times the cosine-weighted sum).
3. Origin deposit: add `L` into the entry of the cell under `x` (SH-L1: the same `add_ray` as `lighting_sh.rs`) and into the texel under `x` (its own mean, one ray).
4. Shared deposit (section 7): add the same hit, re-weighted, into other cells.

Why origin = the pixel's point and not the cell centre: the cell centre is at most 12.5 cm off for a texel and 17 cm for a cell diagonal, which is the parallax the sharing math bounds anyway, and it avoids a lookup of the cell centre per pick. Texel rays today start at the entry's own position; the fill starts at the surface point actually seen. The difference is below the cell size and the first-hit threshold of section 7 keeps it out of the contact range.

One-way rule check: paths read cells (at hits) and never read texels or pixels. Deposits go to cells and to the texel under the pixel. Texels never write cells. The texel deposit is an own sample, not a copy of the cell, so it cannot loop (the same argument as ADR 0039's texel ray cut).

Path vertices beyond the first (stage F5, optional): a path continues for up to 3 bounces, adds `emissive + one shadow-rayed light sample` at each vertex, ends by reading the cache, and propagates the radiance back so each vertex's cell gets the radiance of the rest of the path (SHaRC-style; Epic does not document the write rule, so this is an assumption to be tested, not a claim about Lumen). It is only worth building if the lamp-move bounce latency (section 9) needs it.

## 7. Neighbour sharing

### 7.1 What is shared and what is not

A sample is a first hit `(h, n_h)` with radiance `L` at distance `t` from origin `x`. For a Lambertian hit, the outgoing radiance toward any receiver is the same `L` (diffuse radiance is direction independent). So the sample can be **reconnected** to a neighbour cell at `x_j`: the hit stays fixed, the direction from the neighbour changes, and only geometry changes:

```
w_j = (cos(theta_j) / cos(theta_o)) * |J|,   |J| = (cos(phi_j) / d_j^2) / (cos(phi_o) / t^2)
E_j contribution = L * w_j    (w is the neighbour's contribution relative to what the origin's own sample would add, so it holds for the cache's uniform-hemisphere estimator and for cosine sampling alike)
```

`theta` is the angle at the receiver, `phi` the angle at the hit. This is the reconnection shift of ReSTIR GI (Ouyang et al., HPG 2021), with its Jacobian, minus the resampling.

What the reconnection cannot see: visibility from the neighbour to `h`. That is where light leaks. The gates (section 7.2) keep it small, and a measured near-field test (section 14) decides whether one visibility ray per shared cell is needed there.

### 7.2 Gates (all must pass, otherwise the cell gets nothing)

1. **Same smooth surface.** The receiver cell's texel is on screen in the same tile (or the 3x3 tiles around it), and the plane test of SVGF passes against the pick: `|dot(n, x_j - x)| < 0.02 x depth` (the product test of ReSTIR DI's history, relative depth 0.02) and `dot(n_j, n) >= 0.99`, and `n_j` is on the same cell axis. A surface the picked pixel cannot see across a depth discontinuity never passes, so a wall's two faces and an edge never share. On-screen adjacency is the leak guard: the fill only shares between things the camera sees as continuous.
2. **First-hit distance.** `t >= t_min`, default 2 m (8 cell widths). The model (appendix B, 400,000 samples per row) gives the reweighting spread for a receiver 12.5 to 35 cm from the origin:

   | first hit t (m) | median w | p5 | p95 | p99 | share of samples with w > 2 |
   |---:|---:|---:|---:|---:|---:|
   | 0.25 | 0.22 | 0.00 | 6.68 | 36.0 | 12.6% |
   | 0.5 | 0.60 | 0.00 | 5.40 | 15.0 | 17.8% |
   | 1 | 0.85 | 0.00 | 2.62 | 4.67 | 10.3% |
   | 2 | 0.95 | 0.21 | 1.66 | 2.50 | 2.3% |
   | 4 | 0.98 | 0.40 | 1.29 | 1.65 | 0.3% |
   | 8 | 0.99 | 0.50 | 1.14 | 1.29 | 0% |

   Under 1 m the weights are wide and heavy-tailed (contacts, corners, the range where texel detail is what matters). At 2 m and beyond the median is 0.95 and the p95 is 1.66 (the low tail is receivers at a grazing angle to the hit, which the 0.05 cosine clamp keeps finite). So `t_min = 2 m` and the weight is clamped to `w <= 2`. Samples with a nearer hit are deposited into the origin cell only.
3. **Own-count gate (the expiry).** Shared weight into a cell scales by `max(0, 1 - own_count / share_cap)`, `share_cap = 32` own paths by default. A cell with its own samples stops accepting borrowed ones. Sharing is a prior for cold cells (initial fill, reveal, lamp change), not a permanent filter. This is also why no blur is added: a converged cell's value is its own rays.
4. **Firefly bound.** The shared contribution is clamped to the receiving cell's running upper estimate (`0.1 + short mean + sigma x deviation`, the existing MSME clamp), so a shared bright hit cannot burn into many cells at once.
5. **Change-aware.** A cell with a lamp-change stamp newer than its mean accepts shared samples only from paths traced after the change (the path's frame is after the ring entry). Old light cannot be shared into a cell that is draining.

Shared samples carry a tag: they add to the cell's accumulator with the weight above and count toward `shared_count`, never toward `own_count`. The displayed value follows the same trust blend as a seed: `own_count` gates what counts as own. So a shared sample never makes a cell look converged, which keeps the 8-set minimum honest (the #265 failure mode: the minimum locking before the bounce exists).

### 7.3 Which receivers

Stage F4a: the cells and texels under the other pixels of the same 8x8 tile that pass the gates. At 3 m one tile spans about 2 cm, so it is inside one cell and sharing is moot (near cells are oversampled anyway, table in 5.2); at 50 m a tile spans about 40 cm, one to four cells. So tile-local sharing helps exactly the starved distance range, which is where it is needed. The cost is one atomic add per extra cell (at most 3 more), no extra rays.

Stage F4b (only if F4a shows a gain): the 8 same-plane neighbour cells of the origin cell, if on screen and passing gate 1 against the pick. This spreads one sample over a 3x3 cell patch (75 cm square) and is the "every cell improves together" case for the mid range.

### 7.4 Why this is not a blur and not S72B again

- S72B stored reservoirs per texel without visibility, so one persistent sample coloured many frames and many cells with the same error: low-frequency blotches. Here nothing persists: a deposit goes straight into a running mean, the next frame's picks, directions and strata differ, so the shared error decorrelates in time at the same rate as independent error.
- Sharing weights go to zero with `own_count`, so the steady picture is made of each cell's own rays plus the existing true-normal trilinear read. The a-trous layer 2 stays display-only.
- Correlation is a measured gate, not an argument: the room-blotch metric (frozen 0.014) and the cache-only blotch view must not move up, and a new spatial-correlation metric (section 14) must stay within noise of sharing off.

### 7.5 ReSTIR analogy (what we copy, what we leave)

| ReSTIR (DI and GI) | This design |
|---|---|
| Candidate generation per pixel (RIS) | Picks per tile; paths from the pixel (no light candidates here: lamp pick at the hit is unchanged) |
| Temporal reuse with an `M` clamp (20x) | Running mean to a set cap (cap 8 sets); same clamp idea, already in MSME |
| Spatial reuse from a neighbour disk with depth/normal tests (0.02, 0.9) | Section 7.2 gate 1, same constants |
| Reconnection shift with Jacobian and a distance threshold | Section 7.1 weight, `t_min = 2 m`, clamp 2 |
| Visibility reuse, one ray to make it unbiased | Not in F4a. A measured near-field option, section 14 |
| Pairwise MIS / balance heuristic for unbiased combination | Not used: there is no resampling and no per-cell sample, so there is no combination step. Weights are self-normalised: `E_j = sum(w L) / sum(w)`. Bias is bounded by the gates and measured, not argued |
| Reservoir per pixel | **Not built.** S72B reverted; memory; no reservoirs means no persistent correlation |

Self-normalised weights are a departure from unbiased MIS. It is recorded under "Departures" below and graded against a brute-force oracle (section 14), the way the ReSTIR DI stages were (CPU reference first).

## 8. Temporal averaging, deposits and eviction

Deposit buffer. Each touched cache entry owns one deposit record in a per-frame buffer: RGB (3), SH-L1 (12) and a weight sum (1) as 32-bit fixed point, 16 words = 64 B, plus own and shared counts. `atomicAdd` on integers is order independent, so a frame's deposits are the same sum on every GPU and every thread order. (This is the lesson of #239: a plain multi-writer there made the luma spread between runs.) Records are assigned by hashing the entry's slot into a deposit table sized `4 x tiles` (230,400 entries, 14.7 MB **est** at 1440p).

Resolve pass. One thread per touched record is the **only writer** of the entry: it converts the deposit to a mean and an `n`-ray weight, calls the existing MSME step with that partial set (the #265 partial-quarter sum machinery already stores partial sums and commits on a full set), and writes cell SH, texel mean and counts. No other pass writes the cache entry in the same frame. This is the single-writer rule that the settle/hold bugs of #239 and #252 argued for.

Averaging window. The cap stays what the sweep showed: 8 sets (512 rays) on High, caps 8 to 32 within the path reference's noise, cap 4 blotches. Lumen's 16-frame cap is the same order. Beyond the cap the mean keeps integrating at the MSME rate, a still texel does not freeze (the #239 rule). Shared samples enter the same mean with their weight, and the mean forgets dark early samples exactly as it does today (ADR 0039 stage 1).

Eviction. Unchanged in the first stages: an entry expires after `LIGHTING_CACHE_LIFE` = 1800 frames without a read, unless its pixel is on screen. Lumen's 64 frames is a memory choice we do not need to copy. The tile fill makes eviction cheaper to reason about: a cell that is read is a cell a pick can reach, so its stamp refreshes by reading. ADR 0036's measured occupancy (peak live 54k basic, 153k stress, 407k sun x100 at 1080p) says the table is far from full, so eviction tightening waits for per-preset sizing and is not part of this design.

## 9. Moving lamps without a full relist

Today a lamp change puts the whole cache into settling: every cell and texel is due as a whole set each round for `LIGHTING_WRC_CHANGE_FRAMES` = 120 frames, spread over turns. That is the 1,470,464 spike.

In the proposal nothing is listed:

- The ring (`LIGHTING_WRC_RING`, 16 entries x 10 words: old and new place, range, brightest channel, frame, keep) is written once a round as now (`lighting_wrc_note_lamps`).
- A pick evaluates `lighting_lamp_delta(cell position, cell.updated)` for the pixel's own cell: one ring walk of at most 16 entries. If a lamp that reaches the cell moved after the cell's last update, `stale = 1`, the cell's mean drops to `LIGHTING_WRC_CHANGE_KEEP` = 4 samples at the resolve (as today, per cell and on first touch, not by a sweep), and the pick weight is high. Cells not on screen are not touched; they are marked stale by the same test when revealed (their `updated` is older than the ring entry), so a reveal after a lamp change shows no old light.
- A lamp that went out or moved takes the new set whole (`keep = 0`), a dimmed lamp keeps its history (`keep = 1`), as the ring already says.
- The display follows the A-SVGF lambda (#259, #261): a changed pixel's shown value catches up by lambda, not by the cache sweep. The tile fill supplies the samples; the lambda path decides how fast the display moves to them. The two are separate and the design does not change #259/#261.

Bounce light is the slow part: a lamp's bounce reaches surfaces it never lit directly, and with a cache read at the first hit each cache generation adds one bounce. Today the settle path relights everything for 120 frames; here the stamped cells are re-picked by need as far as the budget allows, nearest first. This is the known risk (section 16, risk 3): the budget lag #265 shows (about 6% of near pixels 20 codes or more off flag off mid-move on the 4070) can come back. Mitigations in order, each its own measured stage: need-weighted near priority (F3), path vertices that carry two or three bounces in one pass (F5), and `fill_ray_ceiling` raised only while a stamped near cell is under target.

First action for stage F0 on this risk: find why #265's mid-move lag exists at 1M. The direct term is per pixel; the missing 6% of near pixels is bounce or stored light on the red box. Without that cause, nothing in this section is more than a hypothesis.

## 10. What it replaces, what is deleted

Nothing is deleted until its replacement passes its gates (ADR 0039 rule). Each row is its own PR.

| Superseded code | Replaced by | Deleted at | Condition |
|---|---|---|---|
| Per-texel due lists, young list, priority histogram and threshold (`LIGHTING_SC_HIST_AT`, `THR_AT`, S58A), the 30-round boost (`LIGHTING_SC_BOOST_ROUNDS`, S66A) | Pick pass, water-filling threshold (5.3) | F6 | Fill curve, still, lamp and turn gates pass on the 4070 |
| Settle path and settle turns (`lighting_lamp_settling`, `lighting_settle_turns`, `LIGHTING_SETTLE_*`, the every-cell-due compact branch) | Stale stamp in `need` (9) | F6 | Lamp 3 move and the `settle_flicker`, `settle_dim`, `wrc_lamps_off` scripts pass; the 1.47M hold spike gone |
| WRC refresh change and background lists (`LIGHTING_WRC_REFRESH`, `CHG_AT`, `BG_AT`) | `stale` and `under` terms; spread-out floor | F6 | Same |
| Scouts (`LIGHTING_WRC_SCOUT_ENTRIES`, `lighting_wrc_scout`) | Virtual tiles (5.5) | F3 | Turn, spin, walk-back gates and `unfilled` counter (S73L) no worse |
| Trickle list (`LIGHTING_SC_TRICKLE`) | Spread-out floor | F6 | Linger and lag gates |
| #265 `bounded_quarters` flag, bit 22, `lighting_bounded_quarters` knob and the quarter admission code | The pick pass owns the ceiling. The partial-quarter sum storage is kept for the resolve pass | F6 | If the tile fill fails its gates, #265 stays as the shipped fallback and nothing here is deleted |
| `surface_cache.ray_budget` proportional list scaling (the old 256k/512k/1M behaviour that never idles) | `fill_ray_ceiling` | F6 | |
| `compact` passes that walk the live table every frame to build lists | Compact keeps only freeing unread entries | F6 | Compact ms measured |

Already gone on #252: screen-probe place, trace, gather and filter, the probe tile and probe settings (they parse, warn once and do nothing). The "screen probes end state" is therefore reached by #252, not by this design. This design removes the last scheduler that belonged to the probe era: the per-entry due-list machinery.

The settled-state a-trous (the dirty-page denoiser) stays. If the pick pass writes fewer entries per frame, the dirty list shrinks and the denoiser cost follows; that is an effect to measure, not a deletion.

## 11. VRAM (soft budget ~200 MB lighting)

New, High at 2560x1440, tile 8 (**est**):

| Item | Size |
|---|---|
| Deposit table, 230,400 records x 64 B | 14.7 MB |
| Pick scratch (per tile: need max, pick count, histogram) 57,600 x 16 B + 64 buckets | 0.9 MB |
| Virtual tile hits (4,096 x 32 B) | 0.13 MB |
| Per-entry added fields (`shared_count`, fill stamp) | in existing spare words, 0 MB |
| **Added** | **about 16 MB** |

Removed: the due-list regions, histogram state and ring words that live in the key table (small, not counted as a saving). The cache table itself (368 MiB at 1M slots) is not touched by this design and is already over budget until ADR 0036's per-preset sizing lands; the fill must not make that worse beyond the 16 MB, and it does not need more slots than today (it touches fewer entries per frame).

The reservoir option for sharing (not built) would have cost 32 B per cell: 1.2 to 3 MB per 100k cells, small; it is rejected for correlation, not for memory.

## 12. Performance targets and hardware floor

Targets, as in the brief: basic scene about 500 fps at 1440p with a moving light, stress about 100 fps. Measured 4070 state: stress hall settled 65 fps, lighting 15.2 ms with 0 cache rays; initial fill 35 fps; lamp move 35.5 fps. Neither target is within reach of any fill change, because the settled lighting cost alone (15.2 ms) is 1.5x the whole 100 fps frame budget (10 ms) and 7.6x the 500 fps budget (2 ms), before a single cache ray is traced. (The 500 fps target is for the basic scene; the 4070 number here is the stress hall, so the basic-scene gap is unmeasured.) So:

- **This design's perf gates** are about the cases it touches: fill fps at or above #265 at 1M (42.5), lamp-move fps at or above #265 (39.0), peak rays at or under 1M in every phase, **yaw fps not below flag off (51.8)** (#265 is 3.5 fps slower there), and settled lighting ms not above flag off (15.17).
- **No claim** on 500 or 100 fps. The report states the gap and which passes own it (direct, ReSTIR, compose, a-trous).
- Expected cost of the passes (**est**, to be replaced by F2 numbers): pick pass 57,600 threads each reading one texel slot and 64 lanes of shared reduction, well under 0.1 ms; path pass at the base 57.6k picks with the existing hit shade about 115k segments, a small fraction of today's 1.4M rays. The real saving is expected from deleting the compact scan and settle machinery, which are measured in F2/F6, not assumed.
- **RAYCEIL guard.** RAYCEIL cut rays and raised ms because the check and escalation path added dispatches and atomics. The tile fill adds three small dispatches (pick, path, resolve) and removes several. F2's gate is total frame ms and per-pass ms in `LIGHTING_MS`, not the ray counter.

Hardware floor:

- **GTX 1060 Low:** Low has no cell layer today (probes-era behaviour retained by ADR 0039 decision 2). The fill is cheap at large tiles: 32x32 tiles is 3,600 picks at 1440p, one dispatch each of pick, path and resolve. Whether Low gets cells plus tile fill, instead of its probes, is decided by measured 1060 numbers (ADR 0039 open question 2). Until then Low is unchanged.
- **RT optional:** the path pass uses the shared tracer (software SDF or `VK_KHR_ray_query`). No hardware RT is required, and no result depends on which tracer ran.
- **macOS:** the software tracer on MoltenVK, tile 16 or 32. One workgroup per tile, 64 lanes: no subgroup-size assumption (the weighted reservoir uses shared memory, as `lighting_screen.comp` does). A fixed-point `atomicAdd` on `uint` is core Vulkan.
- **Compile time.** Compose is the pipeline whose compile stalls the 4070 for about 20 s on quit (#142). The new passes are separate small pipelines, compose stays a pure lookup (no atomics, no writes), and the tile fill adds no code to compose. Compile time of each new pipeline is a gate (F2).

## 13. Making the wrong thing hard (structure first)

Following the trust pyramid, mistakes are removed by structure before rules:

- **One writer.** Only the resolve pass writes a cache entry's mean, SH and counts in a frame. Deposits are integer atomics. A second writer is a compile-time impossibility if the deposit and entry buffers are bound with different access in the pipeline layout (the resolve pipeline is the only one with write access to entries).
- **Typed budgets.** A `FillBudget { tiles, max_picks, ray_ceiling }` value, built once from the preset and the screen size, is the only way to configure the pick pass. No other code reads a raw `ray_budget` number.
- **Newtype for sample provenance.** `Own` and `Shared` deposit weights are different types; only `Own` can raise `own_count`. A shared sample cannot make a cell look converged.
- **Pure lookup compose.** Compose never allocates or writes, as the read-only design argued.
- **Gates as data.** Each share gate is a named reason code (`plane`, `normal`, `axis`, `near_hit`, `own_count`, `stamp`, `clamp`), counted in `gi_metrics` and shown as a heat map. A gate that rejects nothing, or everything, shows up as a number.
- **Knockout tests (section 14).** Every new test must go red when the code it guards is removed (go-hard playbook: a presented-frame gate; the #72 and #76 no-op wiring loops).

## 14. Measurement plan

Rules: frozen thresholds unchanged. The path-traced reference (`img/hall-path-reference.png`, 72 spp, and the GPU reference of E10) is the primary measure. Validation runs headless on lavapipe first; the 4070 is used once a stage passes its accuracy gates, one bounded `nice` job under `~/genos-bench.lock`, free RAM checked first (the zanven-pc rules).

### 14.1 Before any implementation (stage F0)

Both land before any fill code, as the go-hard playbook requires (oracle first, presented-frame gate).

1. **Reference-first comparison test.** A script-driven test (`examples/stress`, the `lighting_oracle` family) that records, for the current build: near-weighted relative-luminance error against the path reference **as a function of frames** for (a) initial fill from a cleared cache, (b) lamp 3 move and hold, (c) still hall, (d) yaw from a settled hall, plus time-to-95%-near-at-cap. It writes `fill_curve.json`. It runs today with no fill code and gives the baseline every later stage must meet or beat. It fails on a build whose fill is disabled (it must go red for "no fill").
2. **Presented-frame knockout test.** The test reads the frame that reaches the swapchain (the compose output the picture gates already read), not a counter or a header word. It runs once with the fill on and once with the new pick pass bypassed, and asserts the pictures differ by more than noise and that the bypassed one fails the curve gate. It exists so that "wired" means the final frame changed (the #252 unlit-albedo and the #187/#188 no-frame loops). Stage F0 lands it against the current scheduler with the *scheduler* knocked out, which proves the knockout works before there is a fill to guard.
3. **Cause of #265's mid-move lag.** Trace the 6% near pixels at 20 codes or more (red box, mid-move): bounce, stored light or per-pixel? One heat map, one number.
4. **First-hit distance histogram.** What fraction of cache rays on the hall have `t >= 2 m`. This sets how much sharing can ever give (a per-ray counter, no behaviour change).
5. **Ray efficiency.** What share of today's 200 M fill rays change a shown pixel by more than 1 code (E23 did this at the 12 Oct checkpoint). This puts a number on how much of the ceiling is waste.

### 14.2 CPU reference for the estimators (stage F1)

A small crate in the `restir-ref` style (CPU, no device) with: tile pick and water-filling; deposit and resolve on a plane; the sharing weight, all gates and the clamp. Tests:

- Unbiasedness on an unoccluded plane against a brute-force cell integral (error under 1% beyond `t_min`).
- A thin wall, a pillar and a wall base: sharing on versus off, leak measured. The gate fails if a shared cell gains more than 2% of the other side's energy; an injected fault (gate 1 removed) must fail it.
- Fault injections that must fail: uniform pick (far starvation), shared samples counted as own, the `t_min` gate off, weights unclamped.

### 14.3 4070 runs (after F2 passes on lavapipe)

One session per stage, back to back, scripts and pose as `pc-cap8-1111` and `pc-quarters-1111`:

| Run | Compare | Report |
|---|---|---|
| Still hall (initial fill, 0-200; settled 320-610) | flag off (cap 8) vs #265 at 1M vs tile fill | rays peak/mean, lighting ms, fps, first-under-1k, lasting idle, error vs reference, frame-640 image diff |
| Yaw from a settled hall | same three | rays, ms, fps, near crop diff vs flag off |
| Lamp 3 move then hold (frames 81-120, then to 320) | same three | rays peak in the hold (the 1.47M spike), fps, mid-move and after near crops, % near pixels >= 20 codes vs flag off |
| Turn / walk back | flag off vs tile fill | unfilled % (S73L) and reveal error |
| Cache cleared (camera cut) | flag off vs #265 vs tile fill | frames to 95% near, error curve |

Images are looked at: near-camera side by sides and contact sheets, delivered to Anthony. A pass needs numbers and a viewed picture.

### 14.4 Acceptance (proposed, to be reviewed)

| Metric | Gate |
|---|---|
| Frozen gates | blotch 0.014, cell energy 0.9, linger 0.05 at 120, lag 0.9 at 300, turn 0.403, contact/corner/thin-wall no worse than no-skip: unchanged |
| Still hall near error vs path reference | not worse than #265 at 1M + 0.005 (0.0601 -> at most 0.065) |
| Rays peak, any phase | at most 1,000,000 (the ceiling); lamp hold spike gone (flag off 1,470,464) |
| Initial fill fps (0-200, median) | at least 42.5 (#265 at 1M) |
| Lamp move fps | at least 39.0 (#265 at 1M) |
| Lamp move near pixels >= 20 codes vs flag off, mid-move | at most 1% (#265: about 6%) and at the frame the lamp stops |
| Yaw fps from a settled hall | not below flag off (51.8) |
| Settled lighting ms | not above flag off (15.17) |
| Frames to 95% near at cap, cleared cache | at most flag off (27 on lavapipe) |
| Far-flat grain (the #255 heat map) | not worse than flag off; fewer far cells under target at frame 120 |
| Sharing on vs off | blotch and spatial-correlation metric within noise (about +/-0.01 on the near metric); leak test within 2% |
| Flicker / pops | pop share (frames 20-80, 80-320) not worse than flag off |

If the lamp-move lag gate (1%) cannot be met at a ceiling of 1M, that is a result: it is reported with the cause from F0.3, and the stage that follows is the path-vertex stage (F5), not a lower gate.

### 14.5 New probes (rubric rows 1 and 2)

Failure modes and the probe that exposes each, all on `debug/lighting` and the MCP catalogue:

| Failure mode | Probe |
|---|---|
| Far starvation | `fill_need` heat map, `fill_cells_under_target` counter by distance bucket |
| Ceiling breached | `fill_rays`, `fill_paths`, `fill_threshold_bucket` per frame; test asserts <= ceiling |
| Leak across a wall or edge | per-gate reject counts and a `fill_share` heat map |
| Shared sample counted as own | `own_count` vs `shared_count` dump per cell |
| Wired but no effect | presented-frame knockout (F0), `fill` pass in `LIGHTING_MS` |
| Correlated blotch | spatial-correlation metric in the cache-only blotch view |

Tunables: `surface_cache.tile_fill` (off|on), `fill_tile`, `fill_ray_ceiling`, `fill_max_picks`, `fill_target`, `fill_floor`, `fill_share` (off|tile|ring), `fill_share_min_hit`, `fill_share_cap`, `fill_offscreen`. Each is a setting with a CLI argument, an env mirror that applies only when the argument is omitted, an MCP entry, and a debug-menu control, per `docs/engine/settings-principles.md`.

## 15. Staged rollout

Each stage is a separate PR under about 400 lines, one concern, a gate command, with files disjoint from its siblings. Shared renderer files get one owner at a time. Cloud agents: designs and reviews high, implementation of a stage with an oracle medium or low (go-hard playbook, section 6).

| Stage | Content | Gate | 4070 |
|---|---|---|---|
| F0 | Reference-first fill-curve test; presented-frame knockout test; first-hit histogram, ray-efficiency counter; #265 lag cause. No behaviour change | Tests green on the current build and red under knockout; baseline JSON committed | One baseline session |
| F1 | CPU reference crate: pick, water-fill, deposit, resolve, share weight, gates | Unbiased plane test, leak tests, fault injections fail | No |
| F2 | Always-on single-ray scheduler. `fill_budget` is spent every cache round (High 100000). One need-weighted pick per screen tile; the rest (`fill_offscreen_floor`) on a 360-degree clipmap. Every ray deposits that frame into MSME with an n-ray weight. Warm-up is 16 rays. World-mode due lists, the settle-every-cell listing, and the old refresh walks are deleted. `fill_check_rate` and the measured-change term wait for F3 | Rays per round <= `fill_budget`; off-screen rays cover every octant; the leash does not thrash; one ray moves the estimator. Frozen gates unchanged. 4070 frame time, still-hall error, and compile time are not measured here | Yes |
| F3 | Need weights (under, stale, disagree, prox), water-filling threshold, spread-out floor, virtual tiles replace scouts | Fill curve at or better than #265 at 1M; turn, spin, walk-back and unfilled gates | Yes |
| F4a | In-tile sharing with all gates | Leak test within 2%, blotch/correlation within noise, fill curve faster than F3 | Yes |
| F4b | Ring sharing (only if F4a gains) | Same | Yes |
| F5 | Path vertices (up to 3 bounces, backward deposit) only if lamp-move lag needs it | Lamp-move lag gate; no energy gain beyond reference | Yes |
| F6 | Flip default on High and Medium; delete the superseded code in the table of section 10, in separate PRs | Full gate list; Anthony tries it himself (cap 8 and any new default wait for his hands-on, as he asked) | Yes, and his own |
| F7 | Low decision from 1060 numbers | ADR 0039 open question 2 | 1060 |

Merging follows the standing rules: lighting stage PRs into `lighting-v3` after reviews and frozen gates pass; defaults change only after Anthony has tried it.

## 16. Risks

1. **Far and flat grain gets worse.** Model: a uniform pick would starve far floors. Mitigation is need weighting plus the spread-out floor; the gate is the #255 far-grain heat map. If a far floor cell still cannot reach target within the ceiling, the answer is more picks per tile, not a smaller tile.
2. **Sharing leaks or blotches.** Gates, `t_min`, clamp, expiry, no persistence; the leak and correlation tests have injected faults that must fail. If F4a does not gain on the 4070, it is dropped and F4b is never built.
3. **Lamp-move lag returns.** #265 already shows it at 1M. Section 9 lists the mitigations; F0.3 finds the cause first.
4. **Milliseconds do not follow rays (RAYCEIL).** Gate is `LIGHTING_MS` per pass. A stage that does not save measurable time is not pushed.
5. **Cold-fill speed.** One path per pick is one sample. At 57.6k picks and a target of 512 rays per texel, a cold hall needs bursts of picks (water-filling up to the 1M ceiling). Whether the base budget alone matches the speed of today's 200 M-ray fill is not known; F0.5 and F2 measure it. If the ceiling is what makes fill fast, then the fill's advantage is the delete list and the lamp case, not the first 100 frames.
6. **Seed and stamp bugs at reveal.** A cell revealed after a lamp change must not show old light; covered by stamp-on-read and the `wrc_lamps_off` and `settle_flicker` scripts.
7. **Atomic contention on hot cells.** Near cells receive hundreds of paths a frame (table in 5.2). The need weight drives their picks to zero once converged; a per-frame cap of 64 deposits a cell (excess picks are re-aimed by `under` next frame) bounds the hot spot. Measured in F2.
8. **GPU-only bugs.** Lavapipe disagrees with the 4070 on atomics ordering and divergence. The integer deposit is order independent by construction; every stage still has a 4070 session in its own window (go-hard rule: check on the 4070 early).
9. **Departures from Lumen are untested.** Epic gives no performance or error numbers. Everything about Lumen here is a design reference, not a measurement.

## 17. Open questions, answered

1. **Per-tile budget or global?** Global water-filling over tiles with a per-tile cap of 4. A fixed one-per-tile cannot serve both a fresh reveal and a converged near field (model, 5.2).
2. **8x8 or 4x4?** 8x8. The starvation is at far range where a smaller tile only raises the pick count, which `m` already does more cheaply (a 4x4 grid is 230,400 pick workgroups, the same as the max picks, with a much larger pick pass).
3. **Should cells grow with distance (Lumen)?** No. S57. Need weighting covers the same ground. Revisit only if F3 cannot reach target at 40 m within the ceiling (**model:** a far floor cell has 3 pixels but can take many strata from them).
4. **Which origin, pixel or cell centre?** The pixel's surface point: it is the point the camera needs and it needs no lookup. Parallax versus the cell centre is bounded by the same distance gate as sharing.
5. **Biased or unbiased sharing?** Self-normalised, gated, measured. The model shows the reweighting is tight beyond 2 m (median 0.95, p95 1.66) and not usable under 1 m, which is why `t_min = 2 m` and the clamp 2. Unbiased visibility reuse (one ray per shared cell) is only worth its rays if the near-field leak test fails.
6. **Replace scouts?** In F3, once virtual tiles pass the turn and walk-back gates. Until then both exist.
7. **Multi-bounce paths or cache chaining?** Cache chaining stays (one-way rule, no regression). Longer paths with a backward deposit are F5 and only if lamp-move lag needs them; Lumen's three bounces is a design reference, not evidence for us.
8. **Eviction 64 frames?** No. Keep 1800 plus on-screen; revisit after ADR 0036 sizing.
9. **Does this need hardware RT?** No. The shared tracer path is used, so software SDF and ray query both work, and the picture does not depend on which.
10. **Does it fix yaw?** There is nothing to fix on a settled yaw (130k peak). It must not make it slower (51.8 fps gate).
11. **What if #265 is enough?** #265 at 1M fixes the fill and hold ceilings and costs a lamp-move lag and 3.5 fps of yaw. If the F0 cause analysis shows the lag is a cache-sweep artefact that a budget cannot fix, #265 is the shipped fallback and this design is kept as the replacement path. If it passes its gates, #265 is superseded and deleted (section 10).

## 18. Networking, glossary, departures

Networking (rubric row 3):

- Replicated state: none. Picks, deposits and the deposit table live in the rendering client's frame.
- Authority: the machine that shades the picture.
- Prediction / rollback: none.
- Determinism: pick order and jitter are hashes of tile, frame and cell key; deposits are integer sums, so the same inputs give the same cache. A server that does not shade runs none of it.
- Headless server: no picks, no deposits, no cache fill. It can store the settings.

Glossary entries to land with the first implementing PR (proposed, not added to `docs/lighting/GLOSSARY.md` by this design PR, which only adds this document): **Tile fill** (SCHED; Lumen World Cache fill, SHaRC update pass), **Pick** (SAMPLE; ReSTIR candidate, Lumen per-tile pixel), **Fill budget / ray ceiling** (SCHED), **Deposit** (STORE; SHaRC accumulation), **Shared sample** (SAMPLE; ReSTIR GI reconnection shift), **Share gate** (VIS; ReSTIR spatial depth/normal test), **Virtual tile** (SCHED; our scout rays), **Need** (SCHED; Lumen update priority). If a term here conflicts with an existing one, the glossary wins.

Departures (rubric row 4; to join the README deviation list with the implementing PRs):

- Lumen World Cache fill: need-weighted pixel choice inside a tile and water-filling across tiles, instead of one uniform random pixel per tile. Reason: fixed cell size (S57).
- Lumen: cells grow with distance (not adopted).
- Lumen: one path with up to three bounces and a final cache read (default here: one vertex and a cache read; F5 tests more).
- ReSTIR GI reconnection shift: self-normalised weights, no pairwise MIS, no visibility ray, no reservoir; distance gate 2 m and weight clamp 2 are ours.
- The origin estimator stays the cache's uniform-hemisphere cosine-weighted sum (not cosine sampling). The sharing weight is relative to the origin's own contribution, so it does not depend on that choice.

## References

- [ADR 0012: GI](../../adr/0012-gi.md), [ADR 0013](../../adr/0013-lighting-invalidation-is-local.md), [ADR 0022](../../adr/0022-lighting-v3-compute-architecture.md), ADR 0036 (on #252), ADR 0039 (one directional world cache, on #252).
- [Lighting README](README.md): deviations, rules, experiment index.
- Experiments: [S57](experiments/S57-no-camera-distance-sizes.md), [S72B](experiments/S72B-restir-gi-texel-reuse.md), [S73A](experiments/S73A-offscreen-prefill-guard-band.md), [S73B](experiments/S73B-lumen-feedback-prefill.md), [S73L](experiments/S73L-unfilled-on-screen.md), [RAYCEIL](experiments/RAYCEIL-world-cache-need-ray-ceilings.md), [E17](experiments/E17-wrc-seeding-prefill.md), [E19](experiments/E19-rate-based-scheduling-design.md), [E23](experiments/E23-cost-waste-analysis.md).
- [ReSTIR DI plan](restir-di-plan.md), [Localized invalidation](localized-invalidation-design.md), Read-only compose (`compose-read-only-design.md`, on #252), Weekly research, 11 Nov (PR #264).
- PR #262 (cap 8), PR #265 (bounded quarters, `docs/systems/lighting/experiments/bounded-quarters-2026-11-10.md` on that branch), PR #252, PR #259 and #261, PR #264.
- Epic Games, Lumen World Cache, UE 5.9 Preview (9 Nov 2026); Wright et al., Lumen, SIGGRAPH 2022; NVIDIA SHaRC; Bitterli et al., ReSTIR DI, TOG 2020; Ouyang et al., ReSTIR GI, HPG 2021; Bitterli et al., GRIS, SIGGRAPH 2022; Schied et al., SVGF 2017 and A-SVGF 2018; Barre-Brisebois et al., Ray Tracing Gems ch. 25.

## Appendix A: coverage model

```python
import math
H=1440; a=2*math.tan(math.radians(30))/H; h=1.7       # pixel angle, eye height
for r in [2,4,6,10,20,40,80]:
    wall=0.0625/(a*a*r*r)                              # pixels on a 25 cm wall cell
    floor=0.0625*h/(a*a*r**3) if r>h else float('nan') # grazing floor cell
    print(r, wall/64, floor/64)                        # paths per frame, one pick per 8x8 tile
```

## Appendix B: sharing weight model

```python
# origin at 0, normal +y, uniform-hemisphere direction (as the cache traces), hit at distance t on a random surface;
# receiver 0.125..0.354 m away on the same plane (adjacent texel .. cell diagonal).
# w = (cos_theta_n/cos_theta_o) * (cos_phi_n/cos_phi_o) * (t**2/d_n**2)
```

Full script: 400,000 samples per row, seed 7, cosines clamped at 0.05 at the origin and hit; the table in 7.2 is its output. It is a model of the geometry term only; it does not model visibility, which is why section 14 tests leaks separately.
