# Cone-traced cascades: implementation plan

Status: phases 0 and 1 passed; phase 2 accepted by Anthony at hall 5.3%, room 11.1%, Sponza atrium 12.9%, dark corner 15.9% (section 13); phase 2b passed with option B (section 14). Phase 3's gate is not met (sections 15 and 16). Anthony asked for no rejection and no revert, a cost cut to 10 to 12 ms or less with sparse voxels, and an experimental GPU path in the renderer so the editor shows it (section 17): phase 4 is in progress behind the setting `cone_cascades`, off by default: the field and the cascade's passes (stages 1 and 2 of section 17) are in the renderer, not yet graded. The evidence is a CPU prototype and a GPU timing of its trace step, in [`debug/single-frame-cascade/`](../../../debug/single-frame-cascade/README.md). A change to the product path needs an ADR first (phase 3).

## 1. What Anthony asked for

Recorded from the session of 23 and 24 Nov 2026.

- The lighting system handles any number of lamps, and the cost does not grow with them. A moving lamp costs the same as a still one.
- Lighting reacts in the frame. The churn in a room no lamp lights, and the slow follow after a lamp goes out, both come from the temporal mean that hides too few rays. The mean and the "a lamp moved, treat this cell as changed" rules should go.
- All lamps are part of the cascades, as Sannikov describes for Path of Exile: light sources are emitters, and everything is indirect light.
- No hardware ray tracing (24 Nov 2026). It is not a tracer in this plan.
- Try cone tracing through our voxels, so the trace cost has a cap.
- Every light is an emitter, the sun included, if the gates allow it (24 Nov 2026). The goal is to remove the direct-light calculations and their milliseconds. A light's shadow is then the occlusion the cone march collects on the way to it; there is no shadow ray or shadow map for it.
- The voxel field is on or off (24 Nov 2026). The old GI path is deleted, not kept beside it for a smaller preset.

Sources: Freeman, Sannikov and Pitt-Francis, arXiv 2607.20384 (the text is in `genos-debug/paper-2607.20384/` on the machine that ran this). Sannikov, "The Engine of Path of Exile", ExileCon 2026, from 27:30. In that talk the Path of Exile version is screen-space, 1 to 2 ms, with no temporal accumulation. The world-space version is about 5 ms and uses hardware ray tracing. Tracing the probes with cones is "a very new direction" with "some accuracy issues".

GI v1 stays removed. This plan does not bring back its probe volume. It changes how the sparse cascade ([sparse-radiance-cascades.md](sparse-radiance-cascades.md)) gets its radiance.

## 2. What is measured

All on the lighting-stress building, 1280x720, one ray or one cone a pixel, one frame, no temporal mean. The tables and the method are in the prototype's README.

| | Today's renderer | Prototype, rays | Prototype, cones |
|---|---|---|---|
| Error, lit hall | 5.4% (settled, full picture) | 6.4% (one bounce) | 11.6% (one bounce) |
| Error, room no lamp lights | 7.2% | 30% | 76% |
| Noise, lamps moving | 2.1 to 2.7% | 2.8 to 3.0% | 1.3% |
| Frames to follow a light change | 9 near a lamp, about 420 in the room no lamp lights | 1 | 1 |
| Lamp light with lamps as 5 cm emitters | not built | 54% error, 130% noise | 21% error, 9.7% noise |
| Trace step, 6 lamps (RTX 4070) | 11.4 ms, all of lighting | about 3.3 ms, hardware rays | 11.8 ms |
| Trace step, 49 lamps | 35.9 ms at 48, all of lighting | about 19.7 ms | 10.2 ms |

The three columns are not the same measure. The renderer's error is the settled full picture over 4x4 pixel blocks. The prototype's is one bounce in one frame for each compared pixel. The prototype's times are the trace step alone.

What the numbers support: a cone finds a small lamp where a ray does not, the cone march does not read the lamp count, and one frame is enough. What they do not support yet: accuracy. The room no lamp lights is the blocker.

## 3. The design this plan builds toward

One frame, in order. Each step names what exists and what is new.

1. **Voxel field** (new). A sparse hierarchy of world voxels, 6.25 cm to 8 m, kept as clip volumes around the eye. A voxel holds, for each of the six axis directions, the surface area that faces that way and the light that area sends out. A lamp is a sphere's disc area and its light. The field covers surfaces the picture does not show: the prototype's error in the hall doubled, 11.6% to 23%, when the voxels came from the picture alone.
   - Still geometry is voxelised once, at load or in the bake ([bake.md](../bake.md)).
   - A mover's voxels are re-written when it moves.
   - A lamp is written each frame from its position, colour and size. That is the only per-lamp work, and it does not depend on the picture size.
