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

These exist because the picture reads the 25 cm cell directly. Level 0 makes them redundant, and the conversion deletes them rather than leaving a second path:

- The child/parent replace that swaps a coarse cell for a fine one instead of merging by transmittance.
- The 1-in-10 floor. Level 0 is rebuilt every frame, so a cell does not wait a period to be seen.
- Burst budgets that grow or shrink with what moved. The upper levels keep a fixed ray count.
- The leftover mover texel pool.
- The reach change-list walk. A mover is a new level-0 sample, not a marked cell.

## Departures

- The papers trace a cascade volume. This conversion reuses the existing world-cell hash and the existing software tracer. There is no hardware ray query.
- Level 0 is a screen-density hash rebuilt from the G-buffer, not a probe brick stored across frames.
- SH-L1 is the directional store. The papers' higher-order angular basis is not this conversion.
- The importance cutoff stays the exact/tail split it already is. Distant lamps join the upper levels. They are not traced as extra exact lights.
- Lamp area sampling for a soft penumbra is deferred.
- The finest bounce cell, 25 cm, keeps the full trace from the ray epsilon out to 200 m. The papers start level 0 near 1 m. A cell at least twice that size has a shell from `2^level` to `2^(level+1)`, and the last shell runs out to 200 m. Applying that shell to the cells the picture still reads raised wall splotch from 0.049 to 0.084, so the trace stays the full range until the merge sums the shells.
- The merge's nearer term is that full integral. Beta is 0 while it exists, and the parent shows only when the nearer sample is missing. Adding the parent on top of the full integral would count the same light twice.

## After the conversion

Per-pass timings are recorded once, when the cascade is feature complete. They are not part of the conversion commits. The frame-rate target, a worst frame above 100 fps on the stress scene and about 500 fps on the basic scene, is the phase after that. Lavapipe times are the wiring check. They are not that target.

## Gates the conversion keeps

Knockout late error at or under 0.213. ReSTIR reference error 0.05. Compose SPIR-V under 248,000 bytes. Hall energy all 120..136 and mid 101..113, exclusive. Moving-box face and mover flicker, hot limit 3. Floor splotch at or under 0.045 and wall splotch at or under 0.053. The cell-sized black-square test. A thin-wall leak test: light does not cross a surface that the leak gates refuse.

## Networking

- Replicated state: none. Level 0, the upper cells, and the merged picture stay on the machine that shades.
- Authority: the machine that shades.
- Prediction / rollback: none.
- Determinism: the same pose, the same G-buffer, the same lamps, and the same cell keys produce the same level-0 insert and the same merged colour.
- Headless server: a server that does not shade leaves the cascade uncomputed.
