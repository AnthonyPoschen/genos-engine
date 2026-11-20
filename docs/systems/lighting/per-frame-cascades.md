# Per-frame cascades (proposal, for review)

Status: proposal, 20 Nov 2026. No engine code changes yet. It builds on [sparse-radiance-cascades.md](sparse-radiance-cascades.md) and replaces the ray scheduling in [one-ray-allocator.md](one-ray-allocator.md).

## Why a voxel can be empty today

On `lighting-v3` (52f96918) a level-0 voxel traces no rays of its own. Each frame, the G-buffer inserts voxels into the level-0 hash. Publish then fills each one from whatever the world cells already hold: the lattice, the 25 cm cell, each coarser size, and one inward neighbour. The world cells get their rays from the fill allocator. That allocator gives one coverage ray to each live cell every two frames, plus a fixed burst ranked by value: 400,000 rays on High. Each cell builds a running mean over many frames. When the camera turns, it inserts a voxel whose surface no world cell has traced yet. Until the allocator reaches that surface, the voxel has nothing to read. That is why the parent fallback exists. Black is the case where every rung of the fallback misses too.

## Target

Level 0 is the only level the picture reads, and every level-0 voxel traces its own rays **every frame**. So by construction, no voxel lacks data in the frame it appears.

Upper levels are world cells around the camera, both on screen and off. They keep history. Each frame refreshes a bounded share of them on a rolling stride, and nearer cells refresh more often. History follows Anthony's rule: a change lowers the weight of stored samples in proportion to its size, and only invalid data is a hard reset.

Off-screen light arrives because an upper cell's rays cover its whole hemisphere, not the screen. The sun-on-the-ground case works like this: a floor voxel on screen has an ancestor cell (say 1 m, interval 4 to 8 m). That cell's rays leave through a window behind the camera, hit sunlit ground outside, and shade that hit with the exact sun plus the bounce stored in the cell there.

## Layout

### Sizes

One power-of-two grid serves every level. Level-0 voxel size stays `PROJ * d`, from the Chebyshev distance rings (`crates/render/src/cascade.rs`, `lighting_cascade.glsl`), with the 10% ring overlap. The proposal changes `PROJ` from 0.05 to 0.0625. Sizes then become `2^n / 16` m (6.25 cm, 12.5, 25, 50 cm, 1 m, ...), the same family as today's 12.5 cm, 25 cm, and 50 cm world cells. A voxel's parent is the cell of twice its size at `floor(pos / 2s)`, keyed by cell, size, and dominant normal axis in the existing world hash. This also scales the level-0 count by about 0.64. The budget below uses the measured 0.05 counts, so it is an upper bound.

### Intervals

The interval depends only on cell size, the rule `lighting_cascade_interval` already uses:

| Level | Probe | Ray interval | Rays per probe |
|---|---|---|---|
| 0 | Screen voxel, size `s` = `PROJ * 2^ring` | `[ε, 8s]` | 16 per frame, stratified, fixed per voxel key, rotated by the golden angle each frame |
| upper, size `S` = `2s`, `4s`, ... | World surface cell | `[4S, 8S]`, the last one clipped at 200 m | 16 for `S` ≤ 0.5 m, 32 up to 4 m, 64 above, per refresh |

From ring 0, nine upper sizes reach 200 m. From ring 4, five do. Penumbra condition: spacing over interval start is `S / 4S` = 1/4 on every upper level.

The 25 cm full-trace exception (`interval()` returns `[1e-4, 200]` for a 25 cm cell) goes away. Contact and the near field now belong to level 0's own `[ε, 8s]` rays.

### Upper extent around the camera (on and off screen)

Upper cells come into being in two ways:

1. **Ancestor claim.** Each level-0 voxel claims its parent chain each frame.
2. **Ray hits.** A level-0 or upper ray that hits a surface claims the cell for that surface at its distance band. Off-screen surfaces that light actually reaches therefore exist and hold a mean.

A cell of size `S` is kept while its Chebyshev distance from the eye is at most `S / PROJ`, which is 16·S at 1/16. That gives a camera-centred clipmap: 25 cm cells to 4 m, 1 m cells to 16 m, 4 m cells to 64 m, in every direction. Camera direction plays no part. A cell outside its extent is freed. That replaces the age rule `LIGHTING_CACHE_LIFE` as a lifetime: age and screen are not inputs.

### Merge

The merge runs from the back towards the eye, `L(a,c) = L(a,b) + β(a,b)·L(b,c)`, per direction. An upper cell stores SH-L1 radiance for `[4S, ∞)` after its own merge. A level-0 voxel fetches its parent SH once: a sparse trilinear over the parent corners that exist, renormalised, with the plane and facing gates. Each level-0 ray then:

