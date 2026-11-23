# Cone-traced cascades: implementation plan

Status: proposal, 24 Nov 2026. Nothing here is in the renderer. The evidence is a CPU prototype and a GPU timing of its trace step, in [`debug/single-frame-cascade/`](../../../debug/single-frame-cascade/README.md). A change to the product path needs an ADR first (phase 3).

## 1. What Anthony asked for

Recorded from the session of 23 and 24 Nov 2026.

- The lighting system handles any number of lamps, and the cost does not grow with them. A moving lamp costs the same as a still one.
- Lighting reacts in the frame. The churn in a room no lamp lights, and the slow follow after a lamp goes out, both come from the temporal mean that hides too few rays. The mean and the "a lamp moved, treat this cell as changed" rules should go.
- All lamps are part of the cascades, as Sannikov describes for Path of Exile: light sources are emitters, and everything is indirect light.
- No commitment to hardware ray tracing. It can be a faster tracer behind the same interface. It is not a requirement.
- Try cone tracing through our voxels, so the trace cost has a cap.

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
3. **Cascade** (exists, changed). The probes are today's level-0 voxels and upper cells. Each visible point marches one cone through the voxel field. The cone's width in each cascade's interval is that cascade's direction spacing. Each interval deposits its light and the share that passes through, as the paper's split ray does. The cascades merge from the last one down. Every cascade is recomputed every frame. There is no running mean, no refresh period and no change signal.
4. **Shade** (exists). A pixel reads the cascade-0 directions around it.
5. **Exact lamps** (exists, kept as an option). The sun and the brightest few lamps at a pixel can keep an exact shadow ray, because a cone's shadow is as soft as a voxel. Whether to keep them, and how many, is a decision for phase 2.

The tracer stays behind one interface (`trace_ray` in `lighting_common.glsl`, `Surfaces` on the CPU). The cone march, the software tracer and hardware ray queries are three tracers behind it. The prototype already runs rays and cones through the same deposit, merge and shade.

## 4. Errors to iterate on

Each row is a fault the prototype shows or a risk it has not tested. "Probe" is how the prototype or the renderer reads it. The order is the order of work.

