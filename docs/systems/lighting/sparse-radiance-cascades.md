# Sparse cascade

Intent for the bounce field that replaces the cell-only picture. The implementation commits on this branch follow this note. GI v1's radiance cascades stay removed. This system does not bring that path back.

Sources: Freeman, Sannikov and Pitt-Francis, arXiv 2607.20384, and Osborne and Sannikov, arXiv 2408.14425. Where this note departs from those papers, the departure is named below.

## Level 0

Level 0 is the surface sample the pixel reads. Nothing else is in the picture.

Each frame, the visible G-buffer positions are snapped onto a voxel grid and inserted into a sparse hash. The key is the cell coordinate plus the level. The grid size keeps a constant projected size on screen: distance rings, like a cascaded shadow map, with Chebyshev distance and about a tenth of overlap so a ring edge does not pop. The map is rebuilt every frame. A mover needs no separate pool and no change list. Its surface is in the G-buffer, so the next frame's level 0 is the new surface.

## Upper levels

Upper levels are the coarse world cells that already exist. Each level up doubles the spacing. The rays for level i cover the interval from about `2^i` to the next level's start, which is the penumbra condition in the papers. A higher level traces more directions. The merge stores SH-L1, averaged before the merge, so a coarse level is one directionally resolved colour rather than a raw ray.

Off-screen surfaces and long-range light live here. A distant lamp and the sun's bounce live here. A lamp near the pixel stays in the exact direct sum and does not enter these levels.

The memory cap is the screen density, not the world size. Lighting VRAM stays at or under 200 MB.

## Sizes

A level-0 voxel is `2^n / 8` m: 12.5 cm, 25 cm, up to 32 m. The size at a surface is `K * dist / sqrt(max(|n . view|, 1/16))` with `K = 1/16`, about 44 by 80 pixels at 1440p. Until 21 Nov 2026 the base was 6.25 cm and `K` was 1/32. Doubling both cut the live voxels to about 30% (close view 4600 to 1318, night view 5266 to 1562, far view 4547 to 1412) with no visible change in the picture.

The larger cells let light past a 40 cm wall in two gates, `a_close_lamp_does_not_cross_the_wall` (base of the back face 68, limit 45) and `lit_views_stay_smooth_and_low_lamps_stop_at_the_wall`. A bounce ray that hit the floor at the wall's foot blended in the next coarser floor cell, which straddles the wall and is seated on the lit side. A hit now reads its footprint level, or the finer one, and never the coarser one (`lighting_cascade_bounce`). Both gates pass. The picture still blends the two levels. The room in `room_bounce.rhai` did not change (walls 0.98 to 1.02 of the reference, floor 0.90 to 0.91).

`a_bright_lamp_does_not_cross_an_opaque_wall` still fails with these sizes. It asks a wall face under a lamp straight above it to be darker than the lit floor. It is not a leak: drawn again, both faces settle near 174 against 190 for the floor, with either size. With these sizes the first draw of the west face overshoots to 190 to 192 before it settles, and the gate samples that draw. With the old sizes that draw read 186. Not resolved.

Cost of the old sizes on an RTX 4070 at 1440p: the cascade passes were 6.3 to 7.4 ms and the frame 18.1 to 19.5 ms. With the doubled sizes they are 2.2 to 2.6 ms and 13.5 to 15.2 ms.

## Voxel seat

A voxel's rays start from its seat. The seat is the cell centre, moved along the cell axis onto the surface of the pixel that owns the voxel. The owner is the pixel nearest the cell centre: every pixel of the cell offers a rank (distance in the high bits, pixel index in the low bits) and `atomicMax` keeps one. Until 21 Nov 2026 the first pixel to claim the slot seated the voxel, and that order is a race.

