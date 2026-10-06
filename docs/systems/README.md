# Systems

A system record is the living note for one engine system. An ADR records a decision and stays. A system record states the current goal, the intent, the code, the game use, and the limits. Edit the system record in the same change that changes those facts.

Read the record before you write a game feature that uses the system. Read it before you change or rewrite the system.

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
- [Physics](physics.md) owns the collision step, including mass, friction, springs, and capsule, box, and mesh shapes.
- [Scene](scene.md) owns the camera, the scripted layout, the coordinate frame, and the agent endpoint.
- [Renderer](renderer.md) owns the Vulkan draw, the world, and culling.
- [UI](ui.md) owns layout, pointer hits, and the lighting panel.
- [Lighting](lighting.md) owns radiance cascades and the light pass inside a draw.
- [Loading](loading.md) owns images, textures, texture maps, meshes, animations, PCM samples, and fonts.
- [Audio](audio.md) owns playback, Doppler, stereo image, and direct-path transmission.
- [Scripting](scripting.md) owns Rhai for games and mods.

The crate map and the frame loop are in [Architecture](../architecture.md). The words are in `CONTEXT.md`.
