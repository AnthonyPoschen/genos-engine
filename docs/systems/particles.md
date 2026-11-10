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

**Stage 1 as built.** Compute dispatches one thread per slot (workgroup 64, `ceil(budget / 64)` groups). Integrate, emit, and the compact each run that wide. The block sums are scanned in workgroups of 64, and those totals are scanned the same way, so no thread walks the whole group list. The scan keeps dead-ring pushes and the compact in slot order, which is the order `particle_sim.rs` walks, so the f64 mirror still matches. Emit is its own compute pipeline. It jumps the xorshift by four steps per spawn so those writes are parallel and still consume the same stream, and that jump is not in the integrate shader. The keep pass draws the compact buffer with one `vkCmdDrawIndirect` and a vertex-pulled quad (`particles.vert`, `particles.frag`). When the keep pass loaded scene depth, that draw depth-tests and does not write depth. The CPU card builder in `particles.rs` stays for emitters that are not this path, including the camera flame. A disabled emitter, a zero count, or a zero budget does not dispatch. Particles are not written into the G-buffer. The fire stays one lamp. `fx.sim_ms` stops before the 4-byte sync witness. `fx.sim_init_ms`, `fx.sim_death_ms`, `fx.sim_emit_ms`, and `fx.sim_compact_ms` are the pieces inside that span. Teardown waits any live flight fence, then the device, before it frees lighting or anything else. The 48 ms drain can give up first; that wait still runs. The swapchain is destroyed after lighting.

Depth collision samples the previous frame's scene depth image on the GPU. The projection is that frame's view-projection, stored with the flight that wrote the image, so a camera move is tested against the view that produced the depth. That matrix already writes Vulkan depth in 0..1, which is the value stored in the attachment, so the particle's ndc z is compared directly. Moving geometry is one frame late: the image is the scene from the frame before. The compute step runs before this frame's scene pass, and it does not sample the attachment that pass is about to clear. `Renderer::set_particle_depth` remains the test override that fills a storage buffer. With no override, and once a frame has stored depth, the shader reads the image. The scene pass stores depth only when `depth_collision` is on. Other frames discard it. After the sample the image is returned to depth-attachment layout, which is the layout the reflection copy assumes.

Params and the timestamp query pool are one per frame in flight. Compact and the indirect args ping-pong per flight, so this frame's draw can overlap the next frame's compute. The persistent ring is one copy, in device memory, as are the slot buffer and the compact. A host-cached copy of those stores made the death walk cross the bus once per particle, and the cached compact made that span swing. The dispatch stays one thread per slot: a dead slot is still in the buffer and has to be visited. The CPU reads positions from a host mirror copied after the sim timestamps, and only when the frame copied the picture. The next compute starts with a pipeline barrier. Its source scope is the previous submission's compute writes. A 4-byte fill keeps its own transfer barrier, so it is not that dependency. This layer records a storage-buffer shader access as a read, and it treats a later queue submit as already ordered with the previous one, so a cross-submit race does not show up. The barrier's transfer source orders a one-word fill to a compute read in the same recording; that is the race the layer can see. The canary fill keeps its own transfer barrier. The compute source is still the ring. The ring shader's stores are not a write the layer can check. There is no CPU wait of the other flight. Alive and spawned are copied into the flight's own readback at the end of its compute; the host reads that copy only after the flight's fence. `instance_count` is clamped to the compact capacity. The fade is one centre sample stored in the compact colour, not a per-pixel Lorach 2007 sample. `fx.sim_ms` and `fx.draw_ms` are read from the flight whose fence has been waited. The draw timestamp brackets the indirect draw, inside the keep pass. `fx.vram_mb` counts the stage-1 buffers and, when depth is retained, both scene depth images.

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
