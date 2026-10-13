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
- [Physics](physics.md) owns the collision step, including mass, friction, springs, and capsule, box, and mesh shapes. The next pipeline is [physics/plan.md](physics/plan.md).
- [Scene](scene.md) owns the camera, the scripted layout, the coordinate frame, and the agent endpoint.
- [Renderer](renderer.md) owns the Vulkan draw, the world, and culling.
- [UI](ui.md) owns layout, pointer hits, and the lighting panel.
- [Lighting](lighting/README.md): how GI v2 lights the picture (direct, shadows, GI, references, checks), each component against Lumen and the papers. Its debug tooling is the reference standard, inventoried in [the debug standard](lighting/debug-standard.md). The path-trace picture check is [the reference check](lighting/reference-check.md).
- [Lighting v1](lighting-v1.md): historical. The radiance cascades path (GI v1), removed on 2026-10-12.
- [GI v2 design](gi-v2-design.md) is the proposed next lighting architecture: meshes, two tracers, a surface cache and screen probes.
- [Bake](bake.md) owns the import-time GI data per mesh: sparse SDFs, surface cache cards and their cache.
- [Loading](loading.md) owns images, textures, texture maps, meshes, animations, PCM samples, and fonts.
- [Audio](audio.md) owns playback, Doppler, stereo image, and direct-path transmission.
- [Scripting](scripting.md) owns Rhai for games and mods.
- [Debugging](debugging.md) owns the shared debug scaffolding, the per-system binaries under `debug/<system>/`, the probes, the commands, the scripts, the captures, and the ground-truth reference.

## Plans

These are plans. They are not system records. A step edits the matching record when it changes a goal, an intent, a game use, or a limit.

- [Asset plan](assets/plan.md): import, the cook, the cache, streaming, packaging, pre-fractured chunks, shared asset ids, and the assets debug binary.
- [Rendering extras plan](rendering/extras-plan.md): materials, shadows outside the lighting pass, particles, post, terrain, foliage, water, sky, decals, and culling. Client culling follows the asset plan's relevancy set.
- [UI plan](ui/plan.md): retained elements, text, input, and binding an element to replicated state. Steps that read `DamageTracker` require PR #5.

The crate map and the frame loop are in [Architecture](../architecture.md). The words are in `CONTEXT.md`.
