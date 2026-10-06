# Lighting

Genos Engine lights a scene with radiance cascades. The notes here are the working description. The decision is [ADR 0008](adr/0008-lighting-uses-radiance-cascades.md).

## Sources

Read these before changing the field.

- Alexander Sannikov, *Radiance Cascades: A Novel Approach to Calculating Global Illumination*. Working paper for the game-lighting form. Repository: <https://github.com/Raikiri/RadianceCascadesPaper>
- PDF: <https://github.com/Raikiri/RadianceCascadesPaper/blob/main/out_latexmk2/RadianceCascades.pdf>
- Source: <https://github.com/Raikiri/RadianceCascadesPaper/blob/main/RadianceCascades.tex>
- Overview that points at the paper: <https://80.lv/articles/radiance-cascades-new-approach-to-calculating-global-illumination>
- Christopher M. J. Osborne and Alexander Sannikov, *Radiance Cascades: A Novel High-Resolution Formal Solution for Multidimensional Non-LTE Radiative Transfer*. arXiv:2408.14425. <https://arxiv.org/abs/2408.14425>

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

The merged colors are blended between the four surrounding near probes. A lookup is not a copy of the single nearest square. The floor mesh asks for a color at each vertex, and the rasterizer blends those vertices.

The near probes are 16 cm apart across the whole floor. The opening camera uses that one field for every solid, wall, and the floor between them. There is no second, coarser spacing in that view. The field is kept until a light or an occluder changes. A hidden object that does not affect light is not part of that gather. A ray that starts inside a wall or a solid does not leave through the far side. That exit was painting the lit face onto the back face.

## Material color

Nothing is added in the dark. A point with no lamp and no bounced light is black.

A lamp reaches a point only when the straight ray misses every wall and solid. The ray uses those shapes. It is not a physics collision step. A wall taller than the ray stops the lamp. A ray that clears the top of a wall still arrives. Bounce rays already stop on the first surface in their interval.

Each face is lit from a point just outside that face, and only when that face points toward the lamp. A face that only grazes the lamp gets less of that lamp. Light from a lamp about one meter away stays below a flat white. Light a few meters from the lamp is dimmer than light next to the lamp. A floor point inside an object's footprint gets no lamp. A point on the lamp side, outside that footprint, stays lit. The shadow edge is the lamp ray that hits a wall or a solid. A sample at the center of a wall is inside the volume. That sample does not light the back face. A probe inside that volume stores no light. A shaded point does not read a probe across a wall.

Light that leaves a surface is the arriving light multiplied by that surface color. The arrival is the lamps plus the previous bounce pass. A material can set a reflectance from 0 to 1. A material can set how much of its color mixes into the bounce. A missing value keeps the built-in shares. Those shares are 0.55 of the lamp and 0.68 of the previous bounce, with the full surface color. A lamp above an object colors the floor on every side. A lamp on one side does not color the far side. A lit wall adds light to the floor shadow in front of that wall. The shadow is brighter near the wall than far from the wall.

A red solid in a white lamp throws red light. A white floor shows that red next to the solid, on top of the white the lamp puts there directly. A colored receiver multiplies the bounce by its own color as well.

The GPU builds the field in four passes. The first pass records light leaving each material under the lamps. Each later pass bounces that light once more and reads only the previous pass of the same build. A hit stores its color even when that color is black, so a dark face blocks the farther interval. A miss blends the next coarser range. Neither pass reads the previous frame. The CPU uploads positions, colors, texture ids, and lamp parameters. It does not compute the lit color.

A normal draw does not stall the game loop on that GPU work. A readback waits. Move or recolor an object and the next build follows the new scene.

## World probes

A world probe is the farthest cascade, not a camera probe and not a baked lightmap. Its grid is larger than the floor, so probes sit outside the floor as well as on it. Its rays start where the far interval ends. They carry the color of whatever material they hit, including a solid the nearer ranges cannot reach.

An empty nearer ray is the only place a world probe enters the merged field. A near hit is not replaced by a far probe.

## What is fixed, and what comes from the scene

Direction counts stay in penumbra order: near, then far, then world. Near spacing is 16 cm across the whole floor. Every ray starts on that fine probe. A miss keeps the same probe and uses the next range. Far quantizes to 36 directions. World quantizes to 64. A coarser neighbor is blended only when its hit agrees with the fine ray. Interval length and the margin past the floor follow the floor's longest side. The builder does not read the example solids, the example light, or the camera.

The example scene in `examples/camera/scene.rhai` is one scene. It is not the layout of the field.

## Budget

Four figures from the paper are the upper end: intervals that double, ray counts that jump by four, and a last cascade with hundreds of directions. That last jump is more than this ground-plane march needs.

The balance used here is three cascades.

- Near spacing is 16 cm. The floor, the walls, and the solids use that same cell. A coarser cell between those objects is the step this view must not show.
- Directions are 32, then 36, then 64. Near stays finer in space. A near miss uses the far interval. A far miss uses the world probes.
- The near interval is about 12% of the floor. Color from a solid stays next to that solid. The far interval reaches most of the floor. World probes go past the floor.
- Probe colors are blended in the fragment shader. Each pixel samples the field. A flat color per probe made the first pictures blocky.
- A hit returns about half of the lamp light, times the material color. There is no fill light. Empty space is cleared to black.

## What this cut does not do

Rays march on the ground plane. A full 3D volume of probes is not built yet. The GPU field stores the merged color. That field is not a lightmap. A new build does not read the previous frame. The engine keeps the finished field until a light or an occluder changes.

Emitting particles add one fire light on the ground probes. The light uses the particle positions and colors. A lit particle takes that field on its card. A density puff uses optical depth and in-scatter on the view ray. That medium is not a 3D probe grid.