2. **Surface light** (new). The light a voxel sends out is its albedo times the irradiance the cascade gathered there last frame, plus its emission. So bounce light gains one bounce a frame, and the lamps' own light arrives in the first frame. No shadow ray to a lamp is cast for this.
3. **Cascade** (exists, changed; Anthony, 24 Nov 2026). No cone starts at a pixel. Cascade 0 is the voxel layer: a probe on each fine voxel of the visible surface, seated on the mean point of the pixels it stands for. The coarser cascades are world probes at twice the spacing, four times the directions and four times the reach each, as radiance cascades space them. Each probe of each cascade traces one cone for each of its directions, as wide as that cascade's direction spacing, so the cost is probes times directions. The cone of a coarser probe starts at the voxel layer (the cascade-0 probe under it nearest its seat) and runs through every interval; each interval is deposited into that voxel probe's probe of the same cascade (the paper's ray splitting, from the voxel layer). A coarse probe that traced from its own seat started its interval past any wall within the shorter intervals under it. Every cascade is recomputed every frame; nothing in it is random, so a still scene does not change. A candidate, if the shadow gates need it: the cascade-0 interval is marched in screen space against the depth buffer (step 6).
4. **Combine and shade** (exists). The cascades merge from the last one down (the paper's equation 7): a direction's cone is its own interval plus what passed it times the coarser cascade's cones, read from the coarser probes around it. That merge is where the voxel layer's short-range detail meets the world probes' long-range light. A pixel reads the voxel layer's probes around it, weighed by the light's mean direction in each direction (phase 0's direction moment). Lamp light and bounce light arrive the same way; there is no direct-light pass for point lamps.
5. **Sun** (exists as an exact ray, to become an emitter). The sun is tried as an emitter in phase 2b, in this order:
   - **A. The sun in the sky.** A cone that leaves the field with light still passing through reads the sky in its direction, and the sun is a small, very bright disc in that sky. No new pass. The sun is 0.5 degrees wide and a cascade-0 direction is one of 16, so the sun's energy is spread over the cone it falls in, to keep the total right. Risks: a coarse cosine, a blurred shadow from the coarse upper cascades, and any wall leak (E1) multiplied by the sun's brightness.
   - **B. Sun visibility in the voxels.** Each voxel stores whether it sees the sun, written by a march through the field toward the sun when the sun or a mover moves. A voxel's light adds albedo times sun times visibility, so sun bounce costs nothing more. A pixel reads that visibility with one texture read. Fog can read the same visibility in place of `trace::sees`. The shadow is as soft as a voxel, and the cost follows the voxel count and the movers, not the pixels.
   - **Fallback.** If A and B miss the phase 2b gate, the sun keeps its one exact ray a pixel (`lamp_pick.rs`, "the sun is one extra ray"). Its cost does not grow with the lamps. It is the only direct light left.
6. **Screen-space first interval** (new, a candidate). Contact shadows and thin casters are finer than a voxel (E6, E15). The cascade-0 interval, the short one next to the surface, can march the depth buffer, which has pixel detail. It runs along the cascade's own directions, not toward each light, so it adds no per-lamp cost. It covers only casters on the screen. Path of Exile's version of the cascade is screen-space.

The tracer stays behind one interface (`trace_ray` in `lighting_common.glsl`, `Surfaces` on the CPU). The cone march and the software tracer are the two tracers behind it. The prototype already runs rays and cones through the same deposit, merge and shade. The prototype's GPU ray timing through `genos-gpuref` stays a measuring tool only.

**Shadows.** With every light an emitter, a light's shadow is the share of the cone's light that a surface between the probe and the light blocks. No light has a shadow pass of its own. Where this is weak:

- A shadow is no sharper than the voxel the cone passes (6.25 cm at the finest level) or the cascade's direction spacing. Contact shadows blur.
- A caster thinner than a voxel adds a fraction of its area and casts a faint shadow or none: poles, railings, fingers.
- A shadow follows its caster only when the caster's voxels are written that frame. A mover and a skinned character are re-written every frame they move.
- A lamp in a fixture shares voxels with the shade or bracket around it. The march can block the lamp with its own fixture, or let its light out through the fixture.
- A penumbra that widens away from the caster is about right for a lamp with a size. A sharp sun shadow 10 m from its caster is the hard case: the march is in the coarse levels there.

## 4. Errors to iterate on

Each row is a fault the prototype shows or a risk it has not tested. "Probe" is how the prototype or the renderer reads it. The order is the order of work.

| # | Fault | Measured | Cause | Candidates | Probe |
|---|---|---|---|---|---|
| E1 | Light leaks through walls into a room no lamp lights | +65% bias, 76% error (rays: +11%, 30%) | A voxel wider than a wall holds the lit surfaces behind it. The trilinear read reaches them before the wall has covered the cone. Narrower cones did not help (+85% and +91% at half and quarter width, before E8 was fixed) | Read only voxels on the near side of a surface that faces the cone. Cap the voxel level near thin walls. Keep occlusion at a finer level than light. Store a wall's two sides in separate voxels (an inset by the normal at every level). **Fixed in phase 1: 7.45%, bias +0.4% (section 11)** | `--eye 4.75393,1.7,7.51477 --yaw 0.0466 --pitch -0.079`: bias against the reference; `--probe-pixel X,Y` for one pixel's ray and cone per direction |
| E2 | The cascade itself is 30% off in that room, with exact rays | 30% error, +11% bias; does not fall with more frames. **Fixed in phase 0: 5.3%** | The shade weighed each of the 32 cascade-0 directions by its centre's cosine; the room's light comes from a few bright patches, so the floor and ceiling took too much and the walls too little. Not the seam or the trilinear weights | The **direction moment** (glossary): shade with the cosine of the light's mean direction in each direction | Same pose, `--tracer ray --bin-cosine centre` against `mean` |
| E3 | Lamp light through cones is blurred and still noisy at the tail | 16 to 21% error, 7.6 to 9.7% median noise, 46% at the 90th percentile | A lamp's light lands in one direction of 16 at cascade 0, so the cosine and the shadow edge are as coarse as that direction. A cone whose axis misses the lamp's voxels reads less of it | More directions at cascade 0 for the lamp term. Aim the lamp's voxel footprint at the cone width. The screen-space first interval (step 6) for the shadow edge near the surface. Not exact shadow rays: every lamp is an emitter | `--mode emissive --lamp-radius 0.05`, hall |
| E4 | A slanted surface shows too much area | Not measured apart; up to 1.73 times for a diagonal | Area is kept by six axis directions, and the sum over them is the L1 norm of the direction | Keep a mean normal and one area, or renormalise by the L1 norm | A test wall turned 45 degrees |
| E5 | Nothing is known past the clip volumes | 126,174 of 1,867,174 voxels were inside them in the hall | A level covers 96 voxels a side. The fine levels end 3 m from the eye | Coarser levels cover the far field; check that a cone moves to them before it leaves a fine level. Size the clip volumes for each preset | Voxels inside the clip volumes, and the mean cover of a cone (0.942 in the hall) |
| E6 | Contact detail under one voxel is lost; a one-sided surface is clear from behind | Not measured | A cone starts 0.75 of the finest voxel off its surface, and only surfaces that face the cone cover it | The screen-space first interval (step 6), or a short exact ray for the first voxel. Two-sided cover for surfaces marked two-sided | Contact shadow checkpoint `contact`; a curtain |
| E7 | Two partial occluders in a row count as one surface | Not measured | Cover adds up along the cone and stops at 1. That is exact for one surface crossing several voxels and wrong for two separate ones | Multiply past the first full voxel of distance. Grade both against the reference | Two half walls, one behind the other |
| E8 | One light sample shared by a patch lights the dark side of a wall | +85% became +65% when the patch went from 25 cm to 6.25 cm | A patch of floor or roof that runs under a wall took one sample for both rooms | In the full design the light is the cascade's irradiance at probe spacing, so the same fault returns at the probe size. The read of last frame's irradiance into a voxel needs the wall-aware weights of E2 | The glowing line where a wall meets the floor and the roof |
| E9 | Bounce through feedback is not built | - | Step 2 of the design | Build it in the prototype before the renderer. Check the energy against the path-traced reference at 1, 2 and many bounces, and in the white room (albedo 1), where each bounce loses little | Mean irradiance by frame after a lamp goes on |
| E10 | The picture under a moving camera is not tested | - | Clip volumes snap to the grid, the level of detail of a probe changes with distance, and every frame takes new directions | Hold directions per probe. Blend the level of detail. Grade pan, strafe and walk as the Sponza gate does | The Sponza gate's pan checks, on the prototype's output |
| E11 | Movers and moving lamps are not tested, and the voxel upkeep is not timed | - | A mover re-writes its voxels. A lamp re-writes its own | Time both. A lamp is cheap. A mover is the open cost | Milliseconds to re-write one mover's voxels |
| E12 | Half-float voxels clip | Not seen yet | Light sums are radiance times area; a lamp's is its colour times 72 | Scale by level. Check the brightest lamp and the sun-lit roof | The largest value written to a voxel texture |
| E13 | Only the analytic building is tested | Ray tracer on Sponza since phase 0 (atrium 5.5%, dark corner 6.1%). The cone tracer does not voxelise glTF yet | The prototype loaded a `kind stress` scene only | Voxelise `GltfSurface` triangles for the cone tracer (phase 1). Sponza has curved surfaces, leaves and thin cloth, where E1, E4 and E6 bite | Sponza atrium, dark corner and curtains |
| E14 | Systems that read the lamps are not covered | - | Fog takes its lamp picks from ReSTIR (ADR 0026) and the sun from `trace::sees`. Reflections and specular read the direct pass | Each reads the voxel field: there is no direct pass left for them to read. Fog reads the sun visibility of step 5 B. Specular needs a sharp lamp highlight, which a cone does not give: find what it reads before phase 5 | - |
| E15 | Shadows are softer than the reference, and thin casters lose theirs | Not measured | A shadow is no sharper than the voxel the cone passes or the cascade's direction spacing. A caster thinner than a voxel covers part of it | The screen-space first interval (step 6). A finer level near the eye. Cover kept at a finer level than light (E1) | Penumbra width at contact and 1, 3 and 10 m from the caster; the thinnest caster that still blocks half the light |
| E16 | A lamp inside its fixture | Not tested; the prototype's lamps float free | A 5 cm lamp shares voxels with its shade or bracket | Write the lamp after the cover and do not let its own voxel cover it. Or set the lamp just outside the fixture's surface. Grade a lamp in a shade and a lamp on a bracket | The fixture scene: the room is lit and the shade's far side is not |
| E17 | Sun leaks through walls | Not tested; the sun is not in the prototype | E1, with a light orders of magnitude brighter than a lamp | E1 first. Phase 2b does not start until phase 1 passes | The room next to Sponza's courtyard, sun on, lamps off |
| E18 | A skinned character's shadow | Not tested | A skinned mesh changes shape every frame, so its voxels are re-written every frame | Voxelise its bones' capsules, not its triangles. Time it in phase 3 | Milliseconds to write one character; frames its shadow lags (must be 0) |

## 5. Cost to iterate on

- **The march.** 11.8 ms for 910,660 cones in the hall: 29.9 steps and 104.8 filtered reads a cone, from seven textures. Candidates: one cone for each cascade-0 probe direction in place of one a pixel (a probe is 8 pixels wide, so about a quarter of the cones), the lamp term folded into the direction textures (three reads a step in place of four), longer steps at the coarse levels, and a stop at full cover that the hall's open cones do not reach.
- **Memory.** Seven 96 x 96 x 768 RGBA16F volumes are 378 MiB. ADR 0012's lighting budget is about 200 MB, and ADR 0036 sizes the light cache for each preset. Candidates: RGB9E5 or R11G11B10 light with an 8-bit area, four directions' worth packed in two textures, fewer levels. A preset that cannot hold the field turns GI off. It does not fall back to the old path, which phase 6 deletes.
- **Voxel upkeep.** Not timed. Still geometry is free after load. Movers, skinned characters (E18), the surface-light write (step 2) and the sun visibility (step 5 B, when the sun or a mover moves) run each frame.
- **What goes.** Every point lamp's direct term and shadow ray, and the sun's if phase 2b passes. The sun ray's own time is not measured: time it in the debug binary before phase 2b, so the saving is known.
- **The shared steps.** Probe insert, deposit, merge and shade are not timed. The paper gives 7.3 ms on an RTX 3080 laptop. The renderer has a level-0 insert and a merge today; their times are in `lighting.gi_metrics` and the cascade timers.
- **The budget.** ADR 0013 caps lighting at 8 ms at 1080p. Today's renderer is 11.4 ms at 720p with 6 lamps.

## 6. Phases

Each phase is its own change, with the gate it names. A phase that misses its gate stops the plan there: a note in this file, and the renderer is left as it is. Phases 0 to 2 are the CPU prototype only.

| Phase | Work | Gate |
|---|---|---|
| 0. A baseline to trust | E2 in the prototype. glTF scenes (E13). The same error measure as `capture.compare` (block means), so the prototype and the renderer are graded alike | Ray tracer, one frame: the room no lamp lights within 10% of the reference, the hall within 7%, Sponza atrium and dark corner within 10%. **Passed 24 Nov 2026:** 5.3%, 5.5%, 5.5%, 6.1% (prototype README, Phase 0) |
| 1. The leak | E1, E4, E7, E8 with cones | Cone tracer: the room no lamp lights within 1.5 times the ray tracer's error from phase 0, and bias under 10%. The hall and Sponza no worse than 1.5 times. **Passed 24 Nov 2026:** room 7.45% (limit 7.95%), bias +0.4%; hall 5.64% (limit 8.3%); Sponza atrium 6.74% (8.3%), dark corner 6.47% (9.2%) |
| 2. Lamps and the full loop | E3, E9, E15, E16: every point lamp an emitter, and bounce through feedback, together. No exact lamps. If the contact or thin-caster gate misses, try the screen-space first interval (step 6) before anything else | The full picture (lamp light and all bounces) within 10% of the path-traced reference in the hall, the room and Sponza after 8 frames, and within 15% in the frame after a lamp goes on. Noise under 3% median. Shadows against the reference: penumbra width at contact and at 1, 3 and 10 m from the caster; the thinnest caster that blocks half the light; a mover's shadow lags 0 frames; a lamp in a shade lights the room and not the shade's far side |
| 2b. The sun as an emitter | E17. The sun's exact ray timed first. Then step 5 A, and B if A misses | Sponza courtyard, sun only: within 10% of the reference, and the same penumbra checks as phase 2. The room next to the courtyard, sun on: bias under phase 1's limit. If A and B both miss, the sun keeps its exact ray and the plan goes on |
| 3. Cost, and the decision | The GPU march tuned (section 5). The voxel write for a mover, a skinned character and the surface light, timed. Camera motion and moving lamps on the GPU prototype (E10, E11, E18). Then the ADR | Trace step, voxel upkeep and the shared steps together under today's `lighting_ms` at 6 lamps, and flat to 48 lamps, on the RTX 4070 at 720p. Memory inside the preset's budget, or the preset has GI off. Anthony accepts the ADR, or the plan stops |
| 4. The voxel field in the renderer | The field and its upkeep in `crates/render`, behind a setting, with the software tracer still the default. Probes and the debug binary (section 7) | The field's cover of a known wall matches the CPU prototype's. `make sponza-gate` and `make moving-lamp-gate` unchanged with the setting off |
| 5. Cones in the cascade | The cascade takes its radiance from the cone march, every cascade every frame. Lamps are written into the field. The per-lamp loops go: the pixel's cascade visibility sample for each lamp, the pixel exact shadows (`RESTIR_EXACT_NEAR`), and `lighting_cascade_direct_at` at every ray hit. The sun's exact ray goes too if phase 2b passed. Fog, reflections and specular read the field (E14) | The phase 2 numbers, read from the presented frame. `lighting_ms` flat from 6 to 48 lamps. A new gate in `scripts/ci-gate-knockout.sh` for the leak (the room no lamp lights) |
| 6. Remove the temporal rules and the old path | Delete the lamp share, the drift rate, the change signals, the refresh period and the long mean in `lighting_cascade_upper_shade.comp`. Delete the software-traced GI path: there is no preset that keeps it | The moving-lamp gate's noise limits hold with no temporal mean, and its follow check passes in 1 frame |

## 7. Probes, tunables and the debug binary

The prototype is `debug/single-frame-cascade/`. From phase 4 the coverage moves into `debug/lighting`, as the rubric requires.

Tunables, each a setting under [the settings principles](../../engine/settings-principles.md) from phase 4: GI on or off, the tracer (cone, or software until phase 6), the cone width, the cascade-0 probe size in pixels, the direction count at cascade 0, the cascade count, the clip volume side for each level, the finest and coarsest voxel level, the screen-space first interval on or off, and the sun's path (sky, voxel visibility, or exact ray).

Probes, each on the command catalogue from phase 4:

| What can fail | Probe |
|---|---|
| A wall leaks (E1) | Mean irradiance of a room with every lamp outside it, over the reference |
| A cone leaves the field (E5) | Mean cover of a cone, and the share of cones that end uncovered |
| A voxel is wrong | A dump of one voxel: area and light for each direction, the lamp term, its level |
| The field is stale after a move (E11) | Frames since a mover's voxels were written |
| The march costs too much | Steps and texture reads a cone, and the march's GPU time |
| A voxel texture clips (E12) | The largest value written this frame |
| Bounce energy drifts (E9) | Mean irradiance by frame after a lamp change |
| A shadow is too soft or missing (E15) | Penumbra width over the reference's, by distance from the caster; the thinnest caster that blocks half the light |
| A shadow lags its caster (E11, E18) | Frames between a caster's move and its shadow's |
| A fixture blocks its own lamp (E16) | Light leaving a lamp's voxel over the lamp's emission |
| The sun leaks indoors (E17) | Mean irradiance of a room with the sun outside it, over the reference |

A view of the voxel field (cover and light, by level) goes in the debug menu with the other lighting views.

## 8. Networking

- Replicated state: none. The voxel field and the cascades stay on the machine that shades.
- Authority: the machine that shades. A lamp's position, colour and size are scene state, with the scene's authority.
- Prediction / rollback: none. A rollback of the scene re-writes the movers' and lamps' voxels the next frame.
- Determinism: the field is a function of the scene and the eye's clip volumes. The cone directions are a function of the frame number. Not bit-exact across GPUs (texture filtering).
- Headless server: does not build the field or the cascades.

## 9. Decisions

Made by Anthony on 24 Nov 2026:

1. **Exact shadow rays.** None for point lamps: every lamp is an emitter, and its shadow comes from the cone march. The sun is tried as an emitter in phase 2b. It keeps its one exact ray only if that phase misses.
2. **Hardware ray tracing.** Not part of this plan, as a GI tracer or as a shadow tracer.
3. **A preset without the memory for the field.** GI is off. The old software path is not kept alive for it, and phase 6 deletes it.
4. **The level-0 long mean of PR #322.** Not part of this plan. That work has stopped.

Still open: what specular and reflections read for a sharp lamp highlight once the direct pass is gone (E14).

## 10. What stays from the session of 23 Nov

Committed on `lighting/stopgap-and-cone-prototype`: the frame-time step for auto exposure, the lamp share and drift rate in the upper cells, the two corner-leak fixes (`CASCADE_RECORD_PLANE`, `CASCADE_SEAT_INSET`), and `make moving-lamp-gate`. They are the stopgap until phase 6. The gate stays: phase 5 and phase 6 are graded by it.

## 11. Phase 1 result, 24 Nov 2026

Passed. Cone tracer, one frame, 8 px probe, 4x4 blocks, 16,384 reference rays a block, the phase 0 baseline (mean cosine) underneath:

| View | Limit | Error | Bias | Noise, median | Voxel reads a cone |
|---|---|---|---|---|---|
| Room no lamp lights | 7.95%, bias 10% | 7.45% | +0.4% | 3.0% | 257 |
| Hall | 8.3% | 5.64% | −2.6% | 2.3% | 369 |
| Sponza atrium | 8.3% | 6.74% | −2.2% | 1.8% | 354 |
| Sponza dark corner | 9.2% | 6.47% | +1.2% | 2.6% | 347 |

The march that passes, in the order the fixes were found (each measured on the room; the room went 63.9% → 7.45%):

1. **Front side** (`--voxel-order front`). A voxel keeps, for each of its six directions, the centroid of the area facing that way. That area counts only when the cone starts on the side it faces. The sunlit top of the roof no longer lights the ceiling under it. 63.9% → 58.3%.
2. **Refine on contact** (`--refine-level 3`). A step at a coarser level that meets cover is walked again at 12.5 cm. 58.3% → 46.1%.
3. **No light under walls.** A floor or ceiling patch whose lifted point is inside a solid (the floor under a wall) keeps its cover and gets no direct light: its shadow rays started inside the wall and escaped. 46.1% → 31.5%.
4. **Plane crossing at fine levels** (`--fine-walk cross`). At 12.5 cm and finer a voxel holds about one plane for each way it faces, so its centroid places that plane. Each plane counts once, where the cone's axis crosses it, in the voxel that holds the crossing point; a voxel at least half full blocks the whole cone (`--cross-full 0.5`; the points' area in a full voxel comes to 0.8 to 1.2 of it). Every step at or under the refine level uses it, so the first short intervals at a corner do too. 31.5% → 7.57%.
5. **The points' box** (`--box-margin`, 3.1 cm). A voxel also keeps the box of the points that face each way; a crossing counts only inside it. A curved or narrow surface (a column, an arch edge) no longer fills its voxel's plane. Sponza atrium 11.1% → 8.9%.
6. **Only the faces rays can reach.** A glTF triangle is voxelised on its front only, unless its material is `doubleSided`. A one-sided solid's back faces show only from inside it; a cone that slipped past a column's front met the inside of its far wall as a dark plane. Sponza atrium 8.9% → 6.74%, dark corner 12.3% → 6.47%.