| # | Fault | Measured | Cause | Candidates | Probe |
|---|---|---|---|---|---|
| E1 | Light leaks through walls into a room no lamp lights | +65% bias, 76% error (rays: +11%, 30%) | A voxel wider than a wall holds the lit surfaces behind it. The trilinear read reaches them before the wall has covered the cone. Narrower cones did not help (+85% and +91% at half and quarter width, before E8 was fixed) | Read only voxels on the near side of a surface that faces the cone. Cap the voxel level near thin walls. Keep occlusion at a finer level than light. Store a wall's two sides in separate voxels (an inset by the normal at every level) | `--eye 4.75393,1.7,7.51477 --yaw 0.0466 --pitch -0.079`: bias against the reference |
| E2 | The cascade itself is 30% off in that room, with exact rays | 30% error, +11% bias; does not fall with more frames | Prototype gaps against the paper: no blend across the level-of-detail seam, no depth-aware weights in the sparse trilinear read, a direction that straddles the horizon takes its whole cosine | Do what the paper does for each. Fix this first: E1 cannot be graded against a baseline that is itself 30% off | Same pose, `--tracer ray` |
| E3 | Lamp light through cones is blurred and still noisy at the tail | 16 to 21% error, 7.6 to 9.7% median noise, 46% at the 90th percentile | A lamp's light lands in one direction of 16 at cascade 0, so the cosine and the shadow edge are as coarse as that direction. A cone whose axis misses the lamp's voxels reads less of it | More directions at cascade 0 for the lamp term. Aim the lamp's voxel footprint at the cone width. Exact shadow rays for the brightest few lamps (step 5) | `--mode emissive --lamp-radius 0.05`, hall |
| E4 | A slanted surface shows too much area | Not measured apart; up to 1.73 times for a diagonal | Area is kept by six axis directions, and the sum over them is the L1 norm of the direction | Keep a mean normal and one area, or renormalise by the L1 norm | A test wall turned 45 degrees |
| E5 | Nothing is known past the clip volumes | 126,174 of 1,867,174 voxels were inside them in the hall | A level covers 96 voxels a side. The fine levels end 3 m from the eye | Coarser levels cover the far field; check that a cone moves to them before it leaves a fine level. Size the clip volumes for each preset | Voxels inside the clip volumes, and the mean cover of a cone (0.942 in the hall) |
| E6 | Contact detail under one voxel is lost; a one-sided surface is clear from behind | Not measured | A cone starts 0.75 of the finest voxel off its surface, and only surfaces that face the cone cover it | A short exact ray, or a screen-space step, for the first voxel. Two-sided cover for surfaces marked two-sided | Contact shadow checkpoint `contact`; a curtain |
| E7 | Two partial occluders in a row count as one surface | Not measured | Cover adds up along the cone and stops at 1. That is exact for one surface crossing several voxels and wrong for two separate ones | Multiply past the first full voxel of distance. Grade both against the reference | Two half walls, one behind the other |
| E8 | One light sample shared by a patch lights the dark side of a wall | +85% became +65% when the patch went from 25 cm to 6.25 cm | A patch of floor or roof that runs under a wall took one sample for both rooms | In the full design the light is the cascade's irradiance at probe spacing, so the same fault returns at the probe size. The read of last frame's irradiance into a voxel needs the wall-aware weights of E2 | The glowing line where a wall meets the floor and the roof |
| E9 | Bounce through feedback is not built | - | Step 2 of the design | Build it in the prototype before the renderer. Check the energy against the path-traced reference at 1, 2 and many bounces, and in the white room (albedo 1), where each bounce loses little | Mean irradiance by frame after a lamp goes on |
| E10 | The picture under a moving camera is not tested | - | Clip volumes snap to the grid, the level of detail of a probe changes with distance, and every frame takes new directions | Hold directions per probe. Blend the level of detail. Grade pan, strafe and walk as the Sponza gate does | The Sponza gate's pan checks, on the prototype's output |
| E11 | Movers and moving lamps are not tested, and the voxel upkeep is not timed | - | A mover re-writes its voxels. A lamp re-writes its own | Time both. A lamp is cheap. A mover is the open cost | Milliseconds to re-write one mover's voxels |
| E12 | Half-float voxels clip | Not seen yet | Light sums are radiance times area; a lamp's is its colour times 72 | Scale by level. Check the brightest lamp and the sun-lit roof | The largest value written to a voxel texture |
| E13 | Only the analytic building is tested | - | The prototype loads a `kind stress` scene | Load glTF through `GltfSurface`. Sponza has curved surfaces, leaves and thin cloth, where E1, E4 and E6 bite | Sponza atrium, dark corner and curtains |
| E14 | Systems that read the lamps are not covered | - | Fog takes its lamp picks from ReSTIR (ADR 0026). Reflections and specular read the direct pass | Decide for each whether it reads the voxel field or keeps its own lamp path | - |

## 5. Cost to iterate on

- **The march.** 11.8 ms for 910,660 cones in the hall: 29.9 steps and 104.8 filtered reads a cone, from seven textures. Candidates: one cone for each cascade-0 probe direction in place of one a pixel (a probe is 8 pixels wide, so about a quarter of the cones), the lamp term folded into the direction textures (three reads a step in place of four), longer steps at the coarse levels, and a stop at full cover that the hall's open cones do not reach.
- **Memory.** Seven 96 x 96 x 768 RGBA16F volumes are 378 MiB. ADR 0012's lighting budget is about 200 MB, and ADR 0036 sizes the light cache for each preset. Candidates: RGB9E5 or R11G11B10 light with an 8-bit area, four directions' worth packed in two textures, fewer levels.
- **Voxel upkeep.** Not timed. Still geometry is free after load. Movers and the surface-light write (step 2) run each frame.
- **The shared steps.** Probe insert, deposit, merge and shade are not timed. The paper gives 7.3 ms on an RTX 3080 laptop. The renderer has a level-0 insert and a merge today; their times are in `lighting.gi_metrics` and the cascade timers.
- **The budget.** ADR 0013 caps lighting at 8 ms at 1080p. Today's renderer is 11.4 ms at 720p with 6 lamps.

## 6. Phases

Each phase is its own change, with the gate it names. A phase that misses its gate stops the plan there: a note in this file, and the renderer is left as it is. Phases 0 to 2 are the CPU prototype only.

