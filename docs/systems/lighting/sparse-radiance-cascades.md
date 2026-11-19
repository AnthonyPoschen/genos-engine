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

Direct light stays exact. The sun and every lamp inside the importance cutoff keep their shadow rays. Lamps under that cutoff stay the stochastic tail they already are. Distant lamps, the ones the cutoff leaves out of the exact sum, fold into the upper levels instead of a second full sum. Area sampling of a lamp for a soft penumbra is a later item. It is not this conversion.

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
- The importance cutoff stays the exact/tail split it already is. A cache ray is not a product pixel (`idx < pixels` is false), so the upper-level shade sums every lamp, distant ones included. The per-pixel tail stays the direct remainder. Removing it would drop hall energy or count the same light twice. Distant lamps are not traced as extra exact lights.
- Lamp area sampling for a soft penumbra is deferred.
- The finest bounce cell, 25 cm, keeps the full trace from the ray epsilon out to 200 m. The papers start level 0 near 1 m. A cell at least twice that size has a shell from `2^level` to `2^(level+1)`, and the last shell runs out to 200 m. Clipping the 25 cm cell before the shells are summed raised wall splotch from 0.049 to 0.084, so that cell keeps the full trace.
- Picture compose loads the per-pixel level-0 word. It does not walk the hash. That walk faults lavapipe. A publish pass, after the lattice fill, writes the merge into that word. The lattice holds 16384 records, so a room evicts the farther surfaces and that record is empty while the world cell still has samples. A miss reads that cell, then each coarser size until one passes the plane, facing, stale, and empty gates. A camera move inserts voxels that have no ray and no stored coarser cell yet. Those read the fine cells already traced on that surface, then the inward pixel's resolved sample, out to 8 m, in the same frame. Beta is 0 while the nearer sample exists. Black only when none of those has data.
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