The seat pass (`lighting_cascade_seat`, in the upper claim dispatch) projects the centre seat into the picture. When the pixel there shows the same surface, the seat stays. When it shows another surface, a 4 cm ray back along the normal looks for a surface under the centre seat (`lighting_cascade_seat_real`). A hit keeps the seat: a floor carries on behind a wall or a box. A miss, or a start inside a solid, makes the seat the owner pixel's own point. A centre off the picture takes the same ray test. Without that ray every hidden centre moved to the owner pixel, and the room in `room_bounce.rhai` lost 9% (floor 0.83 of the reference, walls 0.93). With it the floor is 0.87 to 0.88 and the walls 0.96 to 0.97; before the seat check they were 0.90 to 0.91 and 1.02 to 1.04.

This fixed a one-frame dark flash on moving boxes. A pixel on the bottom edge of a box face can land a hair under the floor. Its cell is the layer under the floor, and the centre of that cell on the face plane is 12 cm under the floor. Rays from there saw the underside of the floor: the voxel read 0.4 against about 3 for its neighbours, lived one frame, and darkened the pixels above it. In the default moving hall over 160 frames, one-frame events of 40 pixels or more fell from 9 to 2, and the largest from 1491 pixels to 314. `examples/stress/scripts/mover_edge_flash.rhai` watches the side-face voxels around the nearest moving box for 200 frames: 26 read under 0.3 of their frame's median before, none now. A floor cell whose centre is under a box used to stay empty (`scene_inside`); it now takes the nearest visible floor point.

## Lamp cone

At night most bounce comes from the ceiling 0.4 m over each lamp. A probe's uniform rays hit that patch by luck, so new cells came out bright or dark by chance: walking in, the brightest small floor patch was 1.39 times its surroundings ten frames after arriving. Each level-0 voxel now aims 4 extra rays, and each upper cell 8, at a cone around its strongest point lamp within 16 m (`lighting_cascade_cone`). The cone holds 1.5 m around the lamp and is at most 1 rad. A uniform ray inside the cone carries no weight. A cone ray carries the cone's solid angle over the cone ray count, a uniform ray `2 pi` over the uniform count, and irradiance is the sum of weight, radiance and cosine, so the estimate is the same integral. A bin is the weighted mean of its rays. The patch ratio is 1.16 ten frames after arriving and 1.05 at two seconds. The room in `room_bounce.rhai` did not darken: floor 0.90 to 0.91 of the reference (0.79 to 0.88 before), walls and ceiling 1.01 to 1.03. The coarse band traces 56 uniform rays, not 64, to keep 64 threads.

## Dispatch

The level-0 trace and shade passes run one work group per seated voxel. The seat pass appends each seated slot to a list and counts the groups into an indirect dispatch command. Each upper band runs one group per due cell the same way: the pick pass raises the band's group count. Until 21 Nov 2026 every one of these dispatches was 16384 groups, most of them empty. Level 0 went from about 0.44 to 0.36 ms; the upper bands did not measurably change.

## Reach

An upper cell's rays run to 32 of its sizes (`lighting_cascade_interval`), from the surface. Level-0 rays stay at 8 cell sizes. Level-0 rays at 32 lit the outside of `a_lamp_sealed_in_walls_under_a_roof_lights_nothing_outside` (41 against 3), so only the upper cells reach farther. Until 21 Nov 2026 the upper cells stopped at 8. Past that the light came only through 16 parent bins, about 0.39 sr each, and a room lit through an opening by a sunlit wall lost the opening in the dark directions around it. At the room pose in `examples/stress/scripts/room_bounce.rhai`, against a 64-bounce reference: walls and ceiling 0.83-0.87 before, 0.94-1.04 after; floor 0.58-0.65 before, 0.77-0.84 after; the outside wall seen through the opening 1.12 before, 1.09 after. Leaving the rays that hit before 4S out of the bins, with a held bin when all its rays were blocked, made the room 0.36-0.50: a parent sits where the child's rays stop, so those near hits are the light the child needs. The shadowed outer wall in `examples/stress/scripts/wall_ground_bounce.rhai` moved from 0.77 to 0.97 of the reference. Cost: an upper cell's rays can run four times as far. Not timed on the 4070. `lighting_bounces` does not limit the cascade (multi-bounce is the history lookup in `lighting_cascade_bounce`), so `bounce_counts_brighten_the_shadow_and_skip_work`, which wants bounce 2 a level brighter than bounce 1, now fails at 0.4.

