# Per-frame cascades

Status: accepted, building, 20 Nov 2026. It builds on [sparse-radiance-cascades.md](sparse-radiance-cascades.md) and replaces the ray scheduling in [one-ray-allocator.md](one-ray-allocator.md).

## Why a voxel can be empty today

On `lighting-v3` (52f96918) a level-0 voxel traces no rays of its own. Each frame, the G-buffer inserts voxels into the level-0 hash. Publish then fills each one from whatever the world cells already hold: the lattice, the 25 cm cell, each coarser size, and one inward neighbour. The world cells get their rays from the fill allocator. That allocator gives one coverage ray to each live cell every two frames, plus a fixed burst ranked by value: 400,000 rays on High. Each cell builds a running mean over many frames. When the camera turns, it inserts a voxel whose surface no world cell has traced yet. Until the allocator reaches that surface, the voxel has nothing to read. That is why the parent fallback exists. Black is the case where every rung of the fallback misses too.

## Target

Level 0 is the only level the picture reads, and every level-0 voxel traces its own rays **every frame**. So by construction, no voxel lacks data in the frame it appears.

Upper levels are world cells around the camera, both on screen and off. They keep history. Each frame refreshes a bounded share of them on a rolling stride, and nearer cells refresh more often. History follows Anthony's rule: a change lowers the weight of stored samples in proportion to its size, and only invalid data is a hard reset.

Off-screen light arrives because an upper cell's rays cover its whole hemisphere, not the screen. The sun-on-the-ground case works like this: a floor voxel on screen has an ancestor cell (say 1 m, interval 4 to 8 m). That cell's rays leave through a window behind the camera, hit sunlit ground outside, and shade that hit with the exact sun plus the bounce stored in the cell there.

## Layout

### Sizes: one world grid, picked by screen footprint

**Every level-0 voxel is a real world cell.** It is an axis-aligned cube on the shared power-of-two grid, `2^n / 16` m (6.25 cm, 12.5, 25, 50 cm, 1 m, ...). That is the grid the upper cells and today's 12.5 cm, 25 cm, and 50 cm world cells use. The voxel's parent is the cell one level up, the integer cell coordinates shifted right by one. That parent is the upper cell that already exists at that size, keyed by cell, size, and dominant normal axis in the world hash. Its parent is the next upper cell, and so on to 32 m.

Screen footprint only chooses **which grid level** a surface point snaps to. It never makes a new grid, a non-cubic box, or a per-axis size. There is no per-axis or anisotropic option.

The rule, in plain words: pick the cube size whose patch of this surface covers about a fixed area on screen. A surface seen face-on gets the size its distance gives. A floor or ceiling seen at a grazing angle is squashed on screen, so it gets a coarser grid level, which is up to two levels (4×) coarser. The level-0 voxel count then follows the screen area, not the geometry.

- The wanted size at a point is `want = K · dist / sqrt(max(|n_a · v|, 1/16))`. Here `dist` is the Euclidean distance from the eye, `v` the view direction, and `n_a` the dominant normal axis. `K` is the angular size of a box. `K = 1/32` gives boxes of about 22 × 40 px at 1440p.
- The `1/16` floor caps the grazing boost at 4×, which is two grid levels.
- **Choice.** Each pixel has a continuous footprint level, `lod = log2(want(pixel) / (1/16 m))`, like a texture mip level. Insert claims the cells at `floor(lod)` and `floor(lod) + 1`. Publish shows `mix(level floor, level floor + 1, fract(lod))`.
  - The weight depends only on the pixel, so there is no edge along cell borders. A voxel appears at weight 0, so its first frames, with one ring slot filled, are not seen.
  - Two levels per pixel cost about 25% more voxels than one (the coarser level has a quarter of the cells).
  - Was: the largest `S` with `S ≤ want(cell centre)`, a quadtree with a blend only in the top tenth of a level. The blend weight was constant per cell, so level borders showed as a cell-shaped staircase.
- **Normal inset.** The point is moved half a cell into the surface along `n_a` before `floor(p / S)`.
  - Today, a floor at y = 0 or a wall on a grid plane flips between the two cells either side of the plane with depth noise. On the census below, 38% of the hall's level-0 records are such duplicates (82% of records sit within 2% of a grid plane).
  - This, not sub-pixel voxels, is the speckled band in the #313 voxel view. Depth noise grows with distance, and the cell size is fixed within a ring, so it is worst just before each ring line. The inset puts the point mid-cell along the normal.
