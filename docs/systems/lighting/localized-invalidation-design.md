# Bounded lighting: world-cache reuse, fixed budgets, priority scheduling

Status: **accepted** by Anthony Poschen, 26 Oct 2026. ADR: [0013](../../adr/0013-lighting-invalidation-is-local.md). Phase 0 records pass times and work counters. The frame now spends those caps: shader gates, a guard-band crop, a controller on the recorded lighting total, and 25 cm–4 m parents of the 12.5 cm lattice. Section 16 is what that code does and where it leaves this text. Terms follow [`docs/lighting/GLOSSARY.md`](../../lighting/GLOSSARY.md).

Revision 2 follows Anthony's review of rev 1. Real play spins the camera while things move. A screen-tile design dirties most tiles on a turn, and its "> 40% dirty or camera cut → full recompute" fallback has no upper bound. Rev 2 therefore:
- moves reuse into the persistent world-space surface light cache;
- gives every expensive pass a work-unit budget per preset, scaled later to hold a millisecond cap;
- deletes the full-frame fallback;
- replaces the global stillness hash with a priority scheduler;
- adds LOD and eviction that mirror densification.

## 1. Why (measured)

Stress/hall breakdown, master `27228dc4`, RTX 4070 on a shared GPU, so these are proportions (`/home/box/shared/moving-lighting-breakdown-2026-10-20.md`):

| pass | frozen | 3 boxes move | boxes + lamps move |
|---|---|---|---|
| direct | ~0.2 | 6.69 | 9.17 |
| denoiser (à-trous, 8 rounds) | ~0 | 5.92 | 5.97 |
| compose | ~0 | 2.69 | 2.70 |
| trace (screen probes + cache rays) | ~0 | 1.93 | 2.07 |
| probe filter | ~0 | 1.28 | 1.25 |
| gather | ~0 | 0.76 | 0.76 |
| **total lighting** | **0.43** | **19.95** | **22.56** |

Today one global hash (`frame_bytes` + mesh key + draw key in `lighting_passes`) switches the whole pipeline on or off. Cost is binary: 0.43 ms or about 20 ms, with nothing that caps it.

**Goal:** per-frame lighting cost ≤ a fixed preset cap in every situation, including camera spins with movers. Quality, not time, is what degrades under load: values get older but are always finished.

## 2. Reuse layer: extend the surface light cache, don't replace it

| | GIBS surfels (Halén et al., EA SEED, SIGGRAPH 2021) | Lumen surface cache (Wright et al., SIGGRAPH 2022) | **ours today**: surface light cache + WRC |
|---|---|---|---|
| Unit | disc: position, radius, normal | card texel in an atlas page | hashed world-grid **cache entry** on one of 6 axes, 8×8 **pages** |
| Spawn | 16×16 tile gap fill from the G-buffer | mesh cards captured at import | first sight + **pre-fill** (coarse → fine) |
| Size | screen-relative radius | card resolution / mip | **cache cell size** 25 cm near, doubling with distance |
| Persistence | yes; recycled stochastically | yes | yes; compact frees unread entries |
| Movers | transform/bone attached | moving cards | **object-local entry** (branch gibs-mover-attach-msme) |
| Blend | MSME | temporal accumulation | **MSME** (S72R, verbatim) |
| Budget | global ray budget, variance-driven | update-priority budget | ray budget / budget shift, trickle, dormant |
| Far field | trapezoid grid | WRC + global SDF | **WRC** |

**Decision: extend.** Our `docs/lighting/research/GIBS.md` verdict stands. The industry moved from surfels as *the* cache to screen probes plus a world hash cache (Lumen, AMD GI-1.0, kajiya), and we are already on that branch. Our entries are hashed, world-anchored and camera-independent (ADR 0012), with MSME, pre-fill and dormancy already working. What we take from GIBS:
- **§1.3 tile spawn throttle:** ≤ 1 new entry per screen tile per frame, randomised threshold, so a reveal can't spike.
- **§1.4 stochastic eviction**, adapted to coarsening (§6).
- **§1.6 screen-relative size**, reintroduced as **LOD levels of the hash** rather than a free radius. This is a deviation from GIBS; see §9.
- **§1.12 variance-driven rays plus a global budget**, made strict and per pass.

