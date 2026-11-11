# Screen-tile fill: one system, always on (addendum)

Status: **Proposed** (11 Nov 2026), addendum to [the screen-tile fill design](screen-tile-fill-design.md). Docs only. Origin: Anthony, 11 Nov 16:59, 17:03, 17:20–17:28, 17:43–17:45. Where this note and the design disagree, this note wins. Code read: `lighting_cache.glsl`, `lighting_cache_slots.glsl`, `lighting_screen.comp` and `shown_age.rs` on #252 (`03ab0952`).

## 1. One system, a scheduler, always on

The world cache is the only store. Texels are its 12.5 cm detail inside the 25 cm SH cells, not a separate store. The tile fill is just a scheduler that decides which of those cells get rays each frame.

A pick traces **one ray** (Lumen style), not a 16-ray quarter. The budget is spent every frame and never settles. In a dynamic game things keep moving, so this is a priority system for where the rays go:

`priority = closeness to the player × (remaining error + measured change)`, with the camera view raising priority.

Around the player, F2 maintains a 360-degree set of world-cache cells in every direction, independent of the camera view. Cells are created by distance from the player. Level of detail is a clipmap: 25 cm within `fill_lod_near` (starting at 8 m), then 25 cm × 2^level, doubling outward to the far view distance (`fill_far`). There is no hard cutoff where GI stops; far cells are huge, coarse, cheap in rays and memory, and low priority. The set is smart: create cells ahead in the direction of motion (velocity look-ahead), and as cells get far they coarsen, then evict. Create more aggressively than drop — the drop/coarsen leash is further than the create/refine distance (hysteresis). The set is updated as the player moves. The view does not decide which cells exist; it only raises their priority. On screen: about one need-weighted pick per 8×8 tile (~57,600 at 1440p). The off-screen share is spent across the rest of the set by priority, so a fast turn or a move forward lands on built-up data.

## 2. Tunables

Every number is a named setting with a starting value. None of these are frozen; they will be tuned later from measurements.

| Setting | Starting value | Role |
|---|---|---|
| `surface_cache.fill_budget` | 100,000 rays a frame (High) | Total rays every frame. Starting band 50–100k. At 1440p the on-screen share is ~57,600, the rest is off-screen. |
| `fill_tile` | 8 px (High) | On-screen tile size. |
| `fill_lod_near` | 8 m | Within this distance, cells stay at 25 cm. |
| `fill_lod_create` | 8 m at 25 cm, then doubles each level | Create/refine radius per LOD. Starting rule: the clipmap radii. |
| `fill_lod_leash` | 1.5 × create radius at that level | Drop/coarsen radius per LOD. Must be further than create (hysteresis). |
| `fill_look_ahead` | 0.5 s of player velocity | Extends the create envelope ahead of motion. |
| `fill_far` | the far view distance | Outer bound of the clipmap. LOD keeps doubling out to here; no separate GI cutoff. |
| `fill_lod_max` | derived from `fill_far` | Coarsest level. Far cells are huge and cheap. |
| `fill_far_share` | 0.05 of `fill_budget` | Minimum rays reserved for the coarsest/far levels. |
| `fill_offscreen_floor` | 0.25 of `fill_budget` | Minimum share spent across the 360-degree set outside the view, so a hungry screen cannot take the whole budget. |
| `fill_error_target` | 0.03 | A cell is sufficient when its MSME standard error over its mean is below this. Rays per cell then vary with the cell, not a fixed count. |
| `fill_prox_min` | 0.5 m | Closeness is `1 / max(distance from the player, fill_prox_min)`, the same weight as the error metric. |
| `fill_check_rate` | one same-seed ray per 3×3 block a frame | Sparse A-SVGF re-checks that produce dirtiness. Check rays come out of `fill_budget`. |
| `fill_change_weight` | 1.0 | Measured change is added to remaining error at this weight. |

## 3. Remaining error and dirtiness

**Remaining error** is the cell's MSME standard error relative to its mean. A cell is sufficient once that ratio is below `fill_error_target`. A flat wall may get there in a handful of rays; a dim corner can take many more. The thing we measure is remaining near error, not frames to 95% near.

