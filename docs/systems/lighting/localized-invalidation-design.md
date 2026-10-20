# Localized invalidation: design

Status: **draft for review** (2026-10-20). ADR: [0013](../../adr/0013-lighting-invalidation-is-local.md). No engine code yet. Terms follow [`docs/lighting/GLOSSARY.md`](../../lighting/GLOSSARY.md). The new terms (*change record*, *dirty tile*, *dirty reason*, *invalidation mode*) are added there in this PR.

## 1. Why

The stress/hall breakdown (master `27228dc4`, RTX 4070 on a shared GPU, so the numbers are proportions; `/home/box/shared/moving-lighting-breakdown-2026-10-20.md`):

| pass | frozen (settled 0.43 total) | 3 boxes move | boxes + lamps move |
|---|---|---|---|
| direct | ~0.2 | 6.69 | 9.17 |
| denoiser (à-trous, 8 rounds) | ~0 | 5.92 | 5.97 |
| compose | ~0 | 2.69 | 2.70 |
| trace (screen probes + cache rays) | ~0 | 1.93 | 2.07 |
| probe filter | ~0 | 1.28 | 1.25 |
| gather | ~0 | 0.76 | 0.76 |
| total | 0.43 | 19.95 | 22.56 |

One global hash (`frame_bytes` + mesh key + draw key in `lighting_passes`) decides "still" for the whole frame. Three boxes covering a few percent of the screen therefore cost the same as moving everything. The goal is that cost scales with the **dirty area**: lighting ≈ settled cost + dirty share × full cost.

## 2. Prior art, and where we deviate

| Source | What we take | Deviation (flagged) |
|---|---|---|
| UE5 Virtual Shadow Maps, cache invalidation (Epic, UE 5.0 docs / SIGGRAPH 2022 course) | A caster's old and new bounds invalidate only the shadow pages they touch. Static and dynamic casters are kept apart. | We invalidate **screen tiles and cache pages**, not shadow-map pages. We have no shadow maps; shadows are traced rays. The caster bounds are extruded toward each lamp in reach (§4.2). |
| Lumen (Wright et al., SIGGRAPH 2022) | Surface-cache pages carry their own update priority. Moving objects get a fast update. Screen probes are rebuilt only from the probes. | **Deviation:** Lumen re-gathers every screen probe and composites every pixel each frame. We skip clean tiles entirely and keep their finished picture. |
| SVGF (Schied et al., HPG 2017) | The à-trous kernel and variance guide are unchanged. | **Deviation:** SVGF filters the full frame. We filter dirty tiles grown by the kernel footprint and read clean neighbours' finished values at the border (§5). |
| A-SVGF (Schied et al., HPG 2018) | Temporal gradients drop history where shading changed. | **Deviation:** the trigger is the scene's own change records (exact, free). Sparse gradients are only a **safety net**: a rotating 1/64 of clean tiles is re-shaded; a gradient above noise dirties the tile and logs `dirty reason = verify`. |
| ReSTIR DI (Bitterli et al., SIGGRAPH 2020) | Per-pixel lamp reservoirs, temporal plus spatial reuse, unbiased merge (Alg. 6). This is `lamp_pick` with `reuse` and `unbiased` on. | **Deviation:** reservoirs are rebuilt only in dirty tiles. Clean tiles keep their reservoirs and finished direct light. Spatial reuse never reads across into a dirty tile from a clean one or back. |

## 3. Data structures (1920×1080; tile = probe tile = 16 px, so 120×68 = 8160 tiles)

| Buffer | Layout | Size |
|---|---|---|
| Change records (CPU → GPU ring) | per record: kind (mover/lamp/sun), old AABB, new AABB, lamp old/new position and reach, frame id; 256 records × 2 frames | ~32 KB |
| Dirty tile mask | per tile: u32 reason bits (direct, shadow, bounce, disocclusion, verify, camera) + u16 age + u16 dilation level; double-buffered | 8160 × 8 × 2 ≈ 130 KB |
| Dirty tile list | indirect-dispatch list of tile ids, one per consumer pass (direct, à-trous, compose, gather/filter, trace) | 5 × 8160 × 4 ≈ 165 KB |
| Tile depth bounds | per tile: min/max view depth + world AABB from the G-buffer, built in the gbuffer pass | 8160 × 32 ≈ 260 KB |
| Cache page dirty bits | 1 bit per page (8×8 entries) + u8 bounce distance; reuses the existing **dirty list** | < 1 MB |
| ReSTIR DI reservoirs | per pixel: lamp id, W, M, target pdf (16 B); current + previous | 2 × 33 MB = 66 MB |
| Finished direct light (held) | per pixel RGB16F, read back by clean tiles; today's direct output is reused where possible | ~8 MB |
| **Total new** | | **≈ 75 MB** |