- **Hits inside `[ε, 8s]`:** shades the hit. That is direct light at the hit, plus the bounce stored in the cell at the hit.
- **Misses:** evaluates the parent SH in its direction (β = 1).

The voxel's value is the cosine-weighted sum of its 16 rays. This is the real interval merge. Today's "β is 0 while the nearer sample exists" substitution goes away, because the nearer sample is always this frame's.

## Temporal stability

| Level | What stays | Rule |
|---|---|---|
| 0 | A one-frame EMA against the same voxel key from the previous frame, double-buffered (two 16384×12-word tables, about 1.5 MB) | The current frame's 16 rays always give a full value, so the EMA only smooths. New weight is at least 0.5. Prior weight shrinks with the change: luminance gap against the voxel's own ray spread, a lamp delta in reach, or a mover sphere. A missing key last frame, a flipped normal, or a ring change is a reset. That reset is still a full value, never black. |
| upper | The running mean and variance, as today | Proportional down-weight. Hard reset only for a gone surface, a failed plane or facing check, or a change of about 1 (lamp off, colour flip). This is the stage-1 rule of `one-ray-allocator.md`, with no allocator around it. |

Stable directions per voxel key, rotated slowly, keep fixed-pattern bias from crawling across a still picture. The ring overlap blend is unchanged.

## Refresh schedule for upper levels

Rolling stride: a cell refreshes on the frame where `(hash(key) + frame) % P == 0`. `P` depends on the size and on whether a level-0 voxel claimed the cell this frame:

| Size band | Claimed by an on-screen voxel | Off screen (ray-hit or retained) |
|---|---|---|
| `S` ≤ 0.5 m | every 2 frames | every 8 |
| 0.5 m < `S` ≤ 4 m | every 4 | every 16 |
| `S` > 4 m | every 8 | every 32 |

A change halves `P` for 30 frames, for cells inside the before-and-after reach of the lamp or mover. That change is the existing `lighting_lamp_delta` and mover-sphere input. There is no rank, histogram, burst, budget split, or seed. The ray count is the sum of `k / P` over resident cells, so it is bounded and known from cell counts.

## Direct light