## Merge

Levels merge from the back toward the eye with transmittance:

`L(a, c) = L(a, b) + beta(a, b) * L(b, c)`

The sample is a sparse trilinear over the parents that exist, renormalised so a missing parent does not darken the result. A missing level contributes nothing, and the coarser level shows through. A pixel is black only when no level has data. That is the same rule the 25 cm lattice already uses for an empty cell: the child weight is its sets over a full cell, and a refused parent walks to the next valid one.

A ray past the child's interval reads the one parent bin that holds its direction (`lighting_cascade_bin_of`). A bin is the mean radiance of its own pi/8 sr, 4 rows of equal cos(theta) by 4 columns of 90 degrees around the cell axis. Until 21 Nov 2026 the ray read a bilinear blend of the four nearest bins. On a vertical wall that blend pulls a lower column, the ground, into the directions above the horizon. A shadowed outer wall facing sunlit ground took 2.24 times the reference with the sky off. With the bin that holds the ray it is 0.76 times (`examples/stress/scripts/wall_ground_bounce.rhai`). At the pose that showed it, mean error over the picture fell from 0.084 to 0.058. The wall is now somewhat dark: about 0.8 of the reference with the sky on and off.

## History

History stays one confidence-weighted mean. There is no full reset. A fresh sample is adopted when the stored mean is junk, and a zero-radiance ray is adopted only when the surface is already dark or a full set of rays agreed on dark. Temporal accumulation improves a frame. It is not what makes the first frame usable. One frame of level 0 is already a picture.

An upper cell's first eight refreshes take the new estimate whole (`CASCADE_UPPER_WARM`, counted in the irradiance word's w), so a cut builds its bounce through the chain of parents as fast as before. After that a refresh moves its 16 bins and its irradiance 0.05 of the way to the new estimate (`CASCADE_UPPER_RATE`; a quarter until the night-patch fix below). A change of more than 0.9 of its irradiance takes the new estimate whole (`CASCADE_UPPER_JUMP`). Until 21 Nov 2026 a refresh replaced them with one to four rays a bin. The cells feed each other's bounce, so a still night scene never settled, and the outer wall popped: 414 pixels a frame step moved more than 4 levels, against 20 now (`examples/stress/scripts/still_gi_pops.rhai`). A reset at 0.5 fired on that noise and made it worse (909). Cost: bounce that follows a moving lamp lags more. The frame the lamps stop, 0.45% of the bounce picture is more than 6 levels from settled, against 0.05% before.

After its warm-up, each refresh of a cell traces new directions: the jitter inside a bin changes with the frame (`lighting_cascade_bin_seed`). The warm-up keeps the fixed jitter, because a running mean of a few refreshes with new directions made a still wall shimmer in its first 32 frames (`a_still_far_wall_does_not_shimmer`, largest pixel step 0.12 against a limit of 0.08). Until 21 Nov 2026 the jitter was always fixed per cell, so the damped refresh averaged the same rays. At night most bounce comes from the ceiling 0.4 m over each lamp, a patch about 1/100 of a floor cell's hemisphere. One of a cell's few rays in a bin hit it or none did, and the floor under each cell showed a fixed bright patch. With new directions the rate had to drop to 0.05: at a quarter a still scene shimmered (largest frame step 0.11). On the night view of `examples/stress/scripts/night_lamp_patches.rhai` the brightest small floor patch fell from 1.60 to 1.11 times its surroundings, the bounce mean did not drop, and the largest still frame step is 0.02. A clamp on each ray's share removed the patches too, but it removed the ceiling's light with them: the picture was 16% darker than the path-traced reference. The moving-lamp lag did not change (0.21% of the bounce picture past 6 levels, against 0.23%).

