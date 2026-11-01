# Lighting glossary (genos-engine)

Status: authoritative (see AGENTS.md, Lighting terminology). Audited 2026-10-14.
Purpose: high-level communication and quick comparison of external techniques. One term, one meaning.
Entry format: plain definition (one or two lines), **Role**, **Literature** (equivalents), **More detail**.
Links are relative to this file. GIBS.md is [research/GIBS.md](research/GIBS.md).
Names in *italics* are current names with a proposed rename in [RENAMES.md](RENAMES.md).

**Roles (fixed set):** STORE storage/representation · SCHED update/scheduling · SAMPLE sampling · TEMPORAL temporal blend · DENOISE spatial filter/denoise · VIS visibility/leak · LIGHTS light selection · OUT output/compose · TOOL settings/tooling.
Literature columns: GIBS (EA SEED 2021 surfels), DDGI (Majercik 2019 probes), Lumen (UE5 surface cache + screen probes + WRC), ReSTIR (Bitterli 2020), MSME (Barré-Brisebois, RTG ch. 25). "—" means there is no equivalent.

---
## Glossary

### Frame and output
**Direct light**: light that reaches a pixel straight from a lamp or the sun, with a shadow ray.
Role: OUT · Literature: GIBS direct + shadow rays; Lumen direct lighting; DDGI —
More detail: [`crates/render/shaders/lighting_direct.comp`](../../crates/render/shaders/lighting_direct.comp), `DirectSettings` in [`crates/scene/src/lighting.rs`](../../crates/scene/src/lighting.rs)

**Direct specular**: the glTF metallic-roughness lobe on direct light (GGX `D`, height-correlated Smith `V`, Schlick `F`, plus Lambert diffuse). `direct_specular=off` keeps the Lambert weight on irradiance. Indirect light stays Lambert.
Role: OUT · Literature: glTF 2.0 Appendix B; Filament `V_SmithGGXCorrelated` · More detail: [ADR 0025](../adr/0025-pbr-brdf-cpu-reference.md), [`pbr_brdf.glsl`](../../crates/render/shaders/pbr_brdf.glsl)

**GI (indirect light)**: light that arrives after one or more bounces. `gi.enabled=false` skips every GI pass.
Role: OUT · Literature: all of them · More detail: [../systems/gi-design.md](../systems/gi-design.md)

**Bounce count**: how many bounces GI carries: 0, 1, 2 or unlimited (the cache feeds itself).
Role: TOOL · Literature: GIBS "infinite bounce via cache"; DDGI recursive probes; Lumen multi-bounce via surface cache · More detail: `BounceSettings`; [GIBS.md §1.11](research/GIBS.md)

**Compose**: the pass that adds direct, indirect, sky and emissive into the final pixel. With *compose reuse* on, it skips pixels whose inputs did not change.
Role: OUT · Literature: GIBS "apply surfels to pixels"; Lumen composite; compose reuse —
More detail: [`lighting_compose.comp`](../../crates/render/shaders/lighting_compose.comp), [`lighting_sun_compose.comp`](../../crates/render/shaders/lighting_sun_compose.comp); [GIBS.md §1.9](research/GIBS.md)

**Sky light**: what a GI ray reads when it misses all geometry.
Role: SAMPLE · Literature: GIBS sky on miss (§1.11); DDGI/Lumen same · More detail: `--no-sky`

**Sun**: the one directional light. It is always traced, never lamp-picked.
Role: LIGHTS · Literature: GIBS treats it as a shadow-ray light · More detail: `--lighting-sun`

**Ambient fill**: a flat light used only when GI is off (constant/hemisphere/day/curve). It is not GI.
Role: OUT · Literature: — (classic ambient term) · More detail: `AmbientMode`