| Light | Where it goes |
|---|---|
| Brightest near lamp | Exact per pixel: one disc-sampled shadow ray (#310's penumbra), as approved today |
| Other lamps, near or distant | Folded into the cascades. Each level-0 voxel takes one disc-sampled shadow ray per lamp per frame at its centre, smoothed by the level-0 EMA. Upper-ray hits shade every lamp (today's `cache_pick`). This leaves no per-pixel cost and no ReSTIR. |
| Sun | Hybrid, pending the sun study (bc-27dddef0). Each voxel traces one sun ray at its centre and four at its corners. A voxel whose five rays agree takes the cascade sun with no per-pixel ray. A voxel whose rays disagree (it straddles a shadow edge) marks its pixels for one exact per-pixel sun ray. Expect 10 to 20% of pixels on edges. Full per-pixel sun stays the fallback until that study shows the edge error against the reference. |

The ReSTIR spatial and candidate passes are deleted, as already approved.

## Deleted (actual code)

### Allocator and scheduling

- `crates/render/shaders/lighting_fill.glsl`: `lighting_fill_q`, `_bin`, `_hist`, `_plan`, `_pour_row`, `_dispatch`, `_emit_on`, `_emit_off`, `_push_check`, `_check_*`, `_store_picks`, `_wg_place`, `_holds_tile`, `_tile_grid`, `_pace`, and `_commit`, which writes blend 1 on a lamp note.
- The same file's F4a seeding: `lighting_fill_share*` and `lighting_fill_seed*`.
- The same file's distance coarsening (`_visit`, `_coarsen`, `_merge_into`). A size's extent replaces it.
- The fill parameter, header, histogram, plan, and pick-list tails in `lighting_gpu.rs`: `lighting_fill_keys_words`, `fill_list_end`, `fill_burst_placement`, `burst_quarter_shares`, `log_fill_cell_error`, and `print_fill_cells`.
- `FillSettings` (`crates/scene/src/lighting.rs`: `budget`, `burst`, `max_picks`, `tile`, `check_rate`, `share_*`, `floor_rays`, ...) and its env mirrors.
- The CPU mirror `crates/fill-ref` (`alloc.rs`, `water.rs`, `sched.rs`, `share.rs`, `pick.rs`, `commit.rs`). `deposit.rs` survives only if the upper blend still uses it.
- `lighting_compact.comp`'s young/mature list and the stale check. `lighting_cache_batch_slot` is already empty on world mode 3. `lighting_cascade_ray_slot` reads the new refresh list.

### Parent fallback and departures

- `lighting_cascade.glsl`: `lighting_cascade_ancestor`, `_children`, `_inward` (`CASCADE_INWARD_HOPS`), `_facing`, `_voxel`, and the lattice-miss chain in `lighting_cascade_merge`.
- `crates/render/src/cascade.rs`: `shown_sample`, `keeps_sample`, and the β-is-0 `merge`.
- Diag 11 stays as the regression counter (`LIGHTING_DIAG_CASCADE_GAP`).
- The publish read of the SH lattice (`lighting_sh_lattice_at`) and the lattice table itself: `lighting_sh_lattice_*`, `lighting_sh_drain.comp`, and the 32-frame refresh.
- The 25 cm full-trace exception in `interval()` and `lighting_cascade_interval`.
- The departures bullets in `sparse-radiance-cascades.md` that describe the above.

### Fine texels (replaced by level 0)

- `lighting_fine_spawn` and `lighting_fine_evict` (12.5 cm children).
- `lighting_compact_hold_texel_rays`, `texel_ray_cut_prod.comp`, `texel_detail_prod.comp`, and their tests (`texel_ray_cut.rs`, `texel_detail_prod.rs`).
- `mover_texels`.

### Clears

- `lighting_lamp_keep`, `lighting_lamp_noted`, the 1/8 and 1/32 blend floors, and the age eviction (`LIGHTING_CACHE_LIFE`).
- Kept: `lighting_lamp_delta`, the mover sphere (as change-size inputs), and the shadow epochs on the direct path.

### Gates affected

The fill-knockout harness (`examples/stress/tests/fill_knockout.rs`, gate ≤ 0.213, frozen) knocks out `cache_fill` writes. It must be re-pointed at the level-0 and upper writes with the same threshold. The gate value does not move.

## Ray budget, 2560×1440, hall

### Measured on the box

This run used lavapipe on `52f96918`, High, the hall pose (14.5, 1.7, 23.5, yaw −0.3554, pitch 0.08), still boxes, frozen sun. A scratch census read the level-0 table, the world hash, and the fill stats words. It was a throwaway local build, not committed. The level-0 table was raised to 65536 for the count, which showed that 16384 is not saturated.

| | Hall, frame 11 | Floor look (pitch −0.55) |
|---|---:|---:|
| Level-0 voxels (incl. overlap) | 11,364 (ring 0: 528, 1: 3,374, 2: 4,578, 3: 2,471, 4: 413) | 7,123 |
| Peak level-0 seen (frame 1) | 15,189 | 8,187 |
| Upper ancestors of those voxels, sizes 0.1 m to 25.6 m | 4,024 (156 / 1,010 / 1,101 / 1,044 / 446 / 160 / 65 / 31 / 11) | 2,882 |
| World cells live today (25 cm + 50 cm) | 48,260 | 30,640 |
| Allocator rays per frame today | **424,011** = 24,011 coverage + 400,000 burst | 279,532 |

The allocator count is set by the preset burst and half the live cells, so the 4070 traces the same number at this pose.

### Cost per ray on the 4070

From the 20 Nov 1440p run on `662f9768` (`/workspace/r-v3-4070-1120/NOTES.txt`), at 424k rays:

| Pass | ms | ns per ray |
|---|---:|---:|
| cache_rays | 0.457 | 1.08 |
| cache_hit_shading | 0.859 | 2.03 |
| cache_round | 0.682 | 1.61 |
| compact | 0.304 | per slot, fixed |

The conservative figure is 4.7 ns per GI ray (trace, shade, merge), which is about 4.7 ms per million rays. Level-0 rays are at most `8s` long, and many miss, which skips hit shading. The optimistic figure is 3.0 ns. A shadow-only ray costs 1.08 ns.

### Level 0, every frame

| Voxels | 8 rays | 16 rays | 32 rays | 64 rays |
|---|---:|---:|---:|---:|
| 11,364 (hall) | 91k rays, 0.43 ms | 182k, **0.86** | 364k, 1.71 | 727k, 3.43 |
| 15,000 (peak margin) | 120k, 0.57 | 240k, 1.13 | 480k, 2.26 | 960k, 4.52 |
| 7,123 (floor) | 57k, 0.27 | 114k, 0.54 | 228k, 1.07 | 456k, 2.15 |

Conservative ms (4.7 ns). The optimistic figures are about 0.64× these.

### Upper levels, rolling refresh

Resident count = on-screen ancestors, plus off-screen cells at 8.6× for sizes up to 1.6 m and 3× above that. 8.6 is the ratio of the full sphere to the 91.5°×60° frustum (1.46 sr). The hall estimate comes to about 37k resident cells, close to today's 48k live.

| Schedule | Hall rays per frame | ms (4.7 / 3.0 ns) | Floor rays per frame |
|---|---:|---:|---:|
| A: on-screen 2/4/8, off-screen 8/16/32 (recommended) | 98k | **0.46** / 0.30 | 71k |
| B: every period doubled | 49k | 0.23 / 0.15 | 35k |
| C: every period halved | 197k | 0.93 / 0.59 | 141k |

Per size for A, hall: 0.1 m 3.9k, 0.2 m 25k, 0.4 m 28k, 0.8 m 26k, 1.6 m 11k, 3.2 m 2.2k, then under 1k each.

### Lighting total, hall (4070 estimate)

| Item | Recommended | Conservative worst |
|---|---:|---:|
| Level 0, 16 rays (11.4k voxels; 15k worst) | 0.86 | 1.13 |
| Level-0 lamp and sun shadow rays at the voxel (5 per voxel) | 0.08 | 0.10 |
| Upper refresh (A; C worst) | 0.46 | 0.93 |
| Claim, refresh list, compaction (replaces compact and fill) | 0.15 | 0.30 |
| Publish and compose (1.66 today, no fallback walk) | 1.2 | 1.66 |
| Direct per pixel: brightest lamp, plus the sun on edge pixels (full per-pixel sun worst) | 1.6 | 2.8 |
| **Lighting** | **≈ 4.4 ms** | **≈ 6.9 ms** |

Today it is 16.1 ms. After the approved cuts, it is estimated at about 7.5. The target is under 8 ms, so even the worst column fits. The GI rays drop from 424k to about 280k per frame, and every visible voxel is fresh every frame.

### If it is over budget

In order:

1. Level 0 at 8 rays with the EMA. The effective count is about 16 over two frames. Saves about 0.4 ms.
2. Upper schedule B. Saves about 0.23 ms.
3. `PROJ` 0.0625 to 0.08. About 0.6× the voxels.
4. A bounded short-ray traversal for level 0 (`t ≤ 8s`, early out).
5. Rings 3 to 4 at 8 rays and rings 0 to 2 at 16. Far voxels are small on screen.

Level 0 is not put on a half rate: every voxel traces every frame.

## Test scenes

| Scene | Pass |
|---|---|
| Hall, still (existing pose) | All frozen gates: hall all 120..136 and mid 101..113, floor splotch ≤ 0.045, wall splotch ≤ 0.053, knockout ≤ 0.213 (re-pointed), ReSTIR 0.05, SPIR-V < 248,000 B, moving box hot ≤ 3 |
| Sun through a window, window behind the camera | New pose: RoomA with a sun-facing window behind the eye, against the path reference. Floor mean within 10% of the reference from frame 2. After a 180° turn over 30 frames, the same floor patch stays within 5% of its facing-window value. |
| Moving camera | `s2-strafe` and a 90°/s yaw. On every frame, a new counter "level-0 voxels traced this frame" equals "level-0 voxels inserted". Diag 11 = 0. Flicker hot ≤ 3. |
| Black cell | The existing test, plus the counter above, from frame 1 of a cold start |
| Cost | The 4070 at 1440p hall: per-level ms (level 0, each upper band, publish, direct) and lighting span < 8 ms |

## Uncertain

- **Cost per ray.** The 4.7 ns comes from today's mix: long rays to 200 m, and hit shading on most rays. Short level-0 rays should be cheaper, but this has not been measured. The level-0 merge (one parent SH fetch per voxel, 8 gated corners) is also unmeasured.
- **Off-screen count.** The 8.6× multiplier is geometric. It ignores occlusion and the rooms behind walls. Ray-hit claims could push the resident count above 37k. The cost scales linearly.
- **Level-0 noise.** 16 rays with a one-frame EMA may shimmer on bright, near contact. The fallback is 32 rays (+0.86 ms) or a 0.33 new-weight EMA.
- **Sun edges.** The quality of the edge-pixel test against the reference is open (bc-27dddef0).
- **Compose and publish.** Under 1.66 ms once the fallback walk is gone, but not measured.
- **Lavapipe counts.** Lavapipe G-buffer and level-0 counts should match the 4070. The 4070 run printed no counters to confirm it.
- **The known leak** at wall, floor, and ceiling joins. Level-0 rays from a voxel that straddles a join could still leak. That stays held.

## Networking

- Replicated state: none.
- Authority: the machine that shades.
- Prediction / rollback: none.
- Determinism: the same pose, G-buffer, lamps, and frame index produce the same voxels, rays, and refresh set.
- Headless server: does not compute it.
