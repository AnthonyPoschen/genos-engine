# Lighting renames (proposal)

Status: proposal, not yet applied. Terms are defined in [GLOSSARY.md](GLOSSARY.md). Audited 2026-10-14.

## Rename table
Policy: keep old CLI/config/env/MCP names as deprecated aliases that log one warning. Nothing is removed in the same PR.

| # | Current | Proposed | Reason | Blast radius | Breaking? |
|---|---|---|---|---|---|
| 1 | `world_cache` group, "surface cache", `GENOS_LIGHTING_SC_*`, `WORLD_CACHE` | `surface_cache.*`, env `GENOS_LIGHTING_SURFACE_CACHE_*`, prose "surface light cache" | synonym (3 names) + collision ("world cache" vs WRC) | lighting.rs `WorldCacheSettings`; keys world_cache/world_cell/world_refresh/world_stale; ~10 env vars; lighting_gpu.rs; debug menu; MCP settings JSON | no |
| 2 | patch / slot / texel / cell | **cache entry**; slot = index only | synonym (4); "texel" implies cards we don't have | comments in lighting_cache.glsl, compact.comp; docs/experiments | no (internal) |
| 3 | `shadow_filter` hard/cell | `shadow_cache` off/cell | drift: it is a cache, not a filter | ShadowFilter enum, key, env, debug dropdown, MCP | no |
| 4 | `world_mode` 0/3 (`GENOS_LIGHTING_WORLD`) | derive from `surface_cache.enabled`; internal enum `gi_path` | magic numbers; duplicates #1 | GiSettings, key, env, tests | no |
| 5 | denoise / denoiser / `denoise_dirty` / `GENOS_LIGHTING_ATROUS_DIRTY` / `dirty_fraction` | `denoiser.*`, env `..._DENOISE_DIRTY` | synonym | lighting.rs keys, 1 env | no |
| 6 | "probe" = screen probe, v1 world probe, ring probe, verification probe | screen probe only; **wake ring entries**; **verified kept hits** | collision | CHG_W_/CHG_V_ comments, debug counter labels | no |
| 7 | "hysteresis" (value hold) | **hold** (`hold_tolerance`) | literature collision: DDGI hysteresis = blend weight | lighting_direct.comp, S78T comments | no |
| 8 | `probe_stale` / `staleness_frames` / `world_stale` | `*_stale_frames` beside `*_refresh_frames` | inconsistent suffix | keys + 2 env | no |
| 9 | `lamp_pick_rays` = `lamp_pick_samples` | `lamp_pick_samples` canonical | synonym | 2 flags, 2 env, slider, MCP | no |
| 10 | lamp pick `reuse`, `both` | `restir` (alias `reuse`); `hits+pixels` | literature; vague | LampPickMode, dropdown | no |
| 11 | `GENOS_LIGHTING_TILE` | `GENOS_LIGHTING_PROBE_TILE` | mirror ≠ key `probe_tile` | 1 env | no |
| 12 | `GENOS_LIGHTING_FILL` | `GENOS_LIGHTING_PREFILL_DEBUG` (or merge into `prefill`) | collision with prefill / ambient fill | lighting_gpu.rs ~L2091 | no |
| 13 | `lighting_filter.comp`, GPU bucket `filter` | `lighting_probe_filter.comp`, `probe_filter` | collision with denoiser/shadow filter | 1 shader, GpuBuckets, GPU_MS labels | yes for GPU_MS parsers unless both labels print for one release |
| 14 | env-only `URGENT, BORROW, MOVED_DROP, YOUNG_ONLY, CACHE_PICKS, CACHE_HZ, SC_SETTLE, WORLD_CAP` | promote the live ones to config/debug/MCP with descriptive names (e.g. `cache_young_only`); delete dead ones | settings rule; opaque names | lighting_gpu.rs ~L350–380, ~L2090–2100 | no |
| 15 | gi-design.md "cards" / "world probe bricks" | state the as-built surface light cache (ADR 0012) | doc drift | 1 doc | no |
| 16 | `residency_mb`, `--material-residency-mb`, `GENOS_MATERIAL_RESIDENCY_MB` | VRAM budget plus `texture_quality` | the fixed cap was file order, not distance, and too small | `MaterialSettings`; the old key warns and is ignored | no |

**Synonym groups: 9** (surface/world cache/SC; patch/slot/texel/cell; denoise/denoiser/atrous-dirty; stale/staleness; rays/samples; TILE/probe_tile; reuse/ReSTIR; ambient flat/constant; shadow hard/traced + cell/cached).
**Collisions: 7** (probe ×4; filter ×3; world cache vs WRC; page vs 12.5 cm shadow cell; fill ×3; ring (wake ring vs distance-band edge); hysteresis vs DDGI).

---
## In-flight conflicts and sequencing (as of 2026-10-14)
- **PR #40** `cursor/change-scaled-denoise-d009` (e9fe003): lighting.rs +28, lighting_settings test, lighting_gpu.rs, cache.glsl, compact.comp, new lighting_screen.comp. Conflicts with #1, #5, #8. Land it first, with its keys named `denoiser.screen_*`.
- **gibs-mover-attach-msme**: no remote ref yet. It will touch cache.glsl, compact, lighting.rs (object-local, MSME). Conflicts with #1, #2, #4. Tell the agent to use "cache entry" and `surface_cache.*` now, then land before renames.
- **PR #37** (668ff79): cache.glsl +334, compact, direct, lighting_gpu. Conflicts with #2, #6, #7 (comments/counters). Moderate rebase.
- **PR #31** (7729ef5) and **#39** (69ed030): based before PR #42 (their diff deletes fps_counter.rs). Heavy lighting.rs/lighting_gpu.rs rewrites; #39 removes lighting_lamp_pick.glsl. They conflict with nearly every key rename (#1, #3–#11) and need a rebase onto 2abeb0dd anyway.

Sequence: (1) glossary + AGENTS.md rule now (docs only, no conflicts). (2) Land PR #40 and the GIBS branch. (3) Public-key renames with aliases, one commit each (#1, #3, #4, #5, #8–#11, #14). (4) Internal/comment renames (#2, #6, #7, #12, #13) after #37 resumes. (5) Rebase #31/#39 onto the renamed tip when usage allows.