Today's lighting state is about 10–20 MB plus the S78X lists (+20 MB). The new total stays near 115 MB, inside the ~200 MB budget. Potato/Low may drop the previous reservoir (−33 MB) and use temporal reuse only.

## 4. How dirtiness spreads

Every frame the CPU emits change records for whatever really moved: mover transforms, lamp position/intensity/radius, and the sun. The camera is not a change record. It reprojects (§4.4). One compute pass, **mark**, turns the records into tile and page bits. Each reason below sets its own bit, so the debug view can show why a tile is dirty.

### 4.1 Direct (lamp change)
A lamp that moved or changed dirties every tile whose world AABB intersects the union of its old and new reach spheres. A lamp switched off dirties its old sphere; one switched on, its new sphere. This is the existing graded lamp change (S78I) moved from all slots to tiles.

### 4.2 Shadow (shadow-caster bounds), as in VSM invalidation
For each mover *m* and each lamp *l* whose reach meets *m*'s swept AABB (old ∪ new), the **shadow volume** is the convex hull of *l*'s position and that swept AABB, clipped to *l*'s reach. Tiles whose world AABB intersects the volume get the `shadow` bit. The sun extrudes the swept AABB along the sun direction to the scene bounds. The test is conservative: it may over-mark but never under-marks. Hull-versus-AABB uses separating axes (the 6 AABB faces and the hull's silhouette planes, ≤ 14 planes).

### 4.3 Bounce reach (GI)
- **Cache pages:** pages whose entries lie in the mover's swept AABB grown by one cache cell size, plus pages inside a changed lamp's reach, get bounce distance 0. Each cache round, a page next to a distance-*d* page whose irradiance changed by more than its inconsistency threshold becomes distance *d*+1, up to `bounce_count` (cap 3). This is the WAKE ring rule (`WAKE-neighbour-wake-design.md`) applied to page granularity; see §8 for why the ring branch itself is not reused.
- **Screen:** a screen probe whose kept hits read a dirty page sets the `bounce` bit on its tile. The kept-hit → page index already exists (S78O).

### 4.4 Disocclusion and the camera
- A mover's old screen footprint (last frame's projected AABB) is dirtied with the `disocclusion` bit: what it uncovers has no valid finished value.
- Camera motion: tiles whose reprojection fails (depth or normal test, as in SVGF) get the `camera` bit. A camera cut, a resize, or a dirty share above `local_max_share` (default 40%) takes today's full-frame path for that frame (S78X2 dense fallback, generalized).

### 4.5 Growth for kernels
Each consumer grows the mask by its read footprint: the probe filter by 1 tile, à-trous round *i* by ⌈2^i × step / 16⌉ tiles, and the ReSTIR spatial reuse radius (30 px) by 2 tiles. Grown tiles are recomputed **in full**, not partially.

## 5. No pops, no flicker

1. **Finished values only.** A dirty tile runs the whole chain in the same frame: direct → gather → probe filter → à-trous rounds → compose. Its published picture is replaced only with that frame's finished output. A clean tile keeps last frame's finished picture, untouched. Nothing is half-updated.
2. **Blended once.** History blending happens in one place per quantity: MSME on cache entries, and lamp reuse (the reservoir) for direct. Compose writes; it does not blend again. This avoids S77B's torn moving shadow edges, which came from reusing shadow results partway across a mover's edge.
3. **Seams.** Kernels in a grown tile read clean neighbours' finished values (valid by rule 1). The grown ring is recomputed with the same kernel, so the boundary matches the full-frame result up to noise. Acceptance: a `lighting-ref` picture diff of local vs `--lighting-invalidation global` on the same frames stays under the ref noise floor (`scripts/verdict/noise-floor.py`).
4. **Stale tiles are bounded.** The verify trickle (§2, A-SVGF row) and a max tile age (`local_max_age`, default 600 frames) force a recompute. Nothing can stay wrong indefinitely.

## 6. Measurement plan

Exclusive or clean-GPU window only, 3 runs each, on zanven-pc RTX 4070. `--gpu-times 60`, `GENOS_GPU_DISPATCH=1`, `genos-lighting --headless --size 1920x1080 --checkpoint hall`, proof window on ws6 silent. Never genos-stress or port 8765.

| scene | today (proportional) | target |
|---|---|---|
| stress, boxes only | 19.95 ms lighting | ≤ 6 ms lighting |
| stress moving (boxes + lamps) | 22.56 ms lighting / 23.2 frame | **toward ~10 ms frame** |
| frozen stress | 0.43 ms | unchanged (≤ 0.5) |
| basic scene (`room` or `bounce_corner`, one moving lamp) | to be measured in P0 | **> 500 fps** (frame < 2.0 ms) |
| picture | — | local vs global under the ref noise floor; no pop metric regression |