Nothing is replaced. The cache gains a level index (L0 = 25 cm … L4 = 4 m) in its key, a priority score, and an age.

## 3. Fixed per-frame budgets

Every expensive pass takes a **work-unit cap** per preset: rays, entry relights, pixel reshades, denoise items. Work units are the mechanism. Per-preset millisecond targets are calibrated from phase 0, and a later runtime controller scales the units so the frame holds that millisecond cap. Work that doesn't fit goes back into the priority queue (§5) and is never run late in a burst. The ms column is the cap on a 4070 at 1080p. Phase 0 records the pass times that calibrate it. The controller is not in phase 0.

| budget (per frame) | Potato | Low | Medium | High |
|---|---|---|---|---|
| screen-probe rays (probe tile × rays) | 0 | 16 px tile × 16 = 33 k | 16 × 32 = 261 k | 16 × 64 = 522 k |
| cache entry relights | 0 | 16 k | 32 k | 64 k |
| cache rays | 0 | 64 k | 128 k | 256 k |
| new entries (spawn, ≤ 1 per tile) | 0 | 2 k | 4 k | 8 k |
| coarsen/evict ops (§6, shares the cap) | 0 | 2 k | 4 k | 8 k |
| direct: fresh shadow rays (pixel-equivalents) | 0.5 M | 0.5 M | 1.0 M | 2.0 M |
| direct: lamp candidates per pixel (ReSTIR DI) | all lamps, no GI | 2, temporal only | 4, temporal + spatial | 8, temporal + spatial |
| denoiser: page items × iterations | — | 4 k × 1 | 8 k × 3 | 16 k × 3 |
| **lighting cap (4070 1080p, P0 calibrates)** | **1.5 ms** | **4 ms** | **6 ms** | **8 ms** |
| fixed screen resolve (§4) inside the cap | 0.8 | 1.5 | 2.0 | 2.5 |

The High cap of 8 ms plus about 0.4 ms of non-lighting gives the ~10 ms stress frame target. Older GPUs keep the same millisecond cap. The controller lowers the work units until the frame holds that cap. S72S already requires that nothing depends on a minimum ray count.

**The full-frame fallback is deleted.** Camera cuts, resizes, a 360° spin and a big reveal all run the same capped passes. The pixels they uncover read:
1. the nearest finished cache level (§6.4);
2. if there is none, the WRC;
3. if that is missing too, the ambient fill (never black).

These values are older or coarser, but they are finished, so nothing pops.

## 4. Screen space: only resolve and cheap per-pixel steps

The screen keeps only fixed-size work. It does not grow with motion:

