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

## Merge

Levels merge from the back toward the eye with transmittance:

`L(a, c) = L(a, b) + beta(a, b) * L(b, c)`

The sample is a sparse trilinear over the parents that exist, renormalised so a missing parent does not darken the result. A missing level contributes nothing, and the coarser level shows through. A pixel is black only when no level has data. That is the same rule the 25 cm lattice already uses for an empty cell: the child weight is its sets over a full cell, and a refused parent walks to the next valid one.

## History

History stays one confidence-weighted mean. There is no full reset. A fresh sample is adopted when the stored mean is junk, and a zero-radiance ray is adopted only when the surface is already dark or a full set of rays agreed on dark. Temporal accumulation improves a frame. It is not what makes the first frame usable. One frame of level 0 is already a picture.

## Direct light

Screen pixels keep an exact disc ray for the sun and for the single brightest near lamp. Every other near lamp is a cascade visibility sample, not a per-pixel shadow ray and not the bounce field. The level is `floor(log2(distance))`, clamped to 0..6, so a lamp inside 2 m uses the 25 cm grid and a lamp in `[2^k, 2^(k+1))` uses the cell of that shell. The outer tenth of the shell blends the next coarser level. The stored value is visibility, 0..1. The pixel multiplies it by its own unshadowed `lamp_light`, so the inverse-square is not quantized. The brightest lamp is left out of that add. The table is 16384 slots in the restir buffer, past the uploaded prefix, two parities. The key stays on the cell, the shell, and the lamp. The visibility count caps at 32. A move keeps that mean and scales the count by `cell / (cell + moved)`, where `cell` is the shell's cell size. A small move keeps most of the count. A move of many cells keeps little. A missing slot is the reset, and a lamp move is not. One invocation claims the slot with `atomicExchange` and traces; the other pixels of that cell read the previous frame's mean. A miss traces from the pixel, so the first frame is not black. Bounce rays are unchanged: a cache ray still sums every lamp into the cell. Distant lamps stay in that upper-level shade.

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