Every phase records: dirty-tile share and its reason breakdown, the per-dispatch table (still / boxes-only / moving), and a lighting-ref diff. A phase passes only if the moving frame drops and the still frame is not worse. Otherwise drop rule: research note, close, delete the branch.

## 7. Debug views and MCP hooks

- `--lighting-invalidation local|global` (mirror `GENOS_LIGHTING_INVALIDATION`): the A/B switch; `global` is today. It is in `--help`, the system record, `EngineSettings`, the MCP catalogue and the debug menu (GI page), per settings-principles.
- Debug view `dirty`: a tile overlay coloured by reason (direct, shadow, bounce, disocclusion, verify, camera), with the grown ring hatched. Dump-able with `--frames N` and the `captures` tool.
- Debug view `dirty-pages`: cache pages tinted by bounce distance.
- Counters on the `--lighting-debug` line and the Perf page: `dirty_tiles`, `dirty_share`, per-reason counts, `grown_tiles`, `fallback` (yes/no + why), `verify_hits`, `max_age_forced`.
- MCP: `get_invalidation` (mode, last frame's counters, record count), `set_invalidation` (mode, `local_max_share`, `local_max_age`, verify rate), `dirty_tiles` (the list for a frame, for scripted checks).
- Probe rule (debug-standard): every failure mode has a counter. An under-mark shows up as `verify_hits` > 0 with reason `verify`.

## 8. Not repeating the drop notes

| Note | What failed | How this design avoids it |
|---|---|---|
| RAYCEIL | Fewer rays per entry made the moving frame +1.35 ms worse | Ray counts stay as they are. Only the **area** shrinks. |
| WORKCUT phase 2 | Confirmed-visibility shade (local invention) failed the picture | No new shading estimator. Dirty tiles run the existing passes unchanged. |
| S77B pixel shadow reuse | Torn moving shadow edges | Shadow results are never reused inside a dirty tile. The whole tile is re-traced. |
| S78A list-driven à-trous | Per-slot lists: atomic order and locality cost more | Dense 16×16 tiles, one workgroup each, indirect dispatch. |
| S79 half-res à-trous | Slower than the scalar wavelet; picture failed | Full resolution inside dirty tiles. No half-res. |
| S72B ReSTIR GI texel reuse | Blotches from reuse without visibility | ReSTIR is for **direct** only, with a visibility ray on the final sample, and spatial reuse stays within dirty tiles. |
| S78Q ReSTIR DI pixels (shelved) | Probe-ray hits still loop all lamps, so the pixel win was hidden | Phase P4 measures the pixel direct dispatch alone. Hits keep `hits` lamp pick. |
| S78N reuse with changes | Verify mismatched often; moving lamps relit anyway | Lamp change is an explicit record (§4.1), so there is no guess and verify. |
| WAKE / PR #31 | Ring launches did not shrink the frame; branch too old | Only launch-size reductions count. Every phase is judged on per-dispatch ms, on a fresh branch from master. |
| S80-verify | Never measured | Measurement is a phase gate (§6). |

## 9. Phases (each a PR with its own A/B)

| PR | Content | Behaviour change | Pass gate |
|---|---|---|---|
| P0 | Change records, mark pass, dirty-tile/page buffers, `dirty` views, counters, MCP. Consumers still run full frame. | none | Overhead ≤ 0.1 ms; dirty share measured on stress, boxes-only and the basic scene (sets the real targets) |
| P1 | Compose + gather + probe filter run on dirty tiles (compose's existing 8×8 quiet path is fed by tiles) | yes | Moving: compose + gather + filter drop roughly in proportion to dirty share; picture diff under floor |
| P2 | à-trous on grown dirty tiles, dense per tile | yes | à-trous ≤ dirty share × 6 ms + 0.3 |
| P3 | Direct on dirty tiles with shadow-caster bounds (§4.2) | yes | Boxes-only direct ≪ 6.7 ms; moving shadow edges pass review on the proof window |
| P4 | ReSTIR DI (`lamp_pick=reuse`, unbiased) in dirty tiles | yes | Moving direct drops further; basic scene > 500 fps |
| P5 | Screen-probe trace + cache-page bounce reach (§4.3) | yes | Trace scales with dirty share; GI lag in the stress script no worse |
| P6 | Remove the global hash as the trigger; `still` = no dirty tiles for N frames; `global` stays as an A/B mode | yes | Stress moving frame toward ~10 ms; frozen unchanged |

## 10. Open questions for Anthony

1. Should `local_max_share` (40%) fall back per pass or per frame?
2. Should we spend the 33 MB previous-reservoir buffer on Low, or use temporal-only reuse there?
3. Is a 1/64 verify trickle enough as the safety net, or should verification follow the existing verification budget?