| Phase | Work | Gate |
|---|---|---|
| 0. A baseline to trust | E2 in the prototype. glTF scenes (E13). The same error measure as `capture.compare` (block means), so the prototype and the renderer are graded alike | Ray tracer, one frame: the room no lamp lights within 10% of the reference, the hall within 7%, Sponza atrium and dark corner within 10% |
| 1. The leak | E1, E4, E7, E8 with cones | Cone tracer: the room no lamp lights within 1.5 times the ray tracer's error from phase 0, and bias under 10%. The hall and Sponza no worse than 1.5 times |
| 2. Lamps and the full loop | E3, E9: lamps as emitters and bounce through feedback, together. Decide how many exact lamps stay | The full picture (lamp light and all bounces) within 10% of the path-traced reference in the hall, the room and Sponza after 8 frames, and within 15% in the frame after a lamp goes on. Noise under 3% median |
| 3. Cost, and the decision | The GPU march tuned (section 5). The voxel write for a mover and for the surface light, timed. Camera motion and moving lamps on the GPU prototype (E10, E11). Then the ADR | Trace step, voxel upkeep and the shared steps together under today's `lighting_ms` at 6 lamps, and flat to 48 lamps, on the RTX 4070 at 720p. Memory inside the preset's budget. Anthony accepts the ADR, or the plan stops |
| 4. The voxel field in the renderer | The field and its upkeep in `crates/render`, behind a setting, with the software tracer still the default. Probes and the debug binary (section 7) | The field's cover of a known wall matches the CPU prototype's. `make sponza-gate` and `make moving-lamp-gate` unchanged with the setting off |
| 5. Cones in the cascade | The cascade takes its radiance from the cone march, every cascade every frame. Lamps are written into the field. The per-lamp loops go: the pixel's cascade visibility sample for each lamp, and `lighting_cascade_direct_at` at every ray hit | The phase 2 numbers, read from the presented frame. `lighting_ms` flat from 6 to 48 lamps. A new gate in `scripts/ci-gate-knockout.sh` for the leak (the room no lamp lights) |
| 6. Remove the temporal rules | Delete the lamp share, the drift rate, the change signals, the refresh period and the long mean in `lighting_cascade_upper_shade.comp`, and the level-0 long mean if it has landed | The moving-lamp gate's noise limits hold with no temporal mean, and its follow check passes in 1 frame |

## 7. Probes, tunables and the debug binary

The prototype is `debug/single-frame-cascade/`. From phase 4 the coverage moves into `debug/lighting`, as the rubric requires.

Tunables, each a setting under [the settings principles](../../engine/settings-principles.md) from phase 4: the tracer (cone, software, hardware), the cone width, the cascade-0 probe size in pixels, the direction count at cascade 0, the cascade count, the clip volume side for each level, the finest and coarsest voxel level, the count of exact lamps.

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

A view of the voxel field (cover and light, by level) goes in the debug menu with the other lighting views.

## 8. Networking

- Replicated state: none. The voxel field and the cascades stay on the machine that shades.
- Authority: the machine that shades. A lamp's position, colour and size are scene state, with the scene's authority.
- Prediction / rollback: none. A rollback of the scene re-writes the movers' and lamps' voxels the next frame.
- Determinism: the field is a function of the scene and the eye's clip volumes. The cone directions are a function of the frame number. Not bit-exact across GPUs (texture filtering).
- Headless server: does not build the field or the cascades.

## 9. Decisions still open

1. Whether the product keeps exact shadow rays for the sun and the brightest few lamps. Cones give a shadow as soft as a voxel.
2. Hardware ray queries as a second tracer. At 6 lamps the ray trace step is about 3.3 ms against 11.8 ms for the untuned march. ADR 0012 lists "Hardware RT as a requirement" as rejected; an optional tracer does not conflict with it.
3. What a preset without the memory for the field gets: no GI, or today's software path kept alive beside the new one.
4. The other session's level-0 long mean (branch `lighting/post-merge`, PR #322). Phase 6 would delete it.

## 10. What stays from the session of 23 Nov

Uncommitted in the main checkout when this was written: the frame-time step for auto exposure, the lamp share and drift rate in the upper cells, the two corner-leak fixes (`CASCADE_RECORD_PLANE`, `CASCADE_SEAT_INSET`), and `make moving-lamp-gate`. They are the stopgap until phase 6. The gate stays: phase 5 and phase 6 are graded by it.