## Direct light

Screen pixels keep an exact disc ray for the sun and for the single brightest near lamp. Every other near lamp is a cascade visibility sample, not a per-pixel shadow ray and not the bounce field. The level is `floor(log2(distance))`, clamped to 0..6, so a lamp inside 2 m uses the 25 cm grid and a lamp in `[2^k, 2^(k+1))` uses the cell of that shell. The outer tenth of the shell blends the next coarser level. The stored value is visibility, 0..1. The pixel multiplies it by its own unshadowed `lamp_light`, so the inverse-square is not quantized. The brightest lamp is left out of that add. The table is 262144 slots in the restir buffer, past the uploaded prefix, two parities, 8 MiB. It is open addressing over four slots from the key's home. A slot's first word is the key's high 24 bits over the frame's low 8 bits, so a slot from an older frame is free and one compare-and-swap claims it. Until 21 Nov 2026 the table was 16384 direct-mapped slots. Two keys on one slot overwrote each other, every pixel of both traced its own ray, and about four lookups in ten missed. At 1440p with five lamps that was 3.2 ms of a 7.9 ms direct pass. Same script and phases before and after (RTX 4070, 1440p, close pose): with five lamps the direct pass went from 9.0 to 7.3 ms and the frame from 16.6 to 14.5 ms. With 50 lamps the direct pass went from 58.1 to 21.7 ms and the frame from 70.1 to 32.6 ms. With no lamps it is 3.5 ms either way. The key stays on the cell, the shell, and the lamp. The visibility count caps at 32. A move keeps that mean and scales the count by `cell / (cell + moved)`, where `cell` is the shell's cell size. A small move keeps most of the count. A move of many cells keeps little. A missing slot is the reset, and a lamp move is not. One invocation claims the slot and traces; the other pixels of that cell read the previous frame's mean. A miss traces from the pixel, so the first frame is not black. Bounce rays are unchanged: a cache ray still sums every lamp into the cell. Distant lamps stay in that upper-level shade.

## The sun in the cascade

The exact sun stays. This is the comparison, not a second sun path.

`SUN_DISC` is 0.00465047 rad. A receiver a distance `s` past an occluder sees a penumbra about `s * 0.00465` wide. Level 0 is a 25 cm cell. The penumbra is narrower than that cell until `s` is about `0.25 / 0.00465`, near 54 m. A contact shadow in the hall, a few metres of occluder distance, is a few millimetres to a couple of centimetres. A 25 cm cell would step that edge by a cell. The outer shells are coarser. Putting the sun only in the cascade would replace the disc mean the penumbra gate grades with a cell-sized step.

Cost of a cascade sun: one disc ray per cell per frame, the same claim as a point lamp, instead of one ray per pixel. At 1440p that is a large cut. The edge inside 54 m is what the cut spends.

Recommendation: hybrid. Keep the exact sun for the pixel. A cascade sun is the far field, past the distance where the penumbra is wider than the cell that would store it. That split is not built. The exact disc ray remains the product sun.

## What this conversion deletes

These existed because the picture read the 25 cm cell directly. They are not a second path:

- The picture's child/parent replace. A coarser sample shows through the merge, and only when the nearer sample is missing. A bounce ray that hits an empty texel still reads the world cell the rays deposit into.
- The 1-in-10 wait. Every resident cell is due every two frames. Level 0 is rebuilt every frame.
- The live-scaled burst. The upper levels keep the preset burst, a fixed ray count.
- The mover texel pool, its struct, its buffer, and its shader. A mover's surface is the G-buffer sample.
- The reach change-list disc. A direct-shade step and a wide cell still mark a tile. A mover sphere does not.

## Departures

