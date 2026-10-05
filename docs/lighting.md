# Lighting

Genos Engine lights a scene with radiance cascades. The notes here are the working description. The decision is [ADR 0008](adr/0008-lighting-uses-radiance-cascades.md).

## Sources

Read these before changing the field.

- Alexander Sannikov, *Radiance Cascades: A Novel Approach to Calculating Global Illumination*. Working paper, not a journal article. Repository: <https://github.com/Raikiri/RadianceCascadesPaper>
- PDF: <https://github.com/Raikiri/RadianceCascadesPaper/blob/main/out_latexmk2/RadianceCascades.pdf>
- Source: <https://github.com/Raikiri/RadianceCascadesPaper/blob/main/RadianceCascades.tex>
- Overview that points at the paper: <https://80.lv/articles/radiance-cascades-new-approach-to-calculating-global-illumination>

Sannikov shipped a screen-space form of this in Path of Exile 2. That choice fits a fixed camera. This engine moves the camera, so the probes stay in the world.

## What a radiance cascade is

A radiance field is the light at each point, in each direction. A cascade is one distance range of that field.

A radiance interval `L(a, b)` at a point, along a ray, is the light that leaves the first surface the ray hits between distance `a` and distance `b`. A miss stores no light. The ranges meet: the next cascade starts where the previous one ends.

The penumbra hypothesis is the reason the ranges are stored differently. Close to a surface you need many probe positions and only a few directions. Far from a surface you need few positions and many directions. A near cascade therefore has a smaller spacing and fewer directions than a far cascade. A world cascade is farther again, so it is coarser in space and finer in angle.

Each cascade is a grid of probes. A probe stores one color per direction for its interval only. The grid is anchored to the floor in the world. It does not follow the camera.

## How the ranges merge

At a point, walk the directions of the nearest probes.

- If the near interval hit a surface, use that color.
- If the near interval missed, use the far interval in that direction.
- If the far interval missed, use the world probe in that direction.

The merged colors are blended between the four surrounding near probes. A lookup is not a copy of the single nearest square. The floor mesh asks for a color at each vertex, and the rasterizer blends those vertices. The cell size follows the near spacing, which follows the floor size.

## Material color

Nothing is added in the dark. A point with no lamp and no bounced light is black.

Light that leaves a surface is that surface's color multiplied by the light arriving there. The arrival is the lamps plus the previous bounce pass. A red solid in a white lamp throws red light. A white floor shows that red next to the solid, on top of the white the lamp puts there directly. A colored receiver multiplies the bounce by its own color as well.

The field is built twice from the current scene. The first pass records light leaving each material under the lamps. The second pass lets that colored light bounce once more. Neither pass reads the previous frame. Move or recolor an object and the next build follows the new scene.

## World probes

A world probe is the farthest cascade, not a camera probe and not a baked lightmap. Its grid is larger than the floor, so probes sit outside the floor as well as on it. Its rays start where the far interval ends. They carry the color of whatever material they hit, including a solid the nearer ranges cannot reach.

An empty nearer ray is the only place a world probe enters the merged field. A near hit is not replaced by a far probe.

## What is fixed, and what comes from the scene

Direction counts stay in penumbra order: near, then far, then world. Spacing, interval length, and the margin past the floor are fractions of the floor's longest side. A larger floor gets a larger grid of the same density. The builder does not read the example solids, the example light, or the camera.

The example scene in `examples/camera/scene.rhai` is one scene. It is not the layout of the field.

## Budget

Four figures from the paper are the upper end: intervals that double, ray counts that jump by four, and a last cascade with hundreds of directions. That last jump is more than this ground-plane march needs.

The balance used here is three cascades.

- Near spacing is about 1/48 of the floor's longest side. A denser grid mostly repeats the blend between probes. A coarser grid reads as tiles.
- Directions are 12, then 36, then 64. Each step is about three times the last. Near stays finer in space and coarser in angle.
- The near interval is about 12% of the floor. Color from a solid stays next to that solid. The far interval reaches most of the floor. World probes go past the floor.
- Probe colors are blended. The floor mesh samples each vertex. One flat color per probe is what made the first pictures blocky.
- A hit returns about half of the lamp light, times the material color. There is no fill light. Empty space is cleared to black.

## What this cut does not do

Rays march on the ground plane. A full 3D volume of probes is not built yet. The mesh stores the merged color. The field is not a lightmap, and it is not reused next frame.
