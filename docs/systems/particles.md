# Particle effects (design)

Status: stage 1 implemented. Stages 2 and 3 stay design. Owner: Engine. Extends the CPU particles in `crates/render/src/particles.rs` and the [rendering extras plan](rendering/extras-plan.md) "Particles"; binds to [fog design](fog-and-volumetric-lighting-design.md) (#76), [HDR design](hdr-output-design.md) (#75), [GI design](gi-design.md).

## 1. Goal
Thousands to a million GPU-simulated particles per scene (sparks, smoke, dust, rain, magic), emitters defined as data, lit by the same lights/GI, fogged, HDR-correct, sorted or order-independent, inside a per-preset budget.

## 2. Where it sits today
| Piece | State | Code |
|---|---|---|
| Seeded CPU emitter, smoke from fire, card builder | done | `crates/render/src/particles.rs`, tests `particles.rs`, `particle_frame.rs`, `particle_resident.rs` |
| Particle atlas images | done | `pack.rs`, `ember.png`, `smoke.png` |
| Density card → fog card, fire → one light | done | `particles.rs`, `crates/load/src/emitters.rs` |
| GPU emit, simulate, compact, indirect draw | stage 1 | `shaders/particles.comp`, `src/gpu/particle_pass.rs`, `src/particle_sim.rs` |
| Sorting, froxel lighting, half-resolution, `.gfx` | later | stages 2 and 3 below |

## 3. Prior art
- Haar & Aaltonen, "GPU-Driven Rendering Pipelines", SIGGRAPH 2015 (persistent buffer, compute update) — extras plan already cites.
- Gareth Thomas, "Compute-based GPU Particle Systems", GDC 2014 (AMD): dead/alive lists, bitonic sort, tiled raster.
- Unreal Niagara (data-driven modules, GPU sim stages); Unity VFX Graph; Frostbite "Particle lighting" (Tatarchuk/Lagarde lit billboards, 2014–2018 talks).
- OIT: McGuire & Bavoil, "Weighted Blended Order-Independent Transparency", JCGT 2013; Wyman, "Stochastic Layered Alpha Blending", 2016. Soft particles: Lorach, NVIDIA 2007.
- Lighting particles cheaply: per-particle (vertex) lighting from the GI cache/froxels (Frostbite, Bungie Destiny "particle lighting" SIGGRAPH 2017).

## 4. Options
| Area | Option A | Option B | Pick |
|---|---|---|---|
| Sim | keep CPU | **compute: alive/dead lists, indirect args** | B |
| Render | expand cards on CPU | **vertex-pulled quads from GPU buffer, `vkCmdDrawIndirect`** | B |
| Transparency | per-emitter back-to-front + **GPU bitonic sort of alive list** | weighted blended OIT | sort for smoke/alpha emitters (exact, matches fog look); WBOIT as Low fallback; additive needs no sort |
| Lighting | unlit | **per-particle sample of froxel in-scatter + GI world cache at the particle centre** | B, half-res option |
| Emitters | code | **data file `.gfx` (spawn, forces, curves) + Rhai events** | data |

## 5. Recommendation
Stage 1: move simulation to compute (extras plan order: sim first, expander second), dead/alive ring, indirect draw, depth-faded soft particles, collision against the depth buffer. Stage 2: data `.gfx` emitters with curve modules (size/colour/velocity over life), sub-emitters, events from Rhai and physics impacts. Stage 3: lighting via the fog froxel grid (#76) and GI world cache, HDR emissive in scene-linear units feeding bloom/exposure (#75), bitonic sort per transparent emitter. Particles stay out of the G-buffer and the GI surface cache (extras plan rule) unless baked as a proxy; fire keeps its single promoted light.

**Deviation flags.** Lighting particles from the froxel grid instead of per-pixel follows Frostbite/Destiny practice, not Thomas 2014. Smoke with density keeps becoming fog cards (existing behaviour) rather than sorted sprites.

**Stage 1 as built.** One compute invocation walks the dead/alive ring, integrates, emits, and compacts into a GPU buffer. The keep pass draws that buffer with `vkCmdDrawIndirect` and a vertex-pulled quad (`particles.vert`, `particles.frag`). The CPU card builder in `particles.rs` stays for emitters that are not this path, including the camera flame. A disabled emitter, a zero count, or a zero budget does not dispatch. Particles are not written into the G-buffer. The fire stays one lamp.

The depth test reads an uploaded depth storage buffer (`Renderer::set_particle_depth`). It does not sample the scene depth attachment, because the compute step is recorded before the pass that writes this frame's depth. The lighting and stress frame loops copy the previous frame's stored depth and pass it to `set_particle_depth` when `depth_collision` is on and the emitter dispatches. Params, the depth upload, and the timestamp query pool are one per frame in flight. The sim buffers are one set: the other flight is waited before a host read or the next dispatch. `instance_count` is clamped to the compact capacity. The fade is one centre NDC sample stored in the compact colour, not a per-pixel Lorach 2007 sample. The walk is serial so the f64 mirror matches order. Haar and Aaltonen 2015 use parallel dead lists and atomics. That parallel walk is the departure.

`sort`, `lit`, and `half_res` are stored and written by the preset. Stage 1 does not dispatch a sort, a froxel light, or a half-resolution target. There is no `.gfx` parser. `--fx-spawn` and `fx.spawn` place this one emitter. `fx.sort_ms` and `fx.overdraw_px` stay 0. `fx.sim_ms` and `fx.draw_ms` are lavapipe timestamps when a frame has run. They are not a score against the GTX 1060 ceilings in section 7.

## 6. Data and settings
`settings.particles`: `max_particles` (Low 64K / Med 256K / High 1M), `sort` (Low off → WBOIT, Med/High on), `lit` (Low vertex-only, High per-particle froxel + GI), `half_res` (Low on), `depth_collision` (Med+). Emitters `.gfx`: rate, burst, lifetime, shape, forces, curves, atlas frame, blend mode, `seed`.

## 7. Per-frame stats and budget
`fx.sim_ms`, `fx.sort_ms`, `fx.draw_ms`, `fx.alive`, `fx.spawned`, `fx.overdraw_px`, `fx.vram_mb`. Low, GTX 1060 6GB, 1080p: 64K particles, sim ≤ 0.3 ms, draw ≤ 1.0 ms at half-res, ≤ 16 MB buffers. Vulkan 1.3 compute + indirect.

## 8. Verification harness and mutation tests
`crates/render/tests/particles_gpu.rs`, `scripts/fx-sparks.rhai`, existing particle tests kept.
- **CPU oracle**: same seed, CPU reference sim in f64 for N steps; GPU positions within 1e-3 m; alive counts equal.
- **Conservation**: spawned − died = alive each frame; no slot leak over 100 000 frames.
- **Sort oracle**: rendered stack vs CPU-sorted reference ≤ 2/255.
- **Fog/HDR**: particle inside dense fog is attenuated vs fog off (fog-check); emissive particle raises bloom energy proportionally.
- **Not in G-buffer**: existing test stays.
Mutations: skip dead-list push (leak), reverse sort, drop dt, ignore fog transmittance, unclamp alpha, unseeded RNG, no indirect count update (zero draws).

## 9. Agent control and verification
CLI `--fx-spawn <file.gfx> x y z`, `--fx-max`, `--fx-freeze`. MCP `fx.spawn`, `fx.set {emitter,param,value}`, `fx.stats`, `fx.list`, `view overdraw|particle-id`; emitter files hot-reload. Agent edits a `.gfx`, replays a seeded script, reads `fx.*` and grades screenshots.

## 10. Networking

- Replicated state: none. Particles are cosmetic and are never replicated.
- Authority: the client that sees the effect. The server does not own a particle buffer.
- Prediction / rollback: none. A client spawns from a replicated gameplay event (an impact, an ability) and does not rewind the buffer.
- Determinism: the spawn seed is derived from that event. The same seed and the same steps match the CPU mirror. The buffer itself is not a replicated snapshot.
- Headless server: the server runs no particles. A zero count does not dispatch.

## 11. Decisions for Anthony
1. Budget numbers per preset (64K/256K/1M) OK?
2. Exact sort (recommended) vs WBOIT everywhere for simplicity?
3. Emitter format `.gfx` data + Rhai events vs Rhai-only?