- **Stability.** Inputs are the pixel position and the eye, so a still camera gives the same voxels every frame. There is no hysteresis: a moving camera changes the weights continuously.
- **Insert and read agree by construction.** Insert, publish, bounce and the voxel debug view all call `lighting_cascade_lod`.
- **Sample point.** A voxel's rays start on the surface: the axis line through the cell centre meets the pixel's surface plane there. On a face that is not axis-aligned (a turned box), moving the centre along the axis alone left the point up to a few centimetres behind the face.
- **Table.** The host zeroes this frame's level-0 table before insert. A slot is claimed by one compare-and-swap from key 0. Before, a slot was free when its stamp was old, and the stamp was written after the key, so two inserts could write one record.
- **Leaks.** A coarser cube spans more along the normal, so it can join two parallel surfaces closer than `S`, such as two floors either side of a thin wall.
  - The cap of two levels bounds this, and the dominant-axis-and-sign key keeps walls apart.
  - The plane gate uses the distance size `K·dist`, not the enlarged `S`. A parallel surface farther than that from the stored plane is refused.
  - The known junction leak stays held. This rule must not make the thin-wall test worse.
- **Ring lines.** Chebyshev rings stop deciding level-0 size. Level boundaries become contours of `dist / sqrt(cos)`. On walls these are close to today's lines. On floors and ceilings they are smooth curves, softened by the blend. The voxel view should draw these contours.
  - The upper cells still keep a Chebyshev retention extent per size (below). That extent is not shown.

### Intervals

The interval depends only on cell size, the rule `lighting_cascade_interval` already uses:

| Level | Probe | Ray interval | Rays per probe |
|---|---|---|---|
| 0 | Screen voxel, a world cell of size `s` chosen by footprint | `[ε, 8s]` | 32 per frame (16 is the cheaper option), from a fixed 128-direction set split into 4 interleaved slots; frame `f` traces slot `f % 4`. Same directions every cycle, plus a per-voxel turn from the key |
| upper, size `S` = `2s`, `4s`, ... | World surface cell | `[4S, 8S]`, the last one clipped at 200 m | 16 for `S` ≤ 0.5 m, 32 up to 4 m, 64 above, per refresh |

From a 6.25 cm voxel, nine upper sizes reach 200 m (12.5 cm to 32 m). Penumbra condition: spacing over interval start is `S / 4S` = 1/4 on every upper level.

The 25 cm full-trace exception (`interval()` returns `[1e-4, 200]` for a 25 cm cell) goes away. Contact and the near field now belong to level 0's own `[ε, 8s]` rays.

### Upper extent around the camera (on and off screen)

Upper cells come into being in two ways:

1. **Ancestor claim.** Each level-0 voxel claims its parent chain each frame.
2. **Ray hits.** A level-0 or upper ray that hits a surface claims the cell for that surface at its distance band. Off-screen surfaces that light actually reaches therefore exist and hold a mean.

A cell of size `S` is kept while its Chebyshev distance from the eye is at most `16·S`. That gives a camera-centred clipmap: 25 cm cells to 4 m, 1 m cells to 16 m, 4 m cells to 64 m, in every direction. Camera direction plays no part. A cell outside its extent is freed. That replaces the age rule `LIGHTING_CACHE_LIFE` as a lifetime: age and screen are not inputs.

### Merge

The merge runs from the back towards the eye, `L(a,c) = L(a,b) + β(a,b)·L(b,c)`, per direction. An upper cell stores SH-L1 radiance for `[4S, ∞)` after its own merge. A level-0 voxel fetches its parent SH once: a sparse trilinear over the parent corners that exist, renormalised, with the plane and facing gates. Each level-0 ray then:

- **Hits inside `[ε, 8s]`:** shades the hit. That is direct light at the hit, plus the bounce stored in the cell at the hit.
- **Misses:** evaluates the parent's 16 direction bins in its direction (β = 1), bilinear across bins.

Parent cells, as built (20 Nov 2026):