**Measured change** (dirtiness) is an A-SVGF-style same-seed 1-ray re-check on a sparse sample of cells, giving a 0–1 lambda per block. Mover and lamp hints only nominate cells for a check; they never reset anything. A box walking past dirties the cells it actually changed. A slowly moving sun dirties everything a little, and nothing is thrown away wholesale.

## 4. Every ray counts the frame it lands

The 64-ray-set commit is removed. MSME used to update once per full set; #265 stores partial quarter sums and commits on a full set. That wait is gone. The resolve pass folds the frame's *n* deposited rays into the estimator with an *n*-ray weight. No waiting for 64 rays or a full set. The averaging window stays a ray cap (512 on High, the existing cap-8 figure), as a tunable, not a set count.

## 5. State audit: nothing in F2 to F5 needs its own store

(a) world-cache data, (b) per-frame scratch, (c) a parallel store.

| Stage and piece | Where it lives | Size and lifetime | Class |
|---|---|---|---|
| F2 pick and need inputs (own counts, `updated`, lamp ring) | existing entry words | 0 added | (a) |
| F2 deposit into cell SH-L1 and texel mean | existing entries, one writer in the resolve pass | 0 added | (a) |
| F2 deposit records | per frame, hashed by slot, sized to the budget | a few MB of scratch at the 100k start | (b) |
| F3 need weights, histogram, tile scratch | per frame, recomputed | about 1 MB | (b) |
| F2 player-centred clipmap (360 degrees, doubling out to `fill_far`) | real entries in the existing hash table, created and LOD'd by distance from the player, with look-ahead and a drop leash, updated as the player moves; a short pick list | under 1 MB of scratch, one frame | (a) plus (b) |
| F4 in-tile and ring sharing | a **seed** on the existing low-confidence channel (`lighting_sc_set_seed`); no `shared_count` field | 0 added | (a) |
| F5 path vertices | backward deposit into the cells the vertices hit; a short per-path scratch | per frame | (a) plus (b) |
| Lamp-change stamp | existing ring compared with `updated` | 0 added | (a) |
| Age stamps | existing visit word and `updated`; no new fill stamp | 0 added | (a) |
| **F1 shown-age stamp, cache region 26** | one vec4 a slot, written only under `GENOS_LIGHTING_SC_TRACE`, never read by compose | 16 MB at 1M slots if always allocated | **(c) as built**: allocate only when the trace is on, and drop it once F2 reports shown age from `updated` |

Added scratch for F2 to F5 is on the order of 10 MB, well inside the ~200 MB lighting budget. The 360-degree clipmap lives in the existing hash table, not a parallel store. Far coarse cells are few, so they stay cheap in memory. Off-screen deposits must refresh the visit stamp or the 1,800-frame eviction frees the cell; the drop leash and `fill_far` bound residency (ADR 0036). This set replaces scouts; physical deletion of the scout path stays at F6.

## 6. Slanted surfaces

A turned box has no 12.5 cm texel; those pixels already read the 25 cm SH cells. Under the one system a slanted pixel is a normal pick whose **target is the cell, not a texel**. The pick finds the cell by hash lookup, takes need from the cell's own counts and stamps, and deposits into the SH-L1 of the cells the read blends. No texel deposit, no probe. The F1 stop-mark age of 11 to 13 frames is the number F2 must remove. The open lamp-lag question in F0 and F1 stays with F2.

## 7. Stages

F2 to F5 stay in the same order. F2 is: wire the always-on single-ray scheduler into the world cache with the tunables, maintain the clipmap-like 360-degree cell set around the player (look-ahead, hysteresis, doubling out to `fill_far`), spend the off-screen share across it by priority, fold each frame's deposits into MSME the same frame, leave the due lists and settle path unused when the flag is on, and measure remaining near error. Physical deletion of the old scheduler stays at F6, once the flag is default-on. F3 adds the full priority terms and check rays; it does not claim the off-screen set.

The stage table in section 15 of the design is updated to match. Frozen quality gates (blotch 0.014, energy 0.9, linger, lag, turn 0.403) are unchanged. F2 also reports rays equal to the budget (always on) and yaw fps as a measurement, not a precomputed estimate.

## 8. Open questions

1. The MSME warm-up bound is today “k at most 1/(n+1) below 16 sets”. With per-ray updates, is that bound 16 rays or 16×64 rays? Starting rule: 16 rays, then tune.
