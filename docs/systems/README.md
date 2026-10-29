# Systems

A system record is the living note for one engine system. An ADR records a decision and stays. A system record states the current goal, the intent, the code, the game use, and the limits. Edit the system record in the same change that changes those facts.

Read the record before you write a game feature that uses the system. Read it before you change or rewrite the system.

Every new system is designed with networking in mind. Its plan states the five lines in the [agent rubric](../agent-rubric.md): replicated state, authority, prediction or rollback, determinism, and headless server. The commit that first implements the system copies those lines into the system record. The session, the socket, and the snapshot codec belong to the [core-networking plan](core-networking/plan.md). When a name in that file differs from another plan, that file wins.

Engine-feature settings follow [the settings principles](../engine/settings-principles.md). Lighting is the reference example.

Each record uses these headings:

- **Goal.** The result the system exists to produce.
- **Intent.** The rules that stay true while the code changes.
- **Code.** The crate, the entry points, and the files that own the behavior.
- **Game use.** The calls a game makes. A game does not reach past these calls.
- **Limits.** The work that is not done yet.
- **Decisions.** The ADRs that bind the system.

## Records

- [Input](input.md) owns the keyboard, the mouse, the gamepad slots, and the action maps.
- [Window](window.md) owns the Wayland seat, focus, pointer lock, and fullscreen.
- [Math](math.md) owns positions, directions, matrices, and quaternions.
- [Physics](physics.md) owns the collision step, including mass, friction, springs, capsule, box, and mesh shapes, and the fracture hot-swap. The next pipeline is [physics/plan.md](physics/plan.md).
- [Fracture](fracture.md) owns the offline Voronoi cook and the `.gfract` asset. The step does not cook.
- [Scene](scene.md) owns the camera, the scripted layout, the coordinate frame, and the agent endpoint.
- [Renderer](renderer.md) owns the Vulkan draw, the world, and culling.
- [UI](ui.md) owns layout, pointer hits, and the lighting panel.
- [Lighting](lighting/README.md): how GI lights the picture (direct, shadows, GI, references, checks), each component against Lumen and the papers. Its debug tooling is the reference standard, inventoried in [the debug standard](lighting/debug-standard.md). The path-trace picture check is [the reference check](lighting/reference-check.md).
- [GI design](gi-design.md): meshes, two tracers, a surface cache and screen probes ([ADR 0012](../adr/0012-gi.md)).
- [Reflections design](reflections-design.md): specular reflections, planar mirrors, portals, and water ([ADR 0019](../adr/0019-reflections.md), accepted).
- [Bake](bake.md) owns the import-time GI data per mesh: sparse SDFs, surface cache cards and their cache.
- [Loading](loading.md) owns images, textures, texture maps, meshes, animations, PCM samples, and fonts.
- [Asset loading](asset-loading.md) is the design for import, offline cook, and async streaming of glTF models (ADR 0020).
- [Audio](audio.md) owns playback, Doppler, stereo image, and direct-path transmission.
- [Scripting](scripting.md) owns Rhai for games and mods.
- [Debugging](debugging.md) owns the shared debug scaffolding, the per-system binaries under `debug/<system>/`, the probes, the commands, the scripts, the captures, and the ground-truth reference.

## Plans

These are plans. They are not system records. A step edits the matching record when it changes a goal, an intent, a game use, or a limit.

- [Job system](job-system.md): a fiber-free work-stealing pool, serial when the server or a replay needs one order. Proposed.
- [Scene ECS](ecs-scene.md): archetype storage, a stable scene handle, a transform parent, and change records for lighting. Proposed.
- [Asset plan](assets/plan.md): import, the cook, the cache, streaming, packaging, pre-fractured chunks, shared asset ids, and the assets debug binary.
- [CCD and destruction](physics-ccd-destruction-design.md): speculative contacts, time-of-impact sub-steps, offline Voronoi chunks, and the bond graph ([ADR 0016](../adr/0016-physics-ccd-destruction.md)). Accepted. Speculative contacts, swept time of impact, and the hot-swap are in the physics step. `slice_chunk` is not.
- [PBR materials design](pbr-materials-design.md): glTF metallic-roughness, the shared shade, textures, and emissive lights ([ADR 0014](../adr/0014-pbr-materials.md)). Proposed. It does not copy the mesh-loading parsers.
- [Rendering extras plan](rendering/extras-plan.md): materials, shadows outside the lighting pass, particles, post, terrain, foliage, water, sky, decals, and culling. Client culling follows the asset plan's relevancy set.
- [HDR output design](hdr-output-design.md): scene-referred colour, exposure, AgX, bloom, HDR10 on Linux and scRGB on Windows ([ADR 0015](../adr/0015-hdr-output.md)). The metric gate is `crates/render/tests/hdr_output_gate.rs`.
- [Renderer performance review](rendering/perf-review.md): the 1920×1080 moving stress frame against a 5 ms budget, ranked by the lighting passes that close the gap.
- [Fog and volumetric lighting](fog-and-volumetric-lighting-design.md): a budgeted froxel volume, height fog, local volumes, shadowed shafts, and a world-cache read. [ADR 0018](../adr/0018-fog-and-volumetric-lighting.md) is accepted. The froxel holds height fog and local volumes. Sun and point-lamp shafts use traced shadows. The world-cache read waits for lighting phase 3. `fog-check` grades the closed forms and each live renderer row. `debug/lighting` exposes the Fog page and `get_fog_settings`, `set_fog_settings`, and `get_fog_stats`.
- [UI plan](ui/plan.md): retained elements, text, input, and binding an element to replicated state. Steps that read `DamageTracker` require PR #5.
- [Lamp layering](lamp-layering-design.md): optional baked GI layer for a static point lamp, and fast-blend settling for the shared live layer ([ADR 0017](../adr/0017-lamp-layering.md), proposed).
- [ReSTIR DI plan](lighting/restir-di-plan.md): CPU reference first, then small GPU stages. The 4070 check is one batched run at the end ([ADR 0022](../adr/0022-lighting-v3-compute-architecture.md) stage C).

The crate map and the frame loop are in [Architecture](../architecture.md). The words are in `CONTEXT.md`.