- **Read.** A bilinear of the four parent cells around the voxel on its surface plane, between cell centres. Along the normal there is one cell, the one holding the surface.
- **Position.** A claimed parent cell sits on the claiming voxel's surface plane, under the cell's tangent centre. It used to sit on a grid plane, which put the cells of an off-grid wall inside the wall; their rays saw nothing.
- **Re-seat.** When a level-0 voxel finds a parent cell behind its own surface (a cell claimed while a box stood elsewhere), it moves the cell onto its plane and clears it; the rolling refresh shades an empty cell in the frame it is found. Only level-0 voxels re-seat, because only they carry the surface normal. Upper cells store the axis normal, and letting them re-seat moved a cell back and forth every frame.
- **Visibility.** A parent corner the voxel cannot see, by one shadow ray from the voxel to the cell, is skipped. A floor next to a thin wall otherwise read parents on the far side.
- **Bins.** A bin is the mean radiance of all its rays: a hit, or the parent's bins on a miss. The earlier rule left out rays blocked before `4S` and stored a bin with no clear ray as zero. Near walls most bins went to zero and the chain lost energy at every level.

Measured with `crates/debug/tests/cascade_energy_probe.rs` (multi-bounce reference, test hall): 0.57 of the reference before these changes, 0.98 after. Walls went from 0.09–0.47 to about 0.9; the fine levels were the darkest, which was the pop when a surface moved to a finer level. `crates/debug/tests/cascade_box_probe.rs` checks every voxel on a box face, turned or not and with the camera still or moving: no voxel below half the reference.

The voxel's value is the cosine-weighted sum of its rays. This is the real interval merge. Today's "β is 0 while the nearer sample exists" substitution goes away, because the nearer sample is always this frame's.

## Temporal stability

| Level | What stays | Rule |
|---|---|---|
| 0 | A four-slot direction ring, double-buffered (two 16384×16-word tables, 2.0 MB) | 128 fixed stratified directions are split into 4 interleaved sets of 32. Frame `f` traces slot `f % 4` and replaces that slot. The voxel value is the average of the filled slots. There is no EMA across cycles: a still scene is exact once all four slots are filled, and a change lands within 4 frames. Non-finite samples leave that slot unchanged. A new key starts with one slot and fills over the next frames. |
| upper | This refresh replaces the stored irradiance and bins. An empty cell (just claimed or re-seated) is due at once | Directions are fixed per key and ray, the same every cycle. A still refresh repeats the same set, so the stored value is stable. A change lands on the next refresh (within `P` frames, default 4). Non-finite samples are refused. |

Each voxel also has a fixed turn from its key (`lighting_hemi_dir`), so neighbouring voxels do not share one ray pattern. Compose filters the read: a bilinear of the four same-level cell centres around the query on its surface plane, renormalised, with the plane and facing gates. Stored values are not blended with neighbours. Levels blend by the pixel's footprint level (see Sizes).

## Refresh schedule for upper levels

Rolling stride: every upper world cell, on or off screen and of every size, refreshes on the frame where `(hash(key) + frame) % P == 0`. `P` is one fixed period (default 4). A debug setting of 1, 2, or 10 replaces it and prints per-level GPU time. There are no size bands, no on/off-screen periods, no halving near changes, and no same-frame big-change catch-up. A lamp, mover, or sun change is taken on the next regular refresh (fixed directions, the new estimate replaces the stored value). Hard reset only for invalid data. There is no rank, histogram, burst, budget split, or seed. The ray count is the sum of `k / P` over resident cells, so it is bounded and known from cell counts.

## Direct light