Tried and taken out: count by centroid ahead of the start (62.0%); count in the step whose slab holds the centroid (327%); count once by plane depth at every level (186% to 590%: a coarse voxel's centroid averages every surface facing one way in it); cap all levels at 12.5 cm (51.3%); leave the floors under walls out instead of dark (50.1%: they also block); multiply transmittance across crossed planes (no change); refine at 25 cm (worse); box entry in place of plane crossing (worse in the room).

Costs for phase 3: the march reads 257 to 369 voxels a cone against 215 before, and the voxel needs the six centroids and six boxes (15 more numbers a direction set). The glTF voxels are 51 million surface points in about 6 million voxels for Sponza. Neither is on the GPU yet.

## 12. Phase 2 progress, 24 Nov 2026

Not passed yet. `--mode full`: point lamps are 10 cm glowing spheres in the voxels, surfaces keep their albedo (the engine's 0.8 paint on the analytic building), the sun is left out (phase 2b), and the reference is a path trace to 64 bounces with the lamps sampled at every hit.

How the light is built, a departure from section 3 step 2:

- **Gather patches.** The surface points are grouped into 25 cm patches that face one way. A floor or ceiling point inside a solid is buried: it is relit, but does not place its patch's gather point.
- **Light transfer, baked.** Every patch casts 128 cones once while nothing moves. Each cone records, for every surface patch it met, the share it filled times that patch's albedo over pi: a row of a sparse matrix T (94,000 patches and 3.8 million entries in the stress building; 257,000 and 23 million in Sponza). T is geometry; a mover would re-bake the rows it reaches.
- **Each frame:** every patch casts 16 cones (`--gather-cones`) through the voxels as they are for the lamps' and sky's light D; `E = D + T E` is iterated 256 times; the surfaces send out albedo times E over pi. So every bounce is in the frame after a lamp changes, with no temporal mean. Section 3's "the cascade's irradiance from last frame" gave one bounce a frame and took 13 frames to settle in the hall.

What it took, in order: the engine's paint reflectance (the analytic mesh's albedo is the colour; at 1.0 the white rooms never lost light); leaving out the ground's underside; steps that tile the axis (a cascade's first step at a smaller level left a stretch unread, and the room no lamp lights read the next room's lamp-lit ceiling through the corridor wall: +105% → +13%); each patch its own cone rotation (the frames swung ±5% together); lamps read once per voxel, at their centroid's depth, with a tent across the axis only (the lamps' light at the patches went from 46% to 104% of exact); a buried test per point.

Gate numbers, 8 frames, 16,384 reference rays a block, `--gather-cones 32` (16 in brackets):

| View | Limit | Error, frames 1 to 7 | Bias | Noise, median (limit 3%) |
|---|---|---|---|---|
| Hall | 10% after 8; 15% at frame 1 | 9.5 to 10.5% | −7 to −8% | 3.6% (4.3%) |
| Room no lamp lights | same | 9.8 to 13.7% | +6 to +12% | 2.4% (4.1%) |
| Sponza atrium | same | 18.2 to 21.0% | −6 to −10% | 10.7% (12.8%) |
| Sponza dark corner | same | 26.9 to 28.2% | −12 to −15% | 6.1% (6.5%) |

The weak part is the lamps' own light at the pixels (E3). Lamp light alone (`--mode emissive`, 10 cm lamps): Sponza atrium 21.7% error and 20% noise, dark corner 36% and −18% bias; the hall 10.3% and −3%. Rays are 78 to 83% with over 100% noise, so the cones are the better tracer, but three small lamps in a large space are still found by too few cones a frame. Next: the lamp term (E3's candidates: more cascade-0 directions for lamps, the cone footprint aimed at the lamp's voxels), then the shadow checks and the fixture lamp.

Cost, CPU: the bake 11 to 30 s for the stress building and 82 to 88 s for Sponza; each frame's gather and solve 4.6 s (stress) and 21 to 37 s (Sponza) at 16 to 32 cones on 16 threads. Phase 3 has to time the GPU version.

## 13. Cones from the voxel layer, 24 Nov 2026

Anthony's direction: no cone from a pixel; cascade 0 is the voxel layer that stands for the pixels over it, the coarser cascades are world probes, and the pixel's light comes from combining the layers as radiance cascades do. Section 3 steps 3 and 4 now say so. Measured with `--theta 8 --probe-px 12` (128 cascade-0 directions, a voxel-layer probe every 12 pixels), the default the gates use:

| Step | What changed | Room no lamp lights | Hall | Noise |
|---|---|---|---|---|
| Each probe traces its own interval from its seat (`--cone-start probe`) | 32 directions, 8 px | 36.8%, +33% | 4.7% | 0 |
| Coarse probes' cones start at a voxel probe under them, split into its cascades (`--cone-start voxel --cone-voxel random`) | 32 directions, 8 px | 11.4%, −7.9% | 4.4% | 1.9% |
| The same, 128 directions, 12 px | | 6.4% | 3.0% | 2.0% |
| The voxel probe nearest the coarse probe's seat, every direction and frame (`--cone-voxel nearest`, default) | | 6.4% | 2.65% | 0 |

The visibility test on the combine (`--combine-visibility`) changed nothing measurable once the cones start at the voxel layer.

Phase 1 with this tracer (one bounce, limits in section 6): room 6.4%, hall 2.65%, Sponza atrium 7.6%, dark corner 5.9%. Cones: 10 to 17 million a frame at 1280x720 (8 to 19 for each shown pixel), against 0.9 million when every pixel marched one cone; phase 3 has to time it.

Phase 2 with this tracer (`--mode full`, 32 gather cones a patch, fixed directions), after the first frame:

| View | Error | Bias | Noise |
|---|---|---|---|
| Hall | 4.8% | −1.6% | 0 |
| Room no lamp lights | 12.4% | +11% | 0 |
| Sponza atrium | 13.4% | −4.2% | 0 |
| Sponza dark corner | 17.8% | −6.0% | 0 |

Left for phase 2: the room's bounce (+11%), and in Sponza the lamps' own light (16.8% alone in the atrium): a lamp's shadow edge is as soft as the cone that finds it.

### Converged references and the cascade knobs, 24 Nov 2026

The phase 2 numbers above were measured against 16,384 path-traced samples a block; in the dim room that reference was itself noisy (23.5% error at 4,096 samples, 11.1% at 32,768). Against 131,072 samples a block (`--stride 64`), phase 2 with 4 cascades, t0 1.6: hall 4.0%, room 11.6% (+11.5% bias; +6.9% with the sky off), Sponza atrium 13.8% (−2.9%), dark corner 17.8% (−6.4%).

The lamps' own light (`--mode emissive`) against cascade count and t0, Sponza: 5 cascades take the dark corner from 22.9% to 16.8% (bias −10% to −2%); t0 2.5 takes the atrium from 16.8% to 15.1%. Phase 2 with `--cascades 5 --t0 2.5`, converged references, after the first frame:

| View | Error | Bias | Cones a frame |
|---|---|---|---|
| Hall | 5.3% | −0.5% | 24.2 million |
| Room no lamp lights | 11.1% | +11.1% | 14.6 million |
| Sponza atrium | 12.9% | −2.3% | 16.7 million |
| Sponza dark corner | 15.9% | +0.8% | 9.6 million |

No noise: nothing in the frame is random. Closing a cone that ends at least 90% covered (so its shortfall takes no sky) changed nothing measurable and was taken out. The gate (10%) is not met in three of the four views: in Sponza by the edges of the small lamps' shadows, in the room by bounce (+7%) and sky (+5%) on a very dim picture.

## 14. Phase 2b: the sun as an emitter, 24 Nov 2026

Anthony accepted phase 2 at the numbers in section 13 and asked to go on. `--sun on` puts the scene's sun in, with `--sun-path`:

- **A, the sun in the sky** (`--sun-path sky`): a cone that leaves the field in the last cascade's direction holding the sun takes the sun's irradiance over that direction's solid angle. The energy is right; the shadow is not: it comes through the coarsest cascade, whose probes are metres apart.
- **B, the sun in the voxel layer** (`--sun-path voxel`, the default): each cascade-0 probe casts one narrow cone toward the sun from its seat; a pixel takes the sun times its cosine times the share of the probes around it that see it, with the same weights it reads the cascade with. The cascades carry no sun. For bounce, each gather patch casts one narrow cone toward the sun.

`--cascades 5 --t0 2.5`, 32 gather cones, 65,536 reference samples a block, after the first frame:

| View | A | B | Gate |
|---|---|---|---|
| Sponza atrium, sun only (`--no-lamps`) | 52.1% | 10.7%, bias −4.3% | within 10% |
| Sponza colonnade, sun only | 19.4% | 23.7%, +13% (mean irradiance 0.005: the reference is weak there) | |
| Dark corner, sun and lamps | 15.7%, +0.9% | 15.7%, +0.9% | bias under 10% |

B passes the leak check and sits at the edge of the sunlit limit; it is the sun's path. The sun's cost is one narrow cone for each voxel-layer probe and each gather patch, whatever the lamp count. No sun ray starts at a pixel.

## 15. Phase 3: the voxel-layer cascade on the GPU, 24 Nov 2026

`--gpu-bench` now times the voxel-layer cascade, not one cone a pixel. One CPU frame lists its cones (start on the voxel layer, first reach, direction, cascade). `shaders/cascade.comp` marches each one on the GPU through the clip volumes: seven RGBA16F face volumes over all eight levels, and twelve RGBA8 plane volumes (centroid and point box) over the levels a crossing reads. The jobs and the stats are in device memory. Per-interval results go to a ring of a million cones, so the writes are paid. Departures from the CPU march: no front rule on the coarse read, and lamps are read trilinear at each step's centre. Hall, 1280x720, `--theta 8 --probe-px 12 --cascades 5 --t0 2.5`, RTX 4070:

| March | ms a frame | Cones | Steps a cone | Texture reads a cone |
|---|---|---|---|---|
| Every interval from the voxel layer (as the CPU prototype does) | 7,362 | 24.2 million | 119 | 657 |
| Each cone its own cascade's interval (`--gpu-own-interval`) | 1,486 | 24.2 million | 40 | 213 |
| Budget (ADR 0013: all lighting at 1080p) | 8 | | | |
| For scale: one hardware path a pixel with 6 lamps' shadow rays | 6.1 | | | |

Textures 459 MiB, cone list 739 MiB. Only 93,401 of the hall's 1.3 million voxels are inside the 96-texel clip volumes, so a larger clip would add cost.

Where the time goes: 24 million cones a frame, about 26 a shown pixel (the old one-cone-a-pixel bench marched 0.9 million), and 1.5 ns a step against 0.43 ns for the old shader, because the plane-crossing test reads up to 16 cells with 6 ways each and keeps its hits in local arrays. To reach 8 ms the cones must fall about tenfold (fewer cascade-0 directions, fewer coarse probes, or cascade directions shared between probes as the paper's sparse probes are) and the step about tenfold (a crossing table per voxel in place of the cell search, the coarse levels without the refine walk). Neither is shown to keep phase 2's accuracy.

## 16. Cutting the cost, 24 Nov 2026

Anthony chose to cut the cost and recheck the accuracy. Converged references (131,072 samples a block), `--cascades 5 --t0 2.5 --probe-px 12`, after the first frame.

**Each cone over its own interval** (`--own-interval`), as a sparse cascade traces, is about five times cheaper on the GPU. On its own it leaks: the room no lamp lights goes from +11% to +50%. The march over every interval had a visibility mask for free: a coarse cone closed before its own interval gave its probe no value in that direction. With its own interval only, the cone starts past the wall and brings the lit room's light back, wherever the reader's finer cascade has not reached that wall yet. Marching that near stretch again with the full rules (`--near-check fine`) brings the room back to 11.4%, at the old cost. With the filtered read only, it reaches 31%.

**The chain** (`--near-check chain`) gets the mask for no march. Every probe starts on the start voxel of one of its finer children (the one nearest its seat), recursively down to the voxel layer, and the cascades are traced finest first. The share open before a cone's interval is the product of what the finer probes on its chain passed, in the bins that hold its direction. A cone the chain closed is not traced, and its bin stays empty.

| View | Phase 2 as accepted | Own interval, chain, theta 8 | Own interval, chain, theta 4 |
|---|---|---|---|
| Hall | 5.3% (24.2 M cones) | | 8.6%, −3.9% (2.4 M) |
| Room no lamp lights | 11.1% (14.6 M) | 14.3% (6.1 M) | 10.7%, +10.0% (1.4 M) |
| Sponza atrium | 12.9% (16.7 M) | | 15.4%, −10.7% (1.4 M) |
| Sponza dark corner | 15.9% (9.6 M) | | 20.5%, −8.8% (1.1 M) |

Tried and dropped: theta 4 or `--probe-px 24` without the chain (room +40% and +67%); the refine walk in the first cascades only (`--refine-cascades`: in the bounce gather it loses the transfer entries, which are recorded only by the crossing test and the walk, −38%; in the cascade it leaks); the merge's visibility test aimed at a coarse probe's start voxel (`--seat-at-start`, no better).

GPU, hall, theta 4 with the chain: 165 ms for 2.4 million cones (16 steps and 95 reads a cone); 3.6 million cones closed by the chain are not traced. The cones are 10 times fewer than phase 2's and the time 45 times less, but the step with the crossing test and the walk costs about 68 ns a cone, and has to fall about 20 times. Not yet timed: the bounce gather (about 3 million cones a frame in the hall over every interval, and 256 sweeps of the transfer), the merge and the shade.

**The descent** (`--descend`): a covered coarse step is walked front to back from its coarsest half down. A piece whose start, middle and end are outside the dilated occupancy (every cell with a surface and its 26 neighbours, per level) is passed whole; only the rest goes down to the refine level's crossing test. The room, the hall and the atrium read the same to four digits with it, so it passes only space the crossing test finds nothing in. GPU, hall: 165 → 102 ms; 16.2 → 12.7 steps a cone.

**The shader's local arrays.** The crossing search kept 16 cells and 24 crossings in per-thread arrays, and they spilled from the registers. Each cell is now handled as it is found, a cell already in the first end's block is skipped, the crossings array holds 8 and is a per-invocation global, not an `inout` copy: 102 → 16.2 ms (least 12.0) for the same 2.4 million cones, the same steps, reads and mean cover. A workgroup of 32 or 128 in place of 64 made no difference.

The cascade's cones in the hall are now 16 ms on the RTX 4070 at 720p, against phase 2's 7.4 s. Still to time on the GPU: the bounce gather (patches times 32 cones over every interval, and the transfer sweeps), the merge and the shade. The budget for all of lighting is 8 ms at 1080p.

**The bounce gather on the GPU.** `--gpu-bench` now also times the per-frame gather cones (each patch's cones over every interval, the lamps' and the sky's light D). Hall at the settings above: 127 ms for 2.1 million cones (65,820 patches of 25 cm, 32 cones each; 80 steps a cone).

- `--transfer-sweeps` (default now 32, was 256): the room reads 12.7% at 8 sweeps, 8.2% at 16 (fewer bounces, so less of its +10% bias), and 10.7% at 32, 64 and 256.
- `--gather-level 5` (50 cm patches) with `--gather-cones 16`: 290,608 gather cones (18,163 patches), 16.4 ms.

| View | 25 cm, 32 cones | 50 cm, 32 cones | 50 cm, 16 cones | 25 cm, 16 cones |
|---|---|---|---|---|
| Hall | 8.6% | | 9.2%, −4.3% | |
| Room no lamp lights | 10.7%, +10.0% | 7.4%, −6.4% | 4.8%, −1.4% | 11.8%, +11.2% |
| Sponza atrium | 15.4%, −10.7% | 16.6%, −13.4% | 16.5%, −13.1% | 16.8%, −10.8% |
| Sponza dark corner | 20.5%, −8.8% | | 21.4%, −11.2% | |

GPU total so far, hall, 720p: 15.9 ms for the cascade's cones and 16.4 ms for the gather's. Not timed: the transfer sweeps, the merge and the shade.

**A fixed gather cone width** (`--gather-width`, default 0: off): 0.4 or 0.75 a metre at every distance takes the gather from 16.4 to 10.5 or 11.8 ms, but the room goes to 9.2% and 11.4% and the atrium to 19.7% and 21.0%. Not taken.

**The GPU march against the CPU's, cone by cone.** `--gpu-bench` now prints each cascade's mean cover on both, and the cones the CPU closes and the GPU leaves open. With the 96-texel clip (6 m at the finest level, 12 m at the next) the GPU's cover is 0.14 against the CPU's 0.44 in the hall, and 0.27 against 0.47 in the room: the cones it leaves open start at the clip's edge or outside it, so its time is for a march through a mostly empty field. With `--gpu-side 192`:

| | Room | Hall |
|---|---|---|
| Cover, GPU / CPU (all cascades but the last) | 0.444 / 0.466 | 0.308 / 0.437 |
| Cascades 1–3, GPU / CPU | equal to three digits | 0.066–0.662 / 0.103–0.766 |
| Voxels inside the clip | 321,768 of 1.3 million | 256,916 of 1.3 million |
| Cascade cones, ms | 11.2 | 25.5 |

The shader reads the field as the CPU does (the room's cascades 1–3 agree); the difference was the clip. The hall's time is still low, as a fifth of its voxels are inside. The dense clip volumes are 3.7 GiB at 192 texels. A renderer would need a sparse brick structure for the field, whose walk costs more than a texel fetch.

Where phase 3 stands (hall, 720p, RTX 4070): the cascade's cones at least 25 ms, the gather at least 16 ms (timed at the 96-texel clip), the transfer sweeps, the merge and the shade not timed. ADR 0013's budget is 8 ms for all lighting at 1080p. Accuracy at these settings: hall 9.2%, room 4.8%, Sponza atrium 16.5%, dark corner 21.4%.

## 17. The experimental path in the renderer, 25 Nov 2026

Anthony's decision: there is no rejection and no revert. Cut the cost to 10 to 12 ms or under with a sparse voxel field, and put the path in the renderer now, so the editor shows it, behind a setting that is off by default. Phase 3's gate (cost under today's lighting, then the ADR) is not met; the experimental path does not wait for it, and the old path stays the default until the gates pass.

Stages, each its own change:

1. The field in `crates/render` (`cone_field.rs`), built on the CPU at load from the scene's surfaces and lamps, uploaded as sparse bricks; a debug view of it.
2. The cascade on the GPU: voxel-layer probes hashed from the G-buffer, cascades traced finest first with the chain mask and the descent, merged, and shaded into compose in place of the direct and bounce terms. Lamp light and shadows, no bounce.
3. The bounce gather and the transfer sweeps.
4. The sun (phase 2b, option B).

**The sparse field.** 8x8x8 bricks for each level over the whole scene, stored only where a surface or a lamp is. The fine levels (6.25 and 12.5 cm), which the march reads one voxel at a time, are compact: a 33-word header for each brick (a 512-bit mask and its counts) and a 32-word record for each voxel (light and area, and the plane for each way; the lamp term). The coarse levels, which it reads filtered, are dense bricks with a one-voxel apron in an atlas, so the hardware filter stays inside a brick. Dense bricks for every level took the hall to 4.3 GiB.

| Scene | Voxels | Fine bricks / records | Coarse bricks | GPU memory | Voxelise / brick (CPU) |
|---|---|---|---|---|---|
| Hall | 1.30 million | 16,071 / 1.23 million (152 MiB) | 1,608 (111 MiB) | 264 MiB | 5.4 s / 0.8 s |
| Sponza | 3.84 million | 39,593 / 3.66 million (452 MiB) | 2,215 (167 MiB) | 633 MiB | 45.5 s / 2.5 s |

Both are over ADR 0012's 200 MB. Next: a record for the ways a voxel faces only (most face one or two), and the lamp term only where a lamp is. The build runs off the frame thread.

**Stage 2, 25 Nov 2026.** `cone_cascades=on` (lighting key, debug-menu switch "Cone-traced cascades") runs seven passes before compose (`crates/render/src/cone_gpu.rs`, `shaders/cone_*.comp`): insert the voxel-layer probes from the G-buffer into each cascade's hash, compact them, chain-mask the cones, trace them through the field, weigh the probes, merge finest-last, and shade. The shade writes the pixel's light into the G-buffer's emitted word, and the direct pass, its shadows, the GI passes and the ambient fill are off. In the hall the picture has the lamps' light, contact shadows and the sky openings (screenshot, not graded yet); there are blotches on the floor near the camera. Not yet measured: the error against the converged reference, the cost, and Sponza.

The first run filled 30 GiB of memory and swap in a minute and systemd-oomd killed the session: the field's key holds the lamps' positions, the hall's lamps move, and every frame started a new build thread. One build runs at a time now; when it ends and the key has changed, the next starts from the world as it is then, so a moving lamp lags by one build (about 5 s in the hall in release). Peak resident memory in the hall is 2 GiB. A lamp written into the field without a rebuild is the next step for moving lamps.

## 18. Stability and exposure, 25 Nov 2026

**Measured.** Sponza at `[-13.39, 2.13, -4.41]`, yaw −4.621, pitch −0.102, 1262x1376, camera still, cone mode on: 42% of pixels pop each frame (the editor: 39%). `probe_px` 32: 0%. `theta` 2 (cone tables four times larger): 44%. 3 cascades: 30%. A fixed table overflows, and which probes are dropped follows the GPU's thread order. Shadows also move when the camera turns: a probe's seat is the mean of the visible pixels in its voxel, and each pixel picks its voxel level from its distance to the eye. The prototype graded single frames only, so neither showed there.

**Anthony's direction.** Cascade 0 is the voxel layer as section 3 says: visible pixels project into fixed world voxels, at fixed sizes that grow with distance. That layer caps the probe count by itself; a probe is never dropped. No probes off screen: surfaces off screen get their light from the gather patches (stage 3). Exposure gets partial adaptation and highlight protection.

**Rules for every run.** Headless only; no windows or pointer on the desktop. Under `systemd-run --user --scope -p MemoryMax=12G -p MemorySwapMax=0` (an uncapped run once filled memory and the session was killed). Outputs under `genos-debug/`, never `/tmp`. Wait for the scene to load (`luminance` over 0.01) before a setting changes: a placeholder world stands in while it loads. Each check is a script that prints PASS or FAIL and keeps its evidence. The CPU path trace is slow here (64x36 at 1 spp takes 4 s): small `spp` a batch and a long `seconds`, or the prototype's converged references. `a_scene_load_returns_inside_the_pump_budget` fails in debug builds already (Sponza 32.6 s against 30 s).

**Steps**, each its own commit with this file updated:

1. Commit the preset-lock fix (`cone_cascades` survives a `--preset` lock; test `a_preset_lock_keeps_the_cone_cascades_switch`).
2. **Exposure**, both paths. `histogram_ev100_from_counts` (`crates/render/src/hdr_output.rs`) brings the plain mean of the middle 80% to mid-grey, so standing in shadow blows out the lit areas. Partial adaptation: the target EV moves only part of the way from a scene key toward the metered EV (a 0..1 strength, near 0.7 to start). Highlight protection: exposure does not open past where a high percentile (near the 98th) clips; percentile and limit are settings. Settings per `docs/engine/settings-principles.md` (`HdrSettings` keys, MCP, debug menu); adaptation steps by frame time. Gate in Sponza: a pose in shadow looking at the lit atrium against the same view from a lit pose; the clipped share and the exposure gap both drop clearly below today's. The old path's gates stay green. Then Anthony judges it by eye.
3. **Probes and gates first; they fail now.** Read back the cone work counters (kept and dropped per cascade, dropped on insert) on MCP and in `debug/lighting` per `debug-standard.md`, and a probe that dumps probe keys and seats for a region. Gate A: still-camera flicker in cone mode at most the old path's at the same pose (hall checkpoint, the Sponza pose). Gate B: zero dropped probes there at 1280x720 and 1920x1080. Gate C: after a 5° yaw and a 10 cm step, every probe key seen at both poses keeps its seat within 1 mm. Gate D: turn away and back, and the frame is bit-identical. Into `scripts/ci-gate-knockout.sh` with named guards in `crates/render/src/gate.rs` and the presented-frame helper. Record which table overflows.
4. **The voxel layer to the design.** Seat from the voxel's own surface in the field, not the pixel mean; pixels only choose which voxels are active (this also removes the 32-bit position sums that can wrap). `probe_px` stays the voxel's least size on screen; level boundaries snap to the world grid, with a margin so a voxel does not flip. Every table (insert hash, probes, cones) is sized from the layer's cap, about pixels / `probe_px`² times a grazing factor, per level and cascade. Gates A to D pass; the frame cost at both poses goes here.
5. **Fast field.** Voxelise on the GPU from the triangles and shapes (cone mode on in Sponza in under 0.5 s; 50 s today). Lamps written into the field each frame on their own, so a moving lamp costs no rebuild; the CPU build thread goes. Gate: the field's cover of a known wall matches the CPU build's; a moving lamp's light follows in 1 frame.
6. **Grade, then stage 3 and stage 4.** Converged references: hall, room, Sponza atrium and dark corner, against section 13. Stage 3, bounce through the gather patches and light transfer. Stage 4, the sun as an emitter (option B).

Stop and report to Anthony when a gate cannot be met, when a step needs a choice this plan does not settle, or when step 2 is ready to judge by eye.

**Step 2 result, 25 Nov 2026.** Auto exposure now takes a highlight key from the frame (`auto_ev100`; `docs/systems/hdr-output-design.md`, "Highlight key"). The plan said "a scene key"; a fixed EV would follow each scene's light units, so the key is the EV that puts the 98th-percentile luminance 3 stops above where the mean goes, and when the frame meters darker than that, exposure goes only `adapt_strength` of the way to the meter. New `HdrSettings` keys `adapt_strength` (0.3 on the auto presets, tuned below), `highlight_percentile` (0.98), `highlight_stops` (3), each with a flag, a `GENOS_HDR_*` mirror, MCP and an Output-page slider. `tools/lighting/exposure_gate.py`, Sponza, under the arcade (the pose above) against the courtyard the same way and the dark corner:

| | Clipped at the arcade | Swing arcade to courtyard | Dark corner opens |
|---|---|---|---|
| Before (`adapt_strength=1`) | 3.7% | 3.62 stops | 2.64 stops |
| strength 0.7 | 0.72% | 2.88 | 2.71 |
| strength 0.5 | 0.38% | 2.38 | 2.77 |
| **strength 0.3 (default)** | **0.23%** | **1.88** | **2.81** |

Limits: clipped 1%, swing 2 stops, opens at least 2 stops. `make sponza-gate` passes all 14 checks. Already failing before this step and not from it: `legacy_shader` in `hdr_output_gate` ("40 bytes moved between frames", the same at `5a5e5058`) and `lighting_settings_round_trip_over_mcp` (the `gi-1bounce` world cache, on master since `edba58c6`). Waiting for Anthony to judge by eye.

**Step 3 result, 25 Nov 2026.** Probes `lighting.cone_stats` (per cascade: probes, cap, dropped past the cap, insert misses, cones traced and closed) and `lighting.cone_probes` (one cascade's keys, cells and seats), and `tools/lighting/cone_stability_gate.py` (gates A to D; `debug-standard.md`, "Cone-traced cascades probes"). Cone mode also turns off the denoiser and compose reuse now: it carries nothing between frames. Scene paused, before step 4:

| View | A popped (old path) | B dropped | C seats moved, 5° / 10 cm | D pixels changed |
|---|---|---|---|---|
| Hall 1280x720 | 0 (0) | none | 7,948 of 8,464 / 14,542 of 21,199 | 0 |
| Sponza 1280x720 | 0 (0.0005) | none | 12,527 of 15,614 / 11,815 of 15,874 | 0 |
| Hall 1920x1080 | 0.074 (0) | c0 16,737 past the cap of 32,768 and 10,370 insert misses; c3 33; c4 65 | | 1,390,410 |
| Sponza 1920x1080 | 0.181 (0.0003) | c0 8,437 and 706; c4 66 | | 1,315,147 |
| Sponza 1262x1376 | 0.232 (0.0013) | c0 3,957 and 129; c4 2 | | |

The tables that overflow are cascade 0's probe table and its insert hash, and cascade 4's probe table (256). Where nothing is dropped, a still frame does not pop and a turn and back gives the same picture bit for bit: the flicker is the drops. Seats move everywhere (gate C): they are the pixel mean. The earlier 28% to 70% still-camera numbers were wrong: `step` advanced the paused hall and moved its lamps, and field rebuilds landed in the measurement.

The gates go into `scripts/ci-gate-knockout.sh` in step 4, when they pass; that script has to stay green. Already failing before this work, on lavapipe: `gpu_frame`'s `a_bright_lamp_does_not_cross_an_opaque_wall`, `a_close_lamp_does_not_cross_the_wall` and `the_corridor_carries_bounce_around_the_bend`, besides the two AGENTS.md lists (`5a5e5058`).

**Step 4 result, 25 Nov 2026.** The voxel layer as section 3 has it. A pixel's probe is the field voxel its surface is filed in (a quarter voxel behind the surface, as the field files points); the level is the finest whose voxel is about `probe_px` pixels wide, measured from the voxel's centre to the eye snapped to a grid four voxels wide at that level. The seat is the voxel's **field seat**: the area-weighted mean of the surface points the field filed in it, stored as a seat word (fine records grow to 33 words; coarse texels use their spare word), lifted along the area-weighted normal of its ways. A voxel the field has no surface for (a mover, or a level past 8 m) is seated at its centre and counted (`seat_fallbacks`). The tables come from the layer's cap: cascade 0 holds 5 probes for each `probe_px` square of the screen (3.0 to 3.5 measured), each cascade a third of the one below, each hash at most half full; the layout is written into the work buffer's header each frame (`cone_layout`).

`tools/lighting/cone_stability_gate.py`, every view and size of step 3: every gate passes. A: no pixel pops. B: nothing dropped; the most full table is under half its cap. C: 0 of 14,298 to 36,279 shared probes move their seat after a 5° yaw or a 10 cm step. D: 0 pixels change after a turn away and back. In CI, `crates/render/tests/cone_stability.rs` runs B and D and C on lavapipe and goes red when `cone_table_cap` (22,176 dropped) or `cone_field_seat` (511 of 525 seats moved) is knocked out (`scripts/ci-gate-knockout.sh`).

Cost (`tools/lighting/cone_cost.py`, RTX 4070, scene paused, frame ms; the cone mode has no bounce and no sun yet):

| View | 1280x720 old / cone | 1920x1080 old / cone | Cascade-0 probes at 1080p | Seat fallbacks c0 |
|---|---|---|---|---|
| Hall | 5.1 / 21.6 | 8.4 / 34.1 | 37,170 | 31 |
| Sponza arcade | 16.1 / 21.9 | 21.2 / 35.7 | 27,726 | 461 |

The work buffer is 160 MiB at 1280x720, 364 MiB at 1920x1080 and 303 MiB at 1262x1376, on top of the field (264 MiB hall, 633 MiB Sponza): over ADR 0012's budget. The cone cost is the trace step; it is the next thing to cut after the field (step 5).

**Step 5, part 1: the lamps each frame, 25 Nov 2026.** The field holds surfaces only; its key is the world and its model, so a lamp that moves, appears or changes colour rebuilds nothing. `cone_lamps.comp` writes every point lamp's sphere (256 points, as the build did) into the **lamp hash** every frame, at every field level, in fixed point; the march reads a lamp's term from it at each fine step and trilinearly at each coarse step, wherever the field has a brick or not. **Lamp presence** (the coarsest-level cells within two of a lamp's) lets a coarse step far from every lamp skip those reads. Keys hold the cell relative to the eye's at that level (about ±32 m across and ±16 m up at 6.25 cm, more at coarser levels); a fine-level lamp term past that is left out and the coarse levels carry the lamp. Gate E (`cone_stability_gate.py`) and `a_moved_lamp_lights_from_its_new_place_in_the_next_frame` pass: the move changes 37,020 pixels of the test frame and the next frame matches the one 30 later; with `cone_lamp_follow` knocked out (the lamps written once, as when the field held them) nothing changes and the test goes red. Gates A to E pass in every view and size of step 3. D and E now grade pixels off by more than one display code: after a long run of camera moves, up to 6 of 2 million pixels in Sponza can move by one code (not the albedo, not reflections, not fog; the source is open).

Cost of the per-frame lamps, frame ms with baked lamps / per frame: hall 21.6 / 24.7 at 1280x720 and 34.1 / 39.7 at 1920x1080; Sponza 21.9 / 24.2 and 35.7 / 40.3. Part 2 is the GPU voxelisation.

**Step 5, part 2a: a faster CPU build (option C), 25 Nov 2026.** Measured in Sponza first: 51.4 million surface points; 25.8 s to sample them (most of it one albedo ray a point), 18.2 s to file them at 8 levels, 2.2 s to brick. The renderer's build (`surface_shape_points`, `FieldVoxels::from_surfaces`) now casts the albedo ray only on a triangle whose material has holes (the ray is the alpha test; cone mode does not use the albedo), samples on every core, and files the points on every core (one task a coarse level, four a fine one, each reading every point in order, so the sums are the serial build's bit for bit: `the_parallel_build_files_every_point_as_the_serial_one_does`). Against the reference build (`genos-single-frame-cascade --scene sponza --field-check`) every level's voxels and area are within +0.01%: the 0.1% of points on solid triangles whose albedo ray missed at an edge, which the reference dropped. Sampling 0.51 s, filing 3.51 s (from 44 s). Cone mode on to its first probes (`tools/lighting/cone_cost.py`): hall 1.8 s, Sponza 6.7 s (from about 6 s and 50 s). Gates A to E, `ci-gate-knockout.sh` and the validation gate pass.

**Step 4b: the light holds still while the eye moves, 25 Nov 2026.** Anthony saw the cone light flicker while moving. Gate F (`cone_stability_gate.py`): walk the camera sideways 1 cm a step and count pixels whose second difference over three steps is over 6 codes where depth and albedo held (a slide over a gradient is near 0, a pop is not). Before: the hall popped 5,500 to 8,900 pixels a step, the old path 1,100 to 1,400. The cause was not the level snap or which neighbours were on screen (fixing those moved it under 10%): it was probes the field has no seat for. A pixel on something the field does not hold (a lamp's fitting) made a probe at its cell's centre, inside the fitting, that saw the lamp at full strength; it spread up the cascades, and every probe and pixel near it read it. Those pixels come and go as thin geometry aliases, so the whole region flashed for one frame at eye positions under 1 mm apart, the same flash at the same eye. Now:

- A probe without a field seat is read only by its own pixels, and does not spread (`CONE_PROBE_SEATED`).
- A pixel puts in every probe its shade reads (the eight around it with a field surface), and a cascade-0 probe puts in the cascade-1 probes its merge reads (`cone_spread.comp`, a new pass between the compacts). From cascade 2 up a probe merges its parent only (`CONE_PARENT_ONLY`): neighbours there have no children to close their cones early and cost the hall 62 ms at 1080p, against 50 ms.
- The probe level is continuous in the pixel's distance from the eye (no snap). A blend across levels removed a little more popping but cost 30 ms more.
- A coarse probe's cones start where a walk down the field leads, not on whichever child is on screen; a cone the chain closes is written dark and closed, not left out.
- The hash clears wait for the compute before them.

Gates A to F pass in every view and size (hall 1280x720 and 1920x1080, Sponza 1920x1080 and 1262x1376). F: hall 2,623 against the old path's 1,087 at 720p, 4,024 against 2,306 at 1080p; Sponza 6,268 against 4,667 at 1080p and 10,434 against 5,626 at 1262x1376. Cone-mode frame at 1920x1080: hall 50 ms (39.7 before), Sponza 40 ms (40.3). Probes per `probe_px` square 2.8; the tables hold 6, each cascade a third of the one below. The `adapt_strength` default goes to 0.15 (Anthony: still a little strong at 0.3): clipped 0.20%, swing 1.50 stops, the dark corner opens 2.85 stops.

**The sun's exact ray, 25 Nov 2026.** From Anthony's picture of the arcade: cone mode turned the old direct pass off, and the sun reached a surface only where a coarsest-cascade cone read the sky, so its light fell in probe-sized blocks with a band where the probe level changed. The shade now traces each sun's one exact shadow ray a pixel (section 3's fallback; `cone_sun` in `cone_shade.comp`, lit as the old direct pass lights a pixel), and the arcade has sharp shadows. Anthony: commit it as is. Its hard edge aliases as the camera moves, so gate F walks with the suns off (it grades the cone light; the shadow edge is the picture's edge). Gates A to G pass in every view and size; frame at 1920x1080: hall 41.2 ms, Sponza 39.0 ms. Stage 4 (the sun in the field, option B) is still the way to its bounce.

Part 2b, the under-0.5 s field, needed a choice the plan did not settle: the whole scene's field on the GPU (A) or clip volumes around the eye as section 3 says (B). Anthony chose B.

**Step 5, part 2b: clip volumes about the eye, 25 Nov 2026.** The fine levels (6.25, 12.5 and 25 cm) are **field clip volumes**, 128 of their voxels across, about the eye; the levels from 50 cm up hold the whole scene. Each level is sampled at half its own voxel (the whole levels at 25 cm), only from the triangles that meet its volume; the whole levels above 50 cm are summed from their children (from 25 cm up a point is filed the same distance behind its surface, so the sum is exact). The three groups build at once, a level per task, and the bricks and their GPU bytes on every core, on the build thread. Past a level's volume the next coarser level stands in: the march steps at it, the refine walk and the visibility test take its filtered cover in place of the crossing test, and a pixel's probe level is not finer than the field holds there. A point's plane box covers the patch it stands for (half the square root of its area about it), so sparser points do not leave gaps between a voxel's planes: without it the hall leaked light at the seams and popped 6,082 pixels a step. A world's first field holds the whole levels alone (0.13 s in Sponza), so cone mode lights at once; the fine levels follow. The eye moving 2 m from the volumes' centre builds them again; the new field fills beside the one in use and takes its place before a frame (`cone_swap_next`), so nothing blinks (gate G).

Sponza, cone mode on to first probes: 0.23 to 0.27 s (from 6.7 s; the gate is 0.5 s), to the fine levels about 1 s; the full build 0.49 s on the build thread, 138 MiB (from 633 MiB). The hall: 0.1 s and 0.4 s. `the_field_about_the_eye_covers_a_wall_as_the_whole_build_does`: every level's area matches the whole-scene build within 0.1%, and the coarse build holds no clip voxels. Gates A to G pass in the hall at 1280x720 and 1920x1080 and in Sponza at 1920x1080 and 1262x1376; `ci-gate-knockout.sh` and the validation gate pass (the cone tests now wait for the fine levels, `Renderer::cone_field_fine`). Neighbours are spread up to cascade 2 again (`CONE_PARENT_ONLY` 3): the frame the clip volumes saved paid for them, and gate F in the hall at 1080p went from 2.6 times the old path to 1.8. Gate F's walk now runs along the camera's right (`look_direction`), and its old-path counts moved with it.

| 1920x1080 | old path | cone, step 4b | cone, clip volumes |
|---|---|---|---|
| Hall | 8.7 ms | 50 ms | 41.6 ms |
| Sponza arcade | 21 ms | 40 ms | 38.6 ms |

The cones still cost most of the frame: each cascade has four times the directions and, on surfaces, a quarter of the probes, so the coarse cascades trace as many cones as cascade 0.