| step | per frame | High est. |
|---|---|---|
| cache apply (interpolate entries at G-buffer points, L-blend) | all pixels, fixed | 0.6 ms |
| screen-probe gather + probe filter for the **budgeted** probes; others reuse kept hits | fixed by budget | 0.6 ms |
| ReSTIR DI resampling (candidates only; shadow rays are counted in the direct budget) | all pixels | 0.5 ms |
| compose | all pixels, one pass | 0.6 ms (today 2.7 ms; compose's own quiet path stays) |
| screen denoise, last iteration only, on the picture | all pixels | 0.2 ms |
| **fixed resolve total** | | **≈ 2.5 ms**, independent of motion |

Direct light: each frame, pixels get fresh shadow rays up to the budget, picked by priority (§5). A pixel without a fresh ray this frame reads its reservoir's lamp visibility from the **shadow cell cache** (12.5 cm per lamp, S74A, pushed). Low and Medium may reuse that cached shadow. Fresh shadow rays go to near-camera pixels first. Pixels inside a mover's shadow volume (§5.2) still rank for torn edges when the scheduler lands, which addresses S77B: those pixels get a fresh ray before reuse.

## 5. Priority scheduler (replaces the global stillness hash)

Each cache entry, screen probe and pixel-group (16×16 tile for direct) has a score. Each budget takes the highest scores and stops at its cap.

```
p = w_change * change_mag          // from change records
  + w_near   * screen_err          // projected size / pixel coverage near the camera
  + w_age    * min(age / age_max, 1)
  + w_new    * newly_visible       // first sight, disocclusion
  + w_var    * inconsistency       // MSME, as today
```

### 5.1 Inputs
- **Change records** (rev 1), now feeding priority instead of a flag. Mover: old and new AABB. Lamp: old and new position, intensity and reach. Sun: direction. Change magnitude = estimated irradiance change at the item. For a lamp, that is Δ(I/d²) inside reach. For a mover, it is the solid angle the swept AABB subtends from the item, times the lamp radiance it can block or bounce.
- **Shadow-caster volumes.** For each mover × lamp in reach: the hull of the lamp position and the mover's swept AABB, clipped to reach. The sun extrudes along its direction. Items inside get `change_mag` from the blocked fraction. This is VSM's rule: a moving caster invalidates "any pages that overlap its bounding box from the light's perspective" (verified, §9).
- **Bounce reach.** Pages within one cache cell of a changed page inherit `change_mag × albedo` for one hop per cache round, up to `bounce_count`. This is the WAKE ring rule at page granularity, taken as a design note and not from the dropped branch.
- **Near the camera.** `screen_err` = projected entry area × (1 + error proxy). Near-camera error matters most.

### 5.2 Selection
A GPU histogram of p is built in 64 log bins. Each budget walks the bins from the top until it hits its cap. One threshold is chosen per budget; there is no sort, and the cost is O(n).

### 5.3 Starvation protection
**10% of each budget is reserved** for a round-robin over all visible or live items ordered by age. Every live entry is touched at least every N = live / (0.1 × budget) frames: at most ~30 frames for visible entries on High. This replaces rev 1's verify trickle and fixes S78G, where trickle starved converging texels.

### 5.4 How it replaces the hash
"Still" is no longer a global state. A frozen scene has no change and no new visibility, so the top bins empty and most budgets run below cap. Cost falls toward the resolve cost plus the starvation slice, about 0.4–0.8 ms (today 0.43). A moving scene runs at or below the cap. `still_frames` stays only as a debug counter: frames since the last change record.

## 6. LOD, coarsening and eviction (mirror of densification)

Densification and eviction are the same operation in two directions. Fine levels are added near the camera when budget allows and removed from fine to coarse with distance or age. Detail is never dropped wholesale, so walking A→B→A never rebuilds.

### 6.1 Levels
The cache key gains a level L0–L4: 25 cm, 50 cm, 1 m, 2 m, 4 m. The target level for a surface point is the one whose cell projects to ~`target_px` pixels (High 8 px, Low 16 px), clamped so L0 is only used within `near_max` (High 6 m).

### 6.2 Hysteresis
- A fine level is **added** when the projected size exceeds 1.5 × target.
- It is **removed** only when the projected size is below 0.5 × target **and** it has gone unseen for `coarsen_age` frames (High 120, i.e. 2 s at 60 fps).
- That gives a 3× band plus a time lag, so back-and-forth motion or a spin doesn't thrash. A fine entry seen again inside the lag stays alive at full value.

### 6.3 Coarse layer that persists
L3 and L4 entries form the **coarse world layer**. The cap is fixed per preset: High 24 MB, Medium 16 MB, Low 8 MB. It does not grow with the world. When the layer is full, the farthest coarse entries are evicted, and a distance tie evicts the least recently seen. They keep MSME state. The WRC remains the fallback beyond that.

### 6.4 Eviction priority and budget
`e = w_d * distance_level_excess + w_t * frames_since_seen / coarsen_age − w_i * importance`. Importance means emissive, lit by a lamp, or flagged by the game (e.g. a waypoint).

Coarsening is a **merge**, not a delete. Before a fine entry is retired, its MSME mean and variance are accumulated into its parent (area-weighted, one level up). The parent therefore holds a finished summary of the detail it lost. These merge/retire ops come out of the "coarsen/evict ops" row of the budget table and share the per-frame cap. Under memory pressure, GIBS-style stochastic selection (§1.4, fixed seed for tests) avoids eviction waves.

### 6.5 Seeding fine entries on return (no pop)
1. A new fine entry starts with its parent's mean as its **finished value**, with variance raised by the parent's spread and treated as "young" (S72N seed rule). Pre-fill already requires that a new entry is never empty.
2. The cache apply blends levels by distance (a trilinear L-blend), so a fine level fades in over its hysteresis band rather than switching.
3. MSME blends new samples in once. The displayed value moves from the parent's estimate toward the fine one with no step larger than its noise.

## 7. No pops, no flicker

- **Finished values only.** Every value read by apply or compose is the output of a completed MSME update or reservoir, never a partial pass.
- **Blended once.** MSME on entries; the reservoir for direct. Compose doesn't blend again.
- **Deferral ages a value; it never blanks it.** The age is visible in the debug view.
- **The camera never resets history.** Stored light is world-space (ADR 0012).

## 8. Test scenarios and gates

All run at 1080p on the 4070 in the clean or exclusive window, 3 runs each, with `genos-lighting --headless` on ws6 silent. Per-frame GPU timestamps are needed for p99; phase 0 records them. Never genos-stress or port 8765.

```sh
genos-lighting --headless --menu hidden --scene stress --checkpoint hall \
  --preset high --gi-metrics --size 1920x1080 \
  --script debug/lighting/scripts/gates/s1-spin.rhai \
  --report genos-debug/gi/s1-spin
```

The same command with `s2-strafe.rhai` through `s6-settle.rhai`. `--out DIR` moves the tree. `GENOS_GI_METRICS` applies only when `--gi-metrics` was omitted. Each script writes `<out>/gi/<name>.csv` (p50 and p99) and `<out>/gi/<name>-frames.csv`. A second run of the same name writes `<name>-2.csv` and `<name>-2-frames.csv`. The live window `<out>/gi/frames.csv` is still replaced on reset. `--report` is the script report directory (`report.md`, `result.json`). A reused directory overwrites that report. It is not the metrics CSV. Each script checks that `lighting_ms` p99 is under `get_budgets.lighting_cap_ms`. Lavapipe checks that the files and the check compile. The millisecond result is the 4070, three runs.

| scenario | script |
|---|---|
| S1 spin | `debug/lighting/scripts/gates/s1-spin.rhai`. 360° yaw in 2 s at hall while lamps (dynamic 50) and boxes move; repeat 3× |
| S2 strafe | `debug/lighting/scripts/gates/s2-strafe.rhai`. 6 m/s lateral strafe for 3 s through the hall, movers on |
| S3 approach | `debug/lighting/scripts/gates/s3-approach.rhai`. Walk from 8 m to 0.3 m of the lamp-wall, then back, at 2 m/s |
| S4 many movers | `debug/lighting/scripts/gates/s4-many-movers.rhai`. Stress scale big is 16 sections × 3 movers = 48 moving boxes, plus 16 moving lamps. The building has no 64-box layout. The script logs that gap |
| S5 A→B→A | `debug/lighting/scripts/gates/s5-aba.rhai`. Walk hall → room-a → hall, 5 cycles, 4 s per leg |
| S6 settle | `debug/lighting/scripts/gates/s6-settle.rhai`. Boxes and lamps still, dynamic 0, sun frozen, hold 600 frames |

| gate | pass condition |
|---|---|
| G1 cap | p99 lighting ms ≤ preset cap, and max ≤ cap × 1.1, in S1–S5. Budget counters never exceed caps. |
| G2 convergence | S6: lighting-ref error vs the path-traced reference ≤ ref floor within **N = 120 frames on High, 240 on Low** |
| G3 no pops/flicker | frame-to-frame pop metric and `temporal_flicker.rhai` ≤ 1.2 × the frozen baseline in all scenarios |
| G4 A→B→A | no frame whose cache spawn + relight count exceeds its budget (no rebuild spike); on each return to hall, lighting-ref error vs the pre-departure capture ≤ 1.5 × floor within N = 60 frames, and ≤ floor within 120 |
| G5 frozen | frozen stress ≤ 0.8 ms lighting |
| G6 basic scene | `room` / `bounce_corner` with one moving lamp > 500 fps on High (frame < 2 ms) |
| G7 stress moving | stress moving frame ≤ ~10 ms on High |

## 9. Papers: what we take and where we deviate

| Source | What we take | Deviation (flagged) |
|---|---|---|
| **UE5 Virtual Shadow Maps, Caching** (Epic, *Virtual Shadow Maps in Unreal Engine*, UE 5.4–5.8 documentation, dev.epicgames.com, checked 2026-10-20) | Verified text: light movement or rotation "will invalidate all cached pages for that light"; a moving, added or removed caster invalidates "any pages that overlap its bounding box from the light's perspective"; separate static caching keeps two copies of depth so dynamic movers don't redraw static geometry (`r.Shadow.Virtual.Cache.StaticSeparate`); a Cached Page visualization (green cached, red invalidated). | We have no shadow maps. The caster-bounds rule raises **priority** of cache entries and pixel groups instead of invalidating pages. The static/dynamic split maps to static entries plus object-local mover entries. A lamp move raises priority inside its reach rather than flushing everything for that lamp. Rev 1 cited "SIGGRAPH 2022 course"; that was unverified and is removed. |
| Lumen (Wright et al., SIGGRAPH 2022) | Persistent surface cache with update priority and budget; WRC; screen probes as final gather | Lumen recomputes the screen gather and composite every frame at full size; our gather is budgeted. We store hashed entries, not mesh cards. |
| GIBS (Halén et al., SIGGRAPH 2021) | Persistence, tile spawn throttle, stochastic recycle, global ray budget, MSME | Not disc surfels. Size by hash LOD levels with hysteresis, not a free screen-relative radius (`GIBS.md` §1.6 said NO for the lattice; levels keep entries world-fixed). Coarsening merges into parents, which GIBS does not do. Pre-fill instead of GIBS's cold start. |
| SVGF (Schied et al., HPG 2017) | à-trous kernel and variance guide | It runs on budgeted pages in the cache plus one screen iteration, not every pixel × N iterations. |
| A-SVGF (Schied et al., HPG 2018) | Temporal gradients to detect change | The trigger is explicit change records. Gradients are replaced by the starvation slice plus the MSME inconsistency term. |
| ReSTIR DI (Bitterli et al., SIGGRAPH 2020) | Reservoirs, temporal + spatial reuse, Alg. 6 unbiased merge (`lamp_pick=reuse`) | Fresh shadow rays are budgeted and prioritized. Pixels without one use the shadow cell cache, so the result is biased until their turn (flagged). |

## 10. Drop notes checked

| Note | What failed | How rev 2 avoids it |
|---|---|---|
| RAYCEIL | Per-entry ray ceilings with deferral made the moving frame +1.35 ms worse | Caps limit the **total** per pass. Per-entry counts are unchanged. G1/G7 must show a frame drop or the phase is dropped. Watch for deferral overhead (the selection histogram must be ≤ 0.1 ms). |
| WORKCUT ph. 2 | Invented confirmed-visibility shade failed the picture | No new estimator: MSME, ReSTIR and the existing shadow cell cache only. |
| S77B | Pixel shadow reuse tore moving shadow edges | Pixels in mover shadow volumes get first claim on fresh rays. Reuse is only for low-priority pixels. |
| S78A | Per-slot scattered lists were slower | Selection by histogram threshold; passes iterate dense pages and tiles. |
| S79 | Half-res à-trous was slower and failed the picture | Full resolution kept. Fewer items, not lower resolution. |
| S72B | ReSTIR GI texel reuse made blotches | ReSTIR for direct only, with visibility on the final sample. |
| S78Q | ReSTIR DI pixel win hidden because probe hits looped all lamps | P4 measures the pixel direct dispatch alone; hits keep `hits` lamp pick. |
| S78N | Guess-and-verify reuse mismatched | Change records are explicit, not guessed. |
| S78G | Stop-at-cap trickle starved converging texels (+40–50% pop) | Reserved 10% starvation slice; young entries also have `w_new`. |
| S73A/S73B | Off-screen pre-fill took ray budget and darkened the picture | Off-screen work only from leftover budget, below on-screen bins. Coarse seeding costs no rays. |
| S72S | Must scale to GTX 1060; no minimum ray count | Budgets are caps; Low and Potato work at small caps. |
| WAKE / PR #31 | Ring launches didn't shrink the frame | Rings only add priority, never extra launches. Judged on per-dispatch ms. |

## 11. Debug views and MCP

- `--lighting-schedule budget|global` (mirror `GENOS_LIGHTING_SCHEDULE`). `global` is today's hash path, kept for A/B only. It is in `--help`, the system record, `EngineSettings`, the MCP catalogue and the debug menu.
- `--lighting-budget-scale F` scales every cap (test only; the preset sets the values).
- Debug views:
  - `priority`: heat map of p;
  - `age`: frames since an entry was refreshed;
  - `lod`: level colour per pixel, with the L-blend shown;
  - `budget`: per-pass fill bars;
  - `coarsen`: entries merged this frame.
- Counters on the Perf page and the `--lighting-debug` line: per-budget used/cap, deferred count, max age seen, starvation-slice coverage, spawns, merges, coarse-layer MB.
- MCP: `get_budgets` / `set_budgets`, `get_schedule_stats`, `lighting_lod_snapshot` (per-level entry counts and memory, used by the S5 gate).

## 12. VRAM (High, 1080p)

| item | MB |
|---|---|
| fine cache levels L0–L2 (today's cache + level key + priority/age 8 B per entry) | ~40 |
| coarse layer L3–L4 (cap) | 24 |
| ReSTIR reservoirs (current + previous, 16 B/px) | 66 |
| priority histogram, budget lists, change records | < 2 |
| shadow cell cache (existing) | ~10 |
| **total** | **≈ 140** (cap 200) |

Low: one reservoir (33 MB), coarse cap 8 MB, ≈ 75 MB total.

## 13. Phases (each its own PR and A/B)

| PR | Content | Gate |
|---|---|---|
| P0 | Per-pass GPU timestamps (`direct`, `atrous`, `compose`, `trace`, `filter`, `gather`, `resolve`) and work counters (rays, cache entries refreshed, lamps shaded), MCP `gi_metrics`, CSV under `./genos-debug/gi/`, and the gate scripts. Behaviour unchanged. Change records, the priority buffer, and the §11 debug views are later phases | overhead measured on the 4070; calibrates the ms caps. Lavapipe checks the wiring only |
| P1 | Budgeted cache relights + rays with the priority scheduler and starvation slice; global hash becomes debug only | G1 in S1–S2; G2; G5 |
| P2 | LOD levels, hysteresis, coarse layer, merge-coarsening, parent seeding | G4 (A→B→A), G3 |
| P3 | Budgeted screen-probe trace, gather and filter | G1 in S1–S4 |
| P4 | Direct: ReSTIR DI with budgeted, prioritized fresh shadow rays + shadow cell cache | G7, G6, S77B edge review |
| P5 | Denoiser on budgeted pages + one screen iteration; compose fixed | G1 all, G3 |
| P6 | Remove the full-frame fallback; `global` mode stays as A/B | all gates on all presets |

A phase that misses its gate is dropped under the standing drop rule: research note, close, delete the branch.

## 14. Rev-1 questions, answered under the budget model

1. *Fall back per pass or per frame at 40%?* Neither. The fallback is deleted. Every pass has its own cap and defers by priority.
2. *Previous reservoir on Low?* No. Low keeps one reservoir with temporal reuse only (−33 MB); spatial reuse starts at Medium.
3. *Is a 1/64 verify trickle enough?* It is replaced by the reserved 10% starvation slice per budget, which gives a bounded refresh period (≤ ~30 frames for visible entries on High), plus the MSME inconsistency term.

## 15. Open questions, decided 26 Oct 2026

Anthony Poschen approved these answers. Phase 0 does not implement the controller, the importance API, or the eviction policy. It records the pass times those answers are calibrated from.

1. Work units are the mechanism. Per-preset millisecond targets are calibrated from phase 0. A later runtime controller scales work units to hold the millisecond cap. "Same units, slower GPU" is not the rule.
2. Convergence stays 120 frames on High and 240 frames on Low for G2. G4 stays 60 frames to 1.5× the floor and 120 frames to the floor.
3. Yes: an optional game-side importance hint (§6.4), through settings and MCP, in a later phase. Not phase 0.
4. No: the coarse-layer cap stays fixed per preset (High 24 MB, Medium 16 MB, Low 8 MB). When the layer is full, evict the farthest coarse entries, then the least recently seen on a distance tie.
5. Yes: cached shadow reuse is allowed on Low and Medium. Near-camera pixels are prioritised for fresh shadow rays. A pixel without a fresh ray reads the shadow cell cache. Mover shadow volumes (§5.2, S77B) still rank for torn edges when the scheduler lands.

## 16. Code (26 Oct 2026)

Presets set schedule mode to `budget`. `schedule_push` multiplies the §3 caps by the controller scale (at most 1) and by `budget_scale` (1/64 through 4). Global mode pushes zeros and stays on the display size.

A pass gates when its grant is under its demand and the frame has a change record, or, for direct and trace, when the scale is below 1. The gate is one centered tile rect. Ten percent of that grant is a band that steps down the picture. À-trous gates only when a change record exists. Outside the rect, a direct pixel keeps a stored value, and a probe ray keeps a finished hit or a finished miss. An empty direct slot still traces. A cache ray past the grant does not run.

The lighting targets are the guard-band size. The picture is the crop. The eye stays put, and the band leads the turn. High starts at 15% of the display on each edge. Probe picks and lamp-pick history project that displayed camera through the crop, so a still probe finds its last tile.

With `--gi-metrics`, the next frame reads the retired `lighting_ms` against that frame's cap. Over the cap, the scale becomes `cap / ms`, down to 1/64. Under 75% of the cap, the scale grows by 5% up to 1. Picture tests do not record, so a lavapipe timestamp does not pull the scale down.

Budget mode with world-cache mode 3 reads parents of an empty 12.5 cm cell at 25 cm, 50 cm, and 1 m. A finished parent (`v.w > 0.5`) is what a waiting pixel shows. Cells at 2 m and 4 m are claimed and are not read. The walk is split, because lavapipe faulted on one five-step body:

- `lighting_sc_parent` (ray hits, and pixels that are not on the seed-split path) loops a constant three times. A runtime trip count faulted lavapipe. From 12.5 cm that is a 25 cm seed claim, a 50 cm read, and a 1 m read. From the 25 cm hit patch that is a 50 cm seed claim, a 1 m read, and a 2 m read.
- `lighting_sc_parent_bilerp` (seed-split pixels, preset high) stays at two bilinear steps, 25 cm then 50 cm. The 25 cm seed claim is the existing one and is not charged to the spawn cap.
- `lighting_sc_level_tail` then starts at the size the walk already reached and doubles twice, stopping above 4 m. A 12.5 cm start therefore claims among 1 m, 2 m, and 4 m. A 25 cm hit start claims among 2 m and 4 m. At most one missing cell is claimed. The claim is counted after the insert. An over-cap key is cleared. Cells at 2 m and 4 m also take a coarse slot (High 24 MB at 368 bytes). The picture reads a finished 1 m parent only when the reader sits in the half-size cell of the point that parent was lit from. It does not read 2 m or 4 m.

The slot table stays 1,048,576 entries. A cell that already holds a finished value does not read the parent.

Departures from this design:

- §5.2's GPU histogram is not built. `select_refresh` is the tested CPU order. It does not pick GPU work. The screen gate is one rect, taken from direct, then à-trous, then trace.
- §5.1's irradiance change, shadow-caster volumes, and bounce reach are not inputs. The gate sees the change-record count.
- §6.1's projected-pixel target does not pick a level. The shown lattice stays 12.5 cm (S57). Coarser sizes are parents of that key.
- §6.4's area-weighted merge is the CPU function `merge_into_parent`. The GPU claims a missing parent empty. `should_remove_level` does not run on the GPU. A fine cell is not dropped because it left the view. Coarse cells leave on the expiry sweep once the live coarse count is over the slot cap, not farthest-first.
- §6.5 does not seed a new fine cell from the parent's mean, and there is no level blend.
- A 2 m or 4 m world-grid cell covers both sides of a corridor wall. Reading it from the compose tail painted the outside floor with the interior value (lavapipe, 1280×720, outside brightness about 75–170 against about 5 with the level bit clear). That tail therefore does not read those sizes. Its 1 m read also requires the half-size cell of the lit-from point. A ray hit's three-step walk can still return its own 2 m step. An occlusion test in the tail would inline the scene marcher into compose. With that limit, the same outside sample measured 0 and 6.
- The parent find cannot be one five-step loop on lavapipe. A runtime trip count faulted it. Five bilinear steps overflowed it. A fourth copy of the `lighting_sc_parent` body overflowed it, including a helper whose extra finds survived an early return. The code keeps a constant three-step find, the original two bilinear steps, and `lighting_sc_level_tail` for 1 m through 4 m. That tail is a nearest read, not a bilinear. A ray hit starts at 25 cm, so its 1 m step stays a read inside the three-step find and is not claimed there. The tail stops at 4 m. Without that cap a 25 cm start continued to 8 m.
- One parent claim is counted after the hash insert, not before it. A rejected coarse claim still consumes the spawn count. The 25 cm seed claim stays outside that cap.
- The denoise row's iteration counts are not applied. High still runs 5 iterations. The rect is the cut.
- §12's ~140 MB figure is missed. The hash table stays about 368 MB. The coarse cap only limits how many 2 m and 4 m cells stay inside that table.
- A cell larger than 0.2 m shares the world-radiance refresh lists, so a new 1 m, 2 m, or 4 m cell competes with those refreshes.
- The spawn counter is cleared on the graphics queue. An overlapping world-cache submission can claim about twice in one frame.
- `lighting_lod_snapshot` reports the caps. It does not read the live coarse count.
- The controller is one scale on the lighting total, not a scale per pass. It moves on the frame after the sample, so the first moving frames can sit over the cap.
- The gate scripts assert p99 under the preset cap. They do not assert max ≤ cap × 1.1, and they do not count rays against the caps. Lavapipe cannot show the 8 ms cap.
- `--report` is the script report directory. The unique file is `<out>/gi/NAME.csv`, then `NAME-2.csv`.
- §11's `priority`, `age`, `lod`, `budget`, and `coarsen` views are not built. The probes are `get_budgets`, `set_budgets`, `get_schedule_stats`, and `lighting_lod_snapshot`, plus the Perf rows `schedule_stats` and `lod_snapshot`.
- The phase table above is the original cut. This change lands the screen gates, the guard crop, the controller, and the parent levels together.