| Light | Where it goes |
|---|---|
| Brightest near lamp | Exact per pixel: one disc-sampled shadow ray (#310's penumbra), as approved today |
| Other lamps, near or distant | Folded into the cascades. Each level-0 voxel takes one disc-sampled shadow ray per lamp per frame at its centre, stored into that frame's ring slot and averaged with the other three. Upper-ray hits shade every lamp (today's `cache_pick`). This leaves no per-pixel cost and no ReSTIR. |
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
- `crates/render/src/cascade.rs`: `shown_sample`, `keeps_sample`, and the β-is-0 `merge`. The Chebyshev ring choice for level 0 (`ring`, `cell_size`, `overlap_blend`, and the shader's `lighting_cascade_ring`, `_cell_size`, `_blend`, `_insert_ring`) is replaced by the footprint level choice. The ring code is kept only where the upper retention extent needs it.
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

Lavapipe on `52f96918`, 2560×1440, High, still boxes, frozen sun. Two poses: the hall (14.5, 1.7, 23.5, yaw −0.3554, pitch 0.08) and a floor look (pitch −0.55). A throwaway census build, not committed, read the level-0 table and the stats words. It raised the table to 65536 entries so nothing was lost; 16384 is not saturated today. The size variants replaced the insert rule in that build only.

**Today (allocator):** 424,011 rays per frame in the hall (24,011 coverage + 400,000 burst) and 279,532 on the floor look, with 48,260 and 30,640 world cells live.

**Level-0 voxels.** "< 1 px" and "< 4 px" are the share of voxels whose shorter on-screen side, along a tangent axis, is under that size.

| Rule | Hall voxels | Floor-look voxels | < 1 px | < 4 px | Median box (short × long side) |
|---|---:|---:|---:|---:|---|
| Today: Chebyshev ring, 5 cm base, no inset | 11,377 | 7,131 | 0.0% | 0.8% | 17 × 32 px |
| Today + normal inset | 8,524 | 5,965 | 0.0% | 1.6% | 17 × 32 px |
| Ring rule on the 1/16 grid, inset | 4,908 | 3,831 | 0.0% | 0.7% | 22 × 40 px |
| **Footprint rule, 1/16 grid, inset, `K` = 1/32 (proposed)** | **4,258** | **3,850** | 0.0% | 0.1% | 22 × 41 px |
| Footprint rule, `K` = 1/16 | 1,428 | 1,122 | 0.0% | 0.1% | 44 × 84 px |

What this shows:

- **No sub-pixel voxels.** Under today's rule, no voxel in the hall is under a pixel along either axis. The eye is 1.7 m up and the hall is about 25 m, so the floor's foreshortening stays above about 0.1. The speckle is the grid-plane duplication, which the inset removes (−25%).
- **The 1/16 grid** is the grid the upper cells use. At the same angular size it makes larger voxels, and that does most of the rest.
- **The footprint rule** makes the count nearly pose-independent: 4,258 against 3,850 is a ratio of 1.1, where today's is 1.6. It also removes the thin boxes: under 4 px drops from 0.8% to 0.1%. That is Anthony's fixed number of roughly screen-constant boxes.
- **Sizes used** (proposed, hall): 6.25 cm 326, 12.5 cm 1,114, 25 cm 1,512, 50 cm 891, 1 m 355, 2 m 60.
- **Parent chain** of those voxels (hall): 1,802 upper cells. By size: 12.5 cm 90, 25 cm 310, 50 cm 513, 1 m 412, 2 m 261, 4 m 118, 8 m 51, 16 m 32, 32 m 15. The floor look has 1,525.

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

| Voxels | 16 rays | 32 rays | 64 rays |
|---|---:|---:|---:|
| 4,258 (hall, proposed) | 68k rays, 0.32 ms | 136k, **0.64** | 273k, 1.28 |
| 6,000 (design margin) | 96k, 0.45 | 192k, 0.90 | 384k, 1.81 |
| 3,850 (floor look, proposed) | 62k, 0.29 | 123k, 0.58 | 246k, 1.16 |
| 11,377 (today's rule, for comparison) | 182k, 0.86 | 364k, 1.71 | 727k, 3.43 |

These are conservative ms at 4.7 ns per ray. The optimistic figure, 3.0 ns, is about 0.64× these. The lower count pays for 32 rays at level 0, which cuts the noise by about 1.4× (1/√2), and still costs less than 16 rays under today's rule.

### Upper levels, rolling refresh

Resident count = the parent chain above, plus off-screen cells: 8.6× for sizes up to 2 m, 3× above that. 8.6 is the ratio of the full sphere to the 91.5°×60° frustum (1.46 sr). That gives about 16k resident cells in the hall.

| Schedule | Hall rays per frame | ms (4.7 / 3.0 ns) | Floor rays per frame |
|---|---:|---:|---:|
| A: on-screen 2/4/8, off-screen 8/16/32 (recommended) | 43k | **0.20** / 0.13 | 37k |
| B: every period doubled | 21k | 0.10 / 0.06 | 18k |
| C: every period halved | 86k | 0.40 / 0.26 | 74k |

### Lighting total, hall (4070 estimate)

| Item | Recommended | Conservative worst |
|---|---:|---:|
| Level 0, 32 rays (4,258 voxels; 6,000 worst) | 0.64 | 0.90 |
| Level-0 lamp and sun shadow rays at the voxel (5 per voxel) | 0.02 | 0.03 |
| Upper refresh (A; C worst) | 0.20 | 0.40 |
| Claim, refresh list, compaction (replaces compact and fill) | 0.15 | 0.30 |
| Publish and compose (1.66 today, no fallback walk) | 1.2 | 1.66 |
| Direct per pixel: brightest lamp, plus the sun on edge pixels (full per-pixel sun worst) | 1.6 | 2.8 |
| **Lighting** | **≈ 3.8 ms** | **≈ 6.1 ms** |

Today it is 16.1 ms. After the approved cuts, it is estimated at about 7.5. The target is under 8 ms. GI rays drop from 424k to about 180k per frame, and every visible voxel traces its own rays every frame. Direct light and compose are now most of the total, so the GI side is no longer what limits the frame.

### If it is over budget

In order:

1. Level 0 at 16 rays (two slots of 16 from a 64-direction set). Saves about 0.3 ms.
2. Upper schedule B. Saves about 0.1 ms.
3. A larger `K`, the box angle. `K` = 1/16 is about a third of the voxels (1,428), at 44 × 84 px boxes.
4. A bounded short-ray traversal for level 0 (`t ≤ 8s`, early out).

Level 0 is never put on a half rate.

## Test scenes

| Scene | Pass |
|---|---|
| Hall, still (existing pose) | All frozen gates: hall all 120..136 and mid 101..113, floor splotch ≤ 0.045, wall splotch ≤ 0.053, knockout ≤ 0.213 (re-pointed), ReSTIR 0.05, SPIR-V < 248,000 B, moving box hot ≤ 3 |
| Sun through a window, window behind the camera | New pose: RoomA with a sun-facing window behind the eye, against the path reference. Floor mean within 10% of the reference from frame 2. After a 180° turn over 30 frames, the same floor patch stays within 5% of its facing-window value. |
| Moving camera | `s2-strafe` and a 90°/s yaw. On every frame, a new counter "level-0 voxels traced this frame" equals "level-0 voxels inserted". Diag 11 = 0. Flicker hot ≤ 3. |
| Black cell | The existing test, plus the counter above, from frame 1 of a cold start |
| Voxel footprint | At the hall and floor poses, and along the strafe, the level-0 count stays within ±20% of `visible px / (K/α)²·c`. No voxel is under 4 px on its short side. No surface cell appears twice along its normal (the inset). Over 120 still frames, no voxel changes level. |
| Cost | The 4070 at 1440p hall: per-level ms (level 0, each upper band, publish, direct) and lighting span < 8 ms |

## Uncertain

- **Cost per ray.** The 4.7 ns comes from today's mix: long rays to 200 m, and hit shading on most rays. Short level-0 rays should be cheaper, but this has not been measured. The level-0 merge (one parent SH fetch per voxel, 8 gated corners) is also unmeasured.
- **Off-screen count.** The 8.6× multiplier is geometric. It ignores occlusion and the rooms behind walls. Ray-hit claims could push the resident count above 37k. The cost scales linearly.
- **Level-0 noise.** 32 rays in a four-slot ring of 128 fixed directions should be still once the ring is full. Structured per-voxel aliasing would be the thing to watch; a stronger per-voxel turn is the fallback, not a longer history.
- **Footprint rule.** Counted on lavapipe at two still poses only. The level-switch dead band, the seam blend, and the leak cap are untested. A footprint-driven coarser cube is a new leak risk on parallel surfaces, and the thin-wall test must cover it.
- **Sun edges.** The quality of the edge-pixel test against the reference is open (bc-27dddef0).
- **Compose and publish.** Under 1.66 ms once the fallback walk is gone, but not measured.
- **Lavapipe counts.** Lavapipe G-buffer and level-0 counts should match the 4070. The 4070 run printed no counters to confirm it. The box side is measured with the screen-centre pixel angle and the stored sample position, so it is approximate.
- **The known leak** at wall, floor, and ceiling joins. Level-0 rays from a voxel that straddles a join could still leak. That stays held.

## Networking

- Replicated state: none.
- Authority: the machine that shades.
- Prediction / rollback: none.
- Determinism: the same pose, G-buffer, lamps, and frame index produce the same voxels, rays, and refresh set.
- Headless server: does not compute it.