- The papers trace a cascade volume. This conversion reuses the existing world-cell hash and the existing software tracer. There is no hardware ray query.
- Level 0 is a screen-density hash rebuilt from the G-buffer, not a probe brick stored across frames.
- SH-L1 is the directional store. The papers' higher-order angular basis is not this conversion.
- Product pixels no longer split near lamps into an exact cutoff and a stochastic tail. The sun and the brightest near lamp stay exact. The other near lamps are the cascade visibility table above. A cache ray is not a product pixel (`idx < pixels` is false), so the upper-level shade still sums every lamp, distant ones included. The bounce merge is not this direct table. Adding a cascade lamp on top of its bounce would count it twice; the direct table is visibility for the pixel's own `lamp_light`, and the bounce stays the indirect field.
- A pixel's exact shadow samples one point on the lamp disc (or the sun disc) per frame. The running mean is the penumbra. That sample is the direct pass, not this conversion. Hits stay on the centre.
- The finest bounce cell, 25 cm, keeps the full trace from the ray epsilon out to 200 m. The papers start level 0 near 1 m. A cell at least twice that size has a shell from `2^level` to `2^(level+1)`, and the last shell runs out to 200 m. Clipping the 25 cm cell before the shells are summed raised wall splotch from 0.049 to 0.084, so that cell keeps the full trace.
- Picture compose loads the per-pixel level-0 word. It claims a 25 cm cell only when that word is empty. It does not walk the hash. That walk faults lavapipe. A publish pass, after the lattice fill, writes the merge into that word. The nearer sample is the confidence-weighted lattice sample. The centre counts twice, each face neighbour out to 6 cells counts once, and a parent ring mixes in by the centre's trust (its sets over a full cell). A young centre keeps the parent. A full centre keeps the weighted sample. Showing the centre at full weight measured wall splotch 0.056 to 0.059 against the 0.053 gate. Compose does not walk. A camera move does not rebuild the lattice. The opening, the 32-frame refresh, and a mover's reach still request it. The lattice holds 16384 records, so a room evicts the farther surfaces and that record is empty while the world cell still has samples. A miss reads that cell, then each coarser size until one passes the plane, facing, stale, and empty gates. A camera move inserts voxels that have no ray and no stored coarser cell yet. Those read the fine cells already traced on that surface, then one neighbour cell toward the image centre, one hash of that surface, in the same frame. Beta is 0 while the nearer sample exists. Black only when none of those has data.
- The merge's nearer term is that full integral. Beta is 0 while it exists, and the parent shows only when the nearer sample is missing. Adding the parent on top of the full integral would count the same light twice.

## After the conversion

Per-pass timings are recorded once, when the cascade is feature complete. They are not part of the conversion commits. The frame-rate target, a worst frame above 100 fps on the stress scene and about 500 fps on the basic scene, is the phase after that. Lavapipe times are the wiring check. They are not that target.

## Known issue

The dark room behind the lit room is not fully closed. A 4070 shot shows thin lit slivers where the shared wall meets the ceiling and the floor, and a lit rectangular patch on the floor near that wall. The rest of that room stays black. Cells or level-0 patches that straddle the junction are the likely cause. The leak fix is held. The thin-wall test still covers a surface the gates refuse. It does not cover these junctions.

## Gates the conversion keeps

Knockout late error at or under 0.213. ReSTIR reference error 0.05. Compose SPIR-V under 248,000 bytes. Hall energy all 120..136 and mid 101..113, exclusive. Moving-box face and mover flicker, hot limit 3. Floor splotch at or under 0.045 and wall splotch at or under 0.053. The cell-sized black-square test. A moving camera after the hall has lit, with no published sample left empty. A thin-wall leak test: light does not cross a surface that the leak gates refuse.

## Networking

- Replicated state: none. Level 0, the upper cells, and the merged picture stay on the machine that shades.
- Authority: the machine that shades.
- Prediction / rollback: none.
- Determinism: the same pose, the same G-buffer, the same lamps, and the same cell keys produce the same level-0 insert and the same merged colour.
- Headless server: a server that does not shade leaves the cascade uncomputed.