**Emissive**: a surface that gives off light. GI rays see it like any other lit surface.
Role: OUT · Literature: Lumen emissive via surface cache · More detail: [../systems/gi-design.md#emissive-surfaces](../systems/gi-design.md#emissive-surfaces); [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

### Display (accepted, ADR 0015)

**Scene-referred image**: Rec.709 linear radiance compose writes, before exposure, bloom, and the display transform. A value above 1 is in range.
Role: OUT · Literature: scene-referred colour (ACES, Frostbite) · More detail: [../systems/hdr-output-design.md](../systems/hdr-output-design.md)

**Exposure**: the scale from scene-referred radiance to the sensor, `1 / (1.2 × 2^EV100)`.
Role: OUT · Literature: Frostbite / Filament saturation exposure · More detail: [hdr-output design](../systems/hdr-output-design.md)

**EV100**: exposure value at ISO 100. A meter uses `log2(L × 100 / 12.5)`. A camera uses `log2(N² / t × 100 / S)`.
Role: TOOL · Literature: ISO 2720 reflected-light meter, K = 12.5 · More detail: [hdr-output design](../systems/hdr-output-design.md)

**Auto-exposure**: the EV100 taken from a log-luminance histogram after the tails are clipped.
Role: OUT · Literature: Reinhard log-average, with a histogram instead of one mean · More detail: [hdr-output design](../systems/hdr-output-design.md)

**Compensation**: stops added to exposure. Positive one stop multiplies exposure by two.
Role: TOOL · Literature: camera EV compensation · More detail: `HdrSettings.compensation`

**Display transform**: the map from exposed linear colour to the code the swapchain stores. AgX is the default.
Role: OUT · Literature: AgX; ACES output transform · More detail: [hdr-output design](../systems/hdr-output-design.md)

**AgX**: Sobotka's image formation, the Eary Chow generator. Inset, a log sigmoid from −10 to +6.5, a 40% hue mix, then outset. The default display transform.
Role: OUT · Literature: Sobotka; Blender AgX (Eary Chow) · More detail: [hdr-output design](../systems/hdr-output-design.md)

**ACES**: the Academy output transform, kept as an alternate display transform.
Role: OUT · Literature: Academy ACES 2.0 · More detail: [hdr-output design](../systems/hdr-output-design.md)

**Legacy shoulder**: the display curve in the tree today. At or below 0.64 it is the identity. Above that it is a shoulder. The `legacy` display mode.
Role: OUT · Literature: — · More detail: `tone_channel` in [`lighting_compose.comp`](../../crates/render/shaders/lighting_compose.comp); `linear_from_display` in [`trace.rs`](../../crates/render/src/trace.rs)

**Bloom**: scene-referred scatter added before exposure. The kernel is the dual-filter pyramid.
Role: OUT · Literature: Jimenez, SIGGRAPH 2014 · More detail: [hdr-output design](../systems/hdr-output-design.md)

**HDR10**: a 10-bit BT.2100 swapchain image encoded with the PQ curve. Graphics white is 203 nits. The sigmoid top maps to 1000 nits when the surface reports no peak. The Linux/Wayland preference.
Role: OUT · Literature: SMPTE ST 2084; `VK_COLOR_SPACE_HDR10_ST2084_EXT` · More detail: [hdr-output design](../systems/hdr-output-design.md)

**scRGB**: a linear extended-sRGB swapchain image. Code 1.0 is 80 nits. Graphics white is 203 nits, code 2.54. The Windows preference.
Role: OUT · Literature: IEC 61966-2-2; `VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT` · More detail: [hdr-output design](../systems/hdr-output-design.md)

**SDR output**: an 8-bit UNORM swapchain image in the sRGB nonlinear color space. Used when HDR is not selected.
Role: OUT · Literature: `VK_COLOR_SPACE_SRGB_NONLINEAR_KHR` · More detail: `create_targets` in [`gpu.rs`](../../crates/render/src/gpu.rs)

### Screen probes (final gather)
**Screen probe**: a point on the visible surface placed every *probe tile* pixels. It traces rays to gather indirect light for nearby pixels.
Role: SAMPLE · Literature: Lumen screen probe; DDGI probe is world-space (different); GIBS — (surfels play this role) · More detail: [`lighting_place.comp`](../../crates/render/shaders/lighting_place.comp), [`lighting_trace.comp`](../../crates/render/shaders/lighting_trace.comp); [../systems/gi-design.md#final-gather-screen-probes](../systems/gi-design.md#final-gather-screen-probes)

**Probe tile**: the spacing of screen probes in pixels (16 from 720p up).
Role: STORE · Literature: Lumen probe downsample factor; GIBS 16×16 spawn tile (§1.3)

**Gather**: turning a screen probe's ray results into its light estimate.
Role: OUT · Literature: Lumen final gather · More detail: [`lighting_gather.comp`](../../crates/render/shaders/lighting_gather.comp)

**Probe filter**: a spatial blur across neighbouring screen probes, then projection to SH. This is not the denoiser.
Role: DENOISE · Literature: Lumen screen-probe spatial filter; GIBS irradiance sharing (§1.14) · More detail: [`lighting_filter.comp`](../../crates/render/shaders/lighting_filter.comp)

**Kept hit**: the stored hit of a screen-probe ray, reused for up to N frames. Its light is re-read every frame, so we reuse visibility, not light.
Role: TEMPORAL · Literature: — in GIBS/DDGI; Lumen reuses radiance instead (we diverge on purpose) · More detail: [gi-design.md, final gather](../systems/gi-design.md#final-gather-screen-probes); experiments (see [../systems/lighting/experiments/](../systems/lighting/experiments/)) S78D, S78O, E05

**Stale frames**: once a kept hit or entry has gone this many frames without an update, it is treated as stale.
Role: SCHED · Literature: — · More detail: `probe_stale`, `world_stale`

### Surface light cache
**Surface light cache** (*world cache*, *SC*): a hash table of small world-space surface patches that store the light arriving there. Every bounce after the first reads it. It stands in for Lumen's card cache, which is not built yet.
Role: STORE · Literature: **GIBS surfel cache (closest)**; Lumen surface cache (cards, not built); DDGI probe volume (different representation) · More detail: header of [`crates/render/shaders/lighting_cache.glsl`](../../crates/render/shaders/lighting_cache.glsl); [GIBS.md §1.1–1.4](research/GIBS.md)

**Cache entry** (today also called patch, slot, texel or cell): one patch of the cache. It is a world-grid cell on one of six axes, and it holds a position, a normal, irradiance and a relight count.
Role: STORE · Literature: GIBS surfel; Lumen card texel; DDGI probe (loosely) · More detail: [`lighting_cache.glsl`](../../crates/render/shaders/lighting_cache.glsl) bindings 9–10

**Slot**: the storage index of an entry. Do not use it for the entry itself.
Role: STORE · Literature: GIBS free-stack index (§1.4)

**Cache cell size**: the world size of an entry: 25 cm near the eye, doubling with distance.
Role: STORE · Literature: GIBS screen-relative surfel size (§1.6); Lumen card resolution · More detail: `world_cell`

**Page**: an 8×8 block of entries scheduled and denoised together.
Role: STORE · Literature: Lumen atlas page · More detail: experiments S72J, S72K

**Young / mature entry**: a young entry has been relit fewer than `LIGHTING_CACHE_MATURE` times and goes first. Mature entries are relit in turn.
Role: SCHED · Literature: GIBS newly spawned vs converged surfels

**Dormant entry**: a converged entry whose new samples agree with its mean. It is relit only rarely.
Role: SCHED · Literature: GIBS dormant surfels (§1.12) · More detail: [`lighting_cache.glsl`](../../crates/render/shaders/lighting_cache.glsl) ~L933

**Cache round**: one scheduling step that relights a batch of entries.
Role: SCHED · Literature: GIBS per-frame ray budget (§1.12); Lumen card update budget

**Cache ray**: a ray traced from an entry to relight it. It is not a screen-probe ray.
Role: SAMPLE · Literature: GIBS surfel rays; DDGI probe rays

**Compact**: rebuilds the live-entry list each round and frees entries nobody has read.
Role: SCHED · Literature: GIBS recycling heuristic (§1.4) · More detail: [`lighting_compact.comp`](../../crates/render/shaders/lighting_compact.comp)

**Trickle**: the lowest relight tier. Settled entries that are still slightly inconsistent get an occasional relight.
Role: SCHED · Literature: GIBS variance-driven ray count (§1.12, partial) · More detail: S65A in [`lighting_compact.comp`](../../crates/render/shaders/lighting_compact.comp); S78H

**Pre-fill**: gives a newly seen entry a usable value on the frame it appears (coarse-to-fine), so the picture never shows missing light. Required by rule.
Role: SCHED · Literature: — (GIBS shows holes and fills them by spawning; Lumen falls back to WRC) · More detail: experiments S72D, S73A

**Unfilled**: a pixel or entry with no light value yet. These are logged to `./genos-debug/unfilled/`.
Role: TOOL · Literature: —

**Object-local entry**: an entry stored in a moving object's local space, so its light moves with the object.
Role: STORE · Literature: GIBS transform/bone-attached surfel (§1.5); Lumen moving cards · More detail: `GENOS_LIGHTING_SC_LOCAL`; branch gibs-mover-attach-msme

**World radiance cache (WRC)**: a coarse world-space cache of incoming light, used for far rays and as a fallback.
Role: STORE · Literature: Lumen WRC; DDGI volume; GIBS — (GIBS uses its grid for lookup only) · More detail: [../systems/gi-design.md#world-probes-radiance-cache](../systems/gi-design.md#world-probes-radiance-cache)

**Acceleration grid** (proposed, not built): a spatial lookup that finds entries near a point in a big world.
Role: STORE · Literature: GIBS trapezoid grid (§1.7) · More detail: [GIBS.md §1.7](research/GIBS.md)

### Estimators, change detection, denoise
**MSME**: the running mean/variance accumulator per entry. It speeds up its blend when the light changes.
Role: TEMPORAL · Literature: MSME (RTG ch. 25), used by GIBS (§1.8); DDGI hysteresis blend; Lumen temporal accumulation · More detail: experiment S72R; [`lighting_cache.comp`](../../crates/render/shaders/lighting_cache.comp) ~L277

**Inconsistency**: how far an entry's new samples disagree with its mean, in standard deviations. It drives trickle, dormancy and wakes.
Role: TEMPORAL · Literature: GIBS variance signal (§1.12)

**Wake**: forces a relight of entries near a change (a lamp or object moved). A **smart wake** is limited to the *wake ring*, the entries the change can actually affect, with duplicates removed and a per-frame cap.
Role: SCHED · Literature: — (GIBS relies on its variance signal; DDGI/Lumen on budgets) · More detail: `CHG_W_*` in [`lighting_cache.glsl`](../../crates/render/shaders/lighting_cache.glsl); S76B-D

**Verification**: a rotating set of kept hits re-traced each frame to catch changes the wakes missed. The verification budget is the number per frame.
Role: SCHED · Literature: — (closest: ReSTIR visibility reuse validation) · More detail: `CHG_V_*`; S78T2, S78L

**Hold** (*hysteresis*): keeps the stored value when the new one differs by less than its noise.
Role: TEMPORAL · Literature: different from DDGI "hysteresis", which means a blend weight (naming collision) · More detail: [`lighting_direct.comp`](../../crates/render/shaders/lighting_direct.comp) ~L30; S78T

**Denoiser**: the SVGF à-trous wavelet filter over the surface light cache (1, 3 or 5 iterations). Preset high launches it over the picture. The other presets launch it over hash slots.
Role: DENOISE · Literature: SVGF (Schied 2017); GIBS also denoises (§1.19); Lumen screen-space denoise · More detail: [`lighting_atrous.comp`](../../crates/render/shaders/lighting_atrous.comp), [`lighting_screen.comp`](../../crates/render/shaders/lighting_screen.comp); S77D, S78X, S80

**Dirty list**: the pages that changed this round. The denoiser runs only on them until they pass `dirty_fraction`.
Role: SCHED · Literature: —

**Adaptive denoise**: skips later à-trous iterations where variance is already low.
Role: DENOISE · Literature: variance-guided SVGF · More detail: `denoise_variance`; S79

**Screen denoise**: the picture-sized launch of the denoiser (`denoise_launch=screen`). Preset high uses it. The other presets keep `hash`. `--denoise-launch` and `GENOS_LIGHTING_DENOISE_LAUNCH` override the preset.
Role: DENOISE · Literature: Lumen/GIBS screen denoise; the kernel is still SVGF (Schied 2017) · More detail: [`lighting_screen.comp`](../../crates/render/shaders/lighting_screen.comp); S80

**Visibility / leak test**: stops light passing through thin walls. Today it is handled by the facing axis and the cell size. Depth moments are not used.
Role: VIS · Literature: GIBS depth moments + Chebyshev (§1.10); DDGI Chebyshev visibility · More detail: [GIBS.md §1.10](research/GIBS.md)

**Change record** (proposed, ADR 0013): one frame's description of what moved: a mover's old and new bounds, a lamp's old and new position and reach, or the sun. It raises the priority of what it can affect.
Role: SCHED · Literature: UE5 VSM caster-bounds invalidation · More detail: [localized-invalidation-design.md](../systems/lighting/localized-invalidation-design.md)

**Schedule mode** (proposed): `budget` (work caps + priority) or `global` (today's single stillness hash). `--lighting-schedule`.
Role: TOOL · More detail: [ADR 0013](../adr/0013-lighting-invalidation-is-local.md)

**Work cap** (proposed, ADR 0013): the static per-preset limit on one expensive pass per frame (rays, relights, spawns, coarsen ops, fresh shadow rays, denoise items). Work beyond it is deferred by priority.
Role: SCHED · Literature: GIBS global ray budget (§1.12); Lumen update budget · More detail: [design §3](../systems/lighting/localized-invalidation-design.md)

**Priority** (proposed): the per-item score that picks which entries, probes and pixel groups get fresh work within the caps. It replaces the global stillness hash.
Role: SCHED · Literature: Lumen surface-cache update priority · More detail: [design §5](../systems/lighting/localized-invalidation-design.md)

**Cache level / coarsening** (proposed): the cache key carries a level (L0 25 cm … L4 4 m). Coarsening merges a fine entry's MSME state into its parent before retiring it; the parent seeds fine entries on return.
Role: STORE · Literature: GIBS screen-relative size (§1.6, deviates: discrete levels) · More detail: [design §6](../systems/lighting/localized-invalidation-design.md)

### Lamps and shadows
**Lamp**: a point light. The sun is not a lamp.
Role: LIGHTS

**Lamp pick**: chooses a few lamps per shaded point, weighted by their expected contribution, instead of shading all of them. Modes: `off` (potato, day-cycle); `hits` (`gi-1bounce`, pixels stay on the full sum); `pixels`; `both`; `reuse` (low, medium, high: a reservoir per pixel). At 8 positive lamps or fewer, and on a quiet held view, `reuse` shades the sum. `reuse (experimental)` is a deprecated value and logs a warning.
Role: LIGHTS · Literature: RIS (Talbot 2005); `reuse` = ReSTIR DI (Bitterli et al. 2020) with pairwise MIS (Bitterli et al. 2022); GIBS weighs both (§1.16) · More detail: [`lighting_lamp_pick.glsl`](../../crates/render/shaders/lighting_lamp_pick.glsl)

**Lamp reuse**: keeps a pixel's chosen lamp in a reservoir and merges it again next frame (`lamp_pick_reuse`). Low is temporal only. Medium and High also merge one neighbour. History is the previous frame's pixel under the motion vector, rejected on depth and normal. A quiet held pixel stores the exact sum and then holds. Off fills a new reservoir each frame. The menu switch is this key.
Role: TEMPORAL · Literature: ReSTIR DI (Bitterli et al. 2020); motion vectors with depth/normal rejection · More detail: `lamp_pick_reuse`; [toggles.md](../systems/lighting/toggles.md)

**Lamp unbiased**: the reservoir merge (`lamp_pick_unbiased`). On is pairwise MIS and does not clamp M. The shade is `light * W`. Off divides by the sum of M, clamps M, and shades with `point_total / phat`. The menu switch is this key.
Role: LIGHTS · Literature: pairwise MIS (Bitterli et al. 2022; Wyman, SIGGRAPH 2023); the off path is Bitterli et al. 2020 section 3 and Wyman and Panteleev, Ray Tracing Gems II, 2021 · More detail: `lamp_pick_unbiased`

**Lamp layer** (proposed, ADR 0017): the finished indirect irradiance of one static point lamp, stored on the cache entries that lamp reaches, at unit white weight. A moving lamp does not get one. This is not the coarse layer, and it is not the traced field or the shown field.
Role: STORE · Literature: Enlighten per-light transport (Martin and Einarsson, SIGGRAPH 2010); Lumen has none · More detail: [lamp-layering design](../systems/lamp-layering-design.md)

**Layer weight** (proposed): the RGB intensity multiplied onto a lamp layer. Off, on, a dim, and a glow change the weight. They do not rebake the layer.
Role: LIGHTS · Literature: Enlighten runtime light scale · More detail: [lamp-layering design](../systems/lamp-layering-design.md)

**Live layer** (proposed): the one indirect cache written by every light that has no lamp layer. That is moving lamps, unflagged static lamps, the sun's indirect, and emissive.
Role: STORE · Literature: Lumen's single surface cache · More detail: [lamp-layering design](../systems/lamp-layering-design.md)

**Fast-blend** (proposed): the live layer's published irradiance closing on a confirmed new mean with a fixed time constant, after the multi-scale mean estimator says the short mean has moved. It is not a display fade.
Role: TEMPORAL · Literature: MSME catch-up (Barré-Brisebois, Ray Tracing Gems ch. 25); the time floor is not in that listing · More detail: [lamp-layering design](../systems/lamp-layering-design.md)

**Layer combine** (proposed): the pass that writes published irradiance as the live layer plus each published lamp layer times its layer weight. Readers see that sum only.
Role: OUT · Literature: — · More detail: [lamp-layering design](../systems/lamp-layering-design.md)

**Candidates / samples**: candidates are lamps considered without shadows. Samples are the shadow rays actually traced.
Role: SAMPLE · Literature: ReSTIR M candidates / visible samples

**Light tree** (in the pipeline): lamps grouped in a tree and walked randomly. Planned as `--lamp-pick tree`.
Role: LIGHTS · Literature: Stochastic Lightcuts (Yuksel 2019), GIBS [§1.16](research/GIBS.md) · More detail: [gi-design.md](../systems/gi-design.md#direct-lighting)

**Shadow cell cache** (*shadow filter = cell*): reuses a shadow result across a 12.5 cm world cell for N frames.
Role: VIS · Literature: — (similar to Lumen cached shadow masks)

**Ray guiding** (proposed, parked): steers rays toward where light comes from using a per-entry 6×6 map.
Role: SAMPLE · Literature: GIBS 6×6 guiding (§1.13); Müller path guiding · More detail: [GIBS.md §1.13](research/GIBS.md)

### Sun scheduling
**Sun band**: the slice of the cache relit for the sun each frame while the sun moves. **Catch-up**: extra relight rounds when lamps stop, so corners converge.
Role: SCHED · Literature: — · More detail: S63–S65, SUN-small-sun-step; [`lighting_gpu.rs`](../../crates/render/src/lighting_gpu.rs) ~L3025

### Participating media
**Participating medium**: a volume that absorbs and scatters light along a ray.
Role: OUT · Literature: Hillaire, Frostbite, SIGGRAPH 2015; Novák et al., SIGGRAPH 2018 course · More detail: [fog design](../systems/fog-and-volumetric-lighting-design.md)

**Extinction**: how much light a ray loses per metre (`σ_t`).
Role: STORE · Literature: Hillaire 2015 · More detail: [fog design](../systems/fog-and-volumetric-lighting-design.md)

**Optical depth**: extinction integrated along a path. Transmittance is `exp` of its negation.
Role: STORE · Literature: Beer-Lambert, as used by Hillaire 2015 · More detail: `fog_along` in [`particles.rs`](../../crates/render/src/particles.rs)

**Transmittance**: the fraction of light that survives a path.
Role: OUT · Literature: Beer-Lambert · More detail: `FogHit::transmittance`

**In-scattering**: light the medium adds to a ray.
Role: OUT · Literature: Hillaire 2015 · More detail: `FogHit::inscatter`; [fog design](../systems/fog-and-volumetric-lighting-design.md)

**Phase function**: the directional pattern of one scatter. Ours is Henyey-Greenstein.
Role: SAMPLE · Literature: Henyey and Greenstein 1941; Hillaire 2015 · More detail: [fog design](../systems/fog-and-volumetric-lighting-design.md)

**Froxel**: one cell of the frustum-aligned volume grid.
Role: STORE · Literature: Hillaire 2015 (frustum voxel); Wronski, SIGGRAPH 2014 · More detail: [ADR 0018](../adr/0018-fog-and-volumetric-lighting.md)

**Froxel grid**: the 3D grid of froxels. Zero width, height, or slices keeps that axis of the lighting preset (Low 64×36×24, Medium 96×54×32, High 160×90×48). A set axis replaces it. The size is not the picture size.
Role: STORE · Literature: Hillaire 2015; Wronski 2014 · More detail: [fog design §6](../systems/fog-and-volumetric-lighting-design.md)

**Froxel sample count**: the cell budget for that grid. Zero dispatches every cell. A positive count drops slices, then rows, then columns until the grid fits, so a later auto-tune can write it.
Role: TOOL · Literature: Hillaire 2015 (froxel resolution) · More detail: [fog design §13](../systems/fog-and-volumetric-lighting-design.md)

**Height fog**: extinction that depends only on world height.
Role: STORE · Literature: Hillaire 2015; Wronski 2014 · More detail: `density * exp(-(y - height) / thickness)`

**Local fog volume**: a box or a sphere of analytic density added into the froxel grid.
Role: STORE · Literature: Hillaire 2015 local media; Epic local fog volumes · More detail: [fog design §6](../systems/fog-and-volumetric-lighting-design.md)

**View-aligned volume sample**: a local-volume in-scatter tap moved onto the line parallel to the view through that volume's screen position. Extinction stays on the view ray.
Role: SAMPLE · Literature: departure from Hillaire 2015, who samples the view ray · More detail: `froxel_align` in [`froxel_common.glsl`](../../crates/render/shaders/froxel_common.glsl)

**Light shaft**: a beam made visible by shadowed direct light in the medium.
Role: LIGHTS · Literature: Hillaire 2015; Engelhardt and Dachsbacher, I3D 2010 (not the method we use) · More detail: [ADR 0018](../adr/0018-fog-and-volumetric-lighting.md)

**Fog history**: a reserved slot for the previous froxel's in-scattering. The setting is off on every preset. Whether to enable it is open.
Role: TEMPORAL · Literature: Hillaire 2015 temporal integration; [ADR 0012](../adr/0012-gi.md) stores no radiance history · More detail: [fog design §7](../systems/fog-and-volumetric-lighting-design.md)

### Quality and tooling
**Preset**: a named full set of settings: potato, day-cycle, gi-1bounce, low, medium, high. **Potato** is direct light plus flat ambient, with no GI. Role: TOOL
**Full effort**: one-shot action that pushes every effort knob to max for comparison. Role: TOOL
**Ray budget / budget shift**: the per-round ray budget divided by 2^shift. Role: SCHED · Literature: GIBS global ray budget (§1.12)
**Refresh frames**: "do this every Nth frame". Role: SCHED
**Reference tester**: [`crates/lighting-ref`](../../crates/lighting-ref): CPU references and picture verdicts. Role: TOOL
**Screen resolve**: gather, filter, and compose added together. The à-trous pass stays outside that sum. Role: TOOL
**GI pass time**: GPU milliseconds of one lighting pass (`direct`, `atrous`, `compose`, `trace`, `filter`, `gather`, `resolve`) or of the lighting total. `resolve` is the screen resolve and is not added again into the lighting total. `trace` is the screen-probe trace plus cache rays. `direct` is the direct dispatch plus hit light. Role: TOOL
**Work counter**: rays traced, cache entries refreshed, or lamps shaded (fresh shadow rays) in one recorded frame. Role: TOOL
**App lifecycle**: the window-crate owner of shutdown. The event loop stops producers, polls GPU fences, destroys the device, unmaps, then exits. A quit that blocks the seat misses `xdg_wm_base.ping`.
Role: TOOL · Literature: — (the seat ping is `xdg_wm_base.ping`) · More detail: [window.md](../systems/window.md)

### Specular, mirrors, portals, water
Accepted in [ADR 0019](../adr/0019-reflections.md) (Engine bot, 2026-10-26; Anthony Poschen delegated design approval). The CPU picture grades planar mirrors, portals, and flat water. Glossy screen marches, the world cache, ray queries, and the reflection denoiser are not built. One term, one meaning.

**Specular lobe**: the view-dependent glossy term, kept separate from diffuse GI.
Role: OUT · Literature: Cook and Torrance, TOG 1982; Lumen reflections · More detail: [../systems/reflections-design.md](../systems/reflections-design.md)

**Fresnel weight**: the fraction of light reflected at a boundary. Dielectrics use the exact equations. From under water the index ratio is inverted, and total internal reflection returns 1. Metals use Schlick with F0 equal to the base colour.
Role: SAMPLE · Literature: Fresnel equations; Schlick, CGF 1994 (metals) · More detail: [reflections design, optics](../systems/reflections-design.md#optics)

**Screen-space reflection**: a ray marched through the depth buffer along one specular direction.
Role: SAMPLE · Literature: McGuire and Mara, JCGT 2014; Stachowiak, SIGGRAPH 2015 · More detail: [reflections design, passes](../systems/reflections-design.md#passes)

**Reflection hit**: the first surface a specular ray accepts, stored as geometry, not as radiance.
Role: VIS · Literature: Lumen screen-trace hit; our kept hit · More detail: [reflections design, data](../systems/reflections-design.md#data)

**World-cache reflection**: specular colour read from a finished surface-cache entry when the screen trace misses.
Role: STORE · Literature: Lumen surface cache at a reflection hit · More detail: [reflections design](../systems/reflections-design.md#fit)

**Reflection ray**: an optional trace through the shared tracer, software SDF or `VK_KHR_ray_query`. The Low preset schedules none.
Role: SAMPLE · Literature: Lumen hardware or software reflection trace · More detail: [ADR 0019](../adr/0019-reflections.md)

**Glossy denoise**: one spatial filter on specular hits. It does not keep a radiance history.
Role: DENOISE · Literature: SVGF spatial kernel (Schied, HPG 2017). The temporal accumulation is not used · More detail: [reflections design, denoise](../systems/reflections-design.md#denoise)

**Planar mirror**: a flat reflector drawn by one extra camera reflected through its plane.
Role: OUT · Literature: Diefenbach, PhD 1996 · More detail: [reflections design, mirrors](../systems/reflections-design.md#mirrors)

**Mirror recursion**: extra mirror and portal views in one frame. A further view is drawn when the running Fresnel product stays at least 0.02 and the shared cap still has room. Portals share the cap.
Role: SCHED · Literature: shipped recursive mirrors and portals (a depth budget) · More detail: [reflections design, budgets](../systems/reflections-design.md#budgets)

**Oblique clip**: the near plane moved onto the mirror or portal so the draw rejects geometry behind that plane.
Role: VIS · Literature: Lengyel, Journal of Game Development 1(2), 2005 · More detail: [reflections design, mirrors](../systems/reflections-design.md#mirrors)

**Portal**: an opening that draws the linked view inside its frame.
Role: OUT · Literature: Luebke and Georges, I3D 1995 · More detail: [reflections design, mirrors](../systems/reflections-design.md#mirrors)

**Portal transform**: the rigid map from a point on the source side of a portal to the linked side.
Role: VIS · Literature: Luebke and Georges, I3D 1995 (the opening); the rigid map is ours · More detail: [reflections design, mirrors](../systems/reflections-design.md#mirrors)

**Water surface**: the mesh that shades reflection, refraction, and absorption from a Gerstner displacement.
Role: OUT · Literature: Finch, GPU Gems ch. 1, 2004 · More detail: [reflections design, water](../systems/reflections-design.md#water)

**Refraction**: the bent transmitted direction through a surface, from Snell's law.
Role: SAMPLE · Literature: Snell's law; Walter et al., EGSR 2007 (the BTDF we do not stochastic-sample) · More detail: [reflections design, optics](../systems/reflections-design.md#optics)

**Caustic**: light focused onto a receiver by refraction. The image-space map is written into the surface cache.
Role: LIGHTS · Literature: Shah, Konttinen, and Pattanaik, IEEE TVCG 13(2), 2007 · More detail: [reflections design, water](../systems/reflections-design.md#water)

**Shoreline**: the shallow band where wave height is clamped by depth and foam coverage comes from that clamp and the displacement Jacobian.
Role: OUT · Literature: McCowan, Philosophical Magazine 32, 1891 (H/d = 0.78; our cap is 0.75); Tessendorf, SIGGRAPH 2001 (Jacobian foam) · More detail: [reflections design, water](../systems/reflections-design.md#water)

**Reflection budget**: the per-preset ceiling on reflection milliseconds, VRAM, rays, and mirror views.
Role: SCHED · Literature: ADR 0013 work cap; Lumen reflection scalability · More detail: [reflections design, budgets](../systems/reflections-design.md#budgets)

### Materials
**Material record**: one glTF material's factors, texture slots, and alpha mode, before any texel is read. The code type is `PbrMaterial`.
Role: STORE · Literature: glTF 2.0 material; Filament `Material` parameters · More detail: [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

**Shade sample**: the linear values of one material record at one UV. Base colour, metallic, roughness, tangent-space normal, occlusion, emissive, and alpha.
Role: SAMPLE · Literature: glTF 2.0 Appendix B inputs; Filament `PixelParams` · More detail: [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

**Diffuse albedo**: base colour times (1 − metallic). It is the weight on irradiance. The base colour is a different value.
Role: OUT · Literature: glTF 2.0 Appendix B diffuse term; Filament diffuse albedo · More detail: `BakeMesh::surface`; [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

**Ambient occlusion**: the glTF occlusion texture. It scales indirect irradiance and does not scale direct light.
Role: VIS · Literature: glTF 2.0 occlusion texture · More detail: [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

**Promoted emitter**: an emissive region extracted at import and placed in the light tree.
Role: LIGHTS · Literature: Lumen emissive cards promoted to lights · More detail: [../systems/gi-design.md#emissive-surfaces](../systems/gi-design.md#emissive-surfaces); [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

**Texture page**: one fixed block of one mip of a texture. The word page alone remains the 8×8 surface-cache block.
Role: STORE · Literature: Mittring virtual-texture page (SIGGRAPH 2008) · More detail: [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md); [assets/plan.md](../systems/assets/plan.md)

**Material residency**: which texture pages are on the GPU.
Role: SCHED · Literature: Mittring physical page pool · More detail: [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

**Texture quality**: the sharpest mip the player allows. Ultra is mip 0. High, Medium, and Low are one, two, and three levels coarser. It does not follow free VRAM.
Role: SCHED · Literature: a sampler LOD clamp · More detail: [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

**Mip demand**: the finest mip a texture needs this frame. λ = log₂(ρ) from the screen-space UV derivatives of its draws, plus `mip_bias`, then clamped by texture quality. A texture that was not drawn keeps its last demand.
Role: SCHED · Literature: Vulkan image level of detail · More detail: [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

**VRAM budget**: `heapBudget` of the device-local heaps, minus the other engine allocations and the headroom. Texture streaming and later GPU users read one helper.
Role: SCHED · Literature: `VK_EXT_memory_budget` · More detail: [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

**Headroom**: bytes kept free under `heapBudget`. Default `max(5% of heapBudget, 256 MB)`.
Role: SCHED · Literature: `VK_EXT_memory_budget` heapBudget · More detail: [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

**Dwell**: frames a mip that just became resident cannot be evicted, unless usage is over `heapBudget`.
Role: SCHED · Literature: streaming hysteresis · More detail: [../systems/pbr-materials-design.md](../systems/pbr-materials-design.md)

### Historical (do not reuse)

**residency_mb**: a fixed High cap of 256 MB, dropped by file order. The runtime budget is the VRAM budget. The key remains a deprecated alias.

Radiance cascade, radiance field, v1 world probe, tier, notice band, prominence spacing: GI v1, removed on 2026-10-12. The radiance-cascades ADR (0008) was deleted; [ADR 0012](../adr/0012-gi.md) is the live decision.

### Retired terms

`scripts/check-terms.py` fails a diff that adds one of these outside the glossary, RENAMES.md, an alias shim (a line saying deprecated or alias), or a history note (retired, removed, gone). When a RENAMES.md entry lands, add its old name here.

- `residency_mb`
- `material-residency-mb`
- `GENOS_MATERIAL_RESIDENCY_MB`
- `radiance cascade`
- `radiance cascades`
- `radiance field`
- `v1 world probe`
- `notice band`
- `prominence spacing`

---
## How to compare a new technique
1. Split the technique into parts and give each part one Role.
2. For each part, find our glossary entry with that Role and the same job.
3. Mark it **replace** (does our job better), **extend** (adds to ours), **new** (we have that Role but not this job) or **no-equivalent** (we lack the Role here or don't want it).
4. Check rules: pre-fill, no whole-frame reuse, repeatable references, and the 200+ fps target.
5. Record the result in a research note with timestamps or paper sections.

**Worked example: GIBS**
| GIBS part | Role | Our term | Verdict |
|---|---|---|---|
| Surfel | STORE | cache entry | extend (same idea, hashed grid vs spawned discs) |
| Screen-relative size | STORE | cache cell size | equivalent (distance doubling) |
| Spawn from G-buffer, 16×16 gap fill | SCHED | pre-fill + young entries | extend: we fill on first sight (rule) |
| Recycling heuristic | SCHED | compact | equivalent |
| Transform/bone attach | STORE | object-local entry | new → in flight (gibs-mover-attach-msme) |
| Trapezoid grid | STORE | acceleration grid | new, later (big world) |
| MSME blend | TEMPORAL | MSME | equivalent (verbatim, S72R) |
| Variance-driven rays, dormant | SCHED | trickle, dormant, inconsistency | extend |
| 6×6 ray guiding | SAMPLE | cache ray (cosine) | new, parked for Anthony |
| Irradiance sharing | DENOISE | probe filter / denoiser | extend |
| Ray binning | SAMPLE | — | no-equivalent (perf only) |
| Depth moments | VIS | visibility / leak test | new (we have no moment test) |
| Lightcuts / ReSTIR | LIGHTS | lamp pick, light tree | extend (`hits` today, tree queued) |
| Transparent SH clipmap | STORE | — | no-equivalent |
| Kept hits | — | kept hit | ours only (GIBS has none) |
| Wakes, verification | — | smart wake, verification | ours only |

MSME reference code: [Ray Tracing Gems ch. 25](https://link.springer.com/chapter/10.1007/978-1-4842-4427-2_25). Papers: DDGI (Majercik et al. 2019, JCGT), ReSTIR (Bitterli et al. 2020), Stochastic Lightcuts (Yuksel 2019), SVGF (Schied et al. 2017). The full GIBS reference list is in [research/GIBS.md §6](research/GIBS.md).
