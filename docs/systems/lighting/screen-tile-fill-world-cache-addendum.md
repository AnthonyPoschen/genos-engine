# Screen-tile fill: one system, a constant spend (addendum)

Status: **Proposed** (11 Nov 2026), addendum to [the screen-tile fill design](screen-tile-fill-design.md). Docs only. Origin: Anthony, 11 Nov 16:59 and 17:03: lean on the one world cache, and spend a large fixed ray budget every frame instead of reacting to demand. Where this note and the design disagree, this note wins. Numbers marked **est** are calculations from the measured rows named beside them, not renders. Code read: `lighting_cache.glsl`, `lighting_cache_slots.glsl`, `lighting_screen.comp` and `shown_age.rs` on #252 (`03ab0952`).

## 1. The point: a fixed spend instead of a demand spike

Today the ray count is the number of entries that became due. On the 4070 (1440p, High, hall; `pc-cap8-1111`, `pc-quarters-1111`) that gave 1.40M a frame at the first fill and 1.47M in the lamp hold, at 35 fps, against 65 to 67 fps and 0 rays when settled. The tile fill reverses it: **the budget is the input and the work is chosen to fit.**

- **Unit.** A pick is one origin that traces one 16-ray stratified quarter, the unit the cache and #265 already use. So the budget is in rays: `picks = budget / 16`. This corrects the design, which counted one ray per pick and so capped spend at 230,400 rays (4 picks x 57,600 tiles). At 16 rays a pick, the 57,600 base picks are 0.92M rays, the same spend as #265's 1M. The pick count is not the limit; the ray budget is.
- **Spent every frame, in this order.** (1) On-screen tiles by need, nearest first. (2) Whatever is left goes to cells near the eye that are off screen or behind the camera (the virtual tiles of section 5.5, now the main use of the remainder). A settled frame has no on-screen need, so its whole budget builds the surrounding 360 degrees instead of spending 0. A small spread-out floor (`tiles/16` picks) stays on top.
- **Tunable.** `surface_cache.fill_budget` (rays a frame) per preset, plus `fill_radius` (how far from the eye off-screen building reaches, default 12 m **est**). The 4070 price of a ray is about 11 to 13 ms per million (from the fill, flag-off and lamp rows: 23.5, 28.6 and 25.6 ms against 14.9 to 15.4 settled).

| Preset (4070 1440p unless noted) | Budget a frame | Frame fps while spending (**est**) | Frames to 95% near (**est**, about 35M rays) | Build the sphere, about 200M to 600M rays (**est**) |
|---|---:|---:|---:|---:|
| High, measured #265 fill | up to 1M (mean 0.75M) | 42.5 (measured) | 56 (lavapipe, F0) | 200 to 600 frames, 5 to 15 s |
| High, constant | 1M | 38 | 35 | 200 to 600 frames, 5 to 16 s |
| High, light | 500k | 48 | 70 | 400 to 1,200 frames, 8 to 25 s |
| Medium (16 px tiles) | 250k | 56 | 140 | 800 to 2,400 frames, 14 to 43 s |
| Low (probes today, no 1060 data) | 100k | 62 | 350 | not recommended before 1060 numbers |

Sources: 200M rays in the first 200 frames is the flag-off hall fill measured on the 4070 (design section 2) and counts the frustum; the sphere is **est** 1 to 3 times that, with near first. About 35M rays to 95% near is read off F0: 27 frames at a 1.0 to 1.4M peak, 56 at 0.75M mean. Ray efficiency (F0.5) is unmeasured, so some of those rays are waste and the real numbers may be better.

What the table says:

- A constant 1M costs 38 fps for as long as there is work. That is 3 fps better than today's 1.47M spike (35 fps) but it never goes away. A 500k budget is 48 fps, a 250k budget is 56. Any constant spend is below the settled 65 to 67 fps until the sphere is built.
- **Proposed spend rule:** constant while any cell within `fill_radius` is under target, then drop to the floor. Spending 1M on a finished room buys nothing and costs the settled fps (open question 1).
- **Gates.** Quality gates (blotch 0.014, energy 0.9, linger, lag, turn 0.403) are unchanged. The still hall at 1M held near error 0.0602 to 0.0607, so 57,600 to 62,500 picks of 16 rays meet it if they match #265's ray count. Two proposed (not frozen) perf gates from design 14.4 are at risk and need Anthony's call: **frames to 95% near at most 27** (it takes about 1.3M a frame; a budget of 1M or less gets 35 unless need-first allocation saves rays) and **yaw fps at least 51.8** (a constant 500k is about 48; the yaw spend alone was 130k, but yaw costs more than its rays, so F2 measures it).

## 2. State audit: nothing in F2 to F5 needs its own store

(a) world-cache data, (b) per-frame scratch, (c) a parallel store.

| Stage and piece | Where it lives | Size and lifetime | Class |
|---|---|---|---|
| F2 pick, need inputs (own sets, `updated`, lamp ring) | read from the existing entry words (`v[3s+2].w`, region 6 `.y`, ring in `ckeys`) | 0 added | (a) |
| F2 deposit into cell SH-L1 and texel mean | the existing entries, one writer in the resolve pass | 0 added | (a) |
| F2 deposit records | per frame, hashed by slot, sized to `2 x picks` (125k x 64 B at 1M) | about 8 MB, one frame (was 14.7 MB) | (b) |
| F3 need weights, water-filling histogram, tile scratch | per frame, recomputed | about 1 MB | (b) |
| F3 virtual tiles and off-screen picks | targets are real entries claimed in the existing hash table; a short pick list only | under 1 MB, one frame | (a) plus (b) |
| F4 in-tile and ring sharing | enters as a **seed**: the existing low-confidence channel (`lighting_sc_set_seed`), never an own set. This replaces the design's per-entry `shared_count` | 0 added | (a) |
| F5 path vertices | backward deposit into the cells the vertices hit; a 3-record scratch per path | per frame | (a) plus (b) |
| Lamp-change stamp | the existing ring compared with `updated` (`lighting_lamp_delta`) | 0 added | (a) |
| Age stamps | the existing visit word (`v[3s].w`) and `updated`; the design's "fill stamp" is not added | 0 added | (a) |
| **F1 shown-age stamp, cache region 26** | one vec4 a slot, written only under `GENOS_LIGHTING_SC_TRACE`, never read by compose | 16 MB at 1M slots, always allocated | **(c) as built**: redesigned below |

The only real (c) is region 26. It is a diagnostic: it exists to measure the lamp lag, and it costs as much memory as the whole fill design adds. Fold or drop it: allocate the region only when the trace is on, and delete it once F2 reports shown age from the existing `updated` word. Added VRAM for F2 to F5 falls from about 16 MB to about 9 MB (**est**), well inside the ~200 MB lighting budget.

Two consequences of building off-screen data. An off-screen entry is not read, so a deposit must refresh its visit stamp or the 1,800-frame eviction frees it. And `fill_radius` bounds residency (ADR 0036 measured 153k live entries in the stress scene, 407k worst, in a 1M-slot table).

## 3. Slanted surfaces under one system

On lighting-v3, a turned box (flag 2048, 6,922 of the 7,756 red-box pixels) has no 12.5 cm texels and shows screen probes (6,689 of 6,922; the 25/50 cm patches under it were 67 to 80 frames old). On #252 and #265 the probes are gone and those pixels read the 25 cm SH cells, blended over 8 corners at the true normal (`lighting_sh_read`), 0.4 to 0.6 frames old mid-move and 11 to 13 at the stop (F1).

Under the one system a slanted pixel is a normal pick whose **target is the cell, not a texel**. The pick finds the cell by the hash lookup (the screen pass stores only fine-texel slots, so there is no owner entry to read), takes `need` from the cell's own sets and stamps, and deposits into the SH-L1 of the cells the read blends. No texel deposit, no probe. Sharing helps here most: a 25 cm cell covers four texels of area, so a first-hit at least 2 m away serves a whole turned face. The F1 stop-mark age of 11 to 13 frames is the number F2 must remove, and the open lamp-lag question in F0 and F1 stays with F2.

## 4. Stage table

The stage table in section 15 of the design is updated: F2 and F3 take the changes above; F4 sharing uses the seed channel; F5 to F7 are unchanged.

## 5. Open questions for Anthony

1. Spend rule: constant only while cells within `fill_radius` are under target (proposed), or a true constant forever (38 fps at 1M, 48 at 500k, for no new data once built)?
2. The proposed frames-to-95%-near gate (at most 27) needs about 1.3M a frame. Keep it and allow a 1.4M burst, or restate it against the ray-count budget you pick?
3. Per-preset defaults above are **est**: High 1M or 500k? Medium 250k? Low waits for 1060 data.
