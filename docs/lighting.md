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

Each cascade is one uniform grid of probes. A probe stores one radiance interval per direction, and the transparency β of that interval. The probes sit in the world. They do not follow the camera. Cascade 0 is the fine grid on the floor. Each next cascade doubles the spacing, doubles the direction count, and doubles the interval length. The world cascade starts four probes outside the floor on each side.

## How the ranges merge

At a point, walk the directions of the nearest probes.

- If the near interval hit a surface, use that color.
- If the near interval missed, use the far interval in that direction.
- If the far interval missed, use the world probe in that direction.

The merged colors are blended between the four surrounding near probes. A lookup is not a copy of the single nearest square. The floor mesh asks for a color at each vertex, and the rasterizer blends those vertices.

The picture gather places one probe on each 16 by 16 pixel tile. A 720p frame stays at or under 80 by 45 probes. Those probes sit on a world lattice around the eye, 0.5 m apart. A turn does not move the lattice. Each probe traces 4 m. The last 0.5 m blends into the world cache. A miss reads that cache.

The world cache is a 2.5 m grid with 8 directions. A lamp change marks the cache dirty. Each frame writes one band until the cache is current. A still camera with a clean cache submits no world work. The pixel reads the screen probes. A point outside that grid reads the world cache.

The blend drops a probe behind the shaded face. It also drops a probe whose hit is much closer than the shaded point. An upward face reads probes outside its footprint. A probe inside that footprint stores no light.

The field rebuilds when the grid size changes. A lamp change or an occluder change rebuilds it too. A turn does not rebuild it. The lattice stays on the eye that built it. A still camera keeps the field. Screen rectangles do not rebuild it. A hidden object that does not affect light is not part of that gather. A ray that starts inside a wall or a solid does not leave through the far side. That exit was painting the lit face onto the back face.

## Material color

Nothing is added in the dark. A point with no lamp and no bounced light is black.

A lamp reaches a point only when the straight ray misses every wall and solid. The ray uses those shapes. It is not a physics collision step. A wall taller than the ray stops the lamp. A ray that clears the top of a wall still arrives. Bounce rays already stop on the first surface in their interval.

Each face is lit from a point just outside that face, and only when that face points toward the lamp. A face that only grazes the lamp gets less of that lamp. Light from a lamp about one meter away stays below a flat white. Light a few meters from the lamp is dimmer than light next to the lamp. A unit white lamp about 7 m above a white floor stays bright enough to read. A floor point inside an object's footprint gets no lamp. A point on the lamp side, outside that footprint, stays lit. The shadow edge is the lamp ray that hits a wall or a solid. A sample at the center of a wall is inside the volume. That sample does not light the back face. A probe inside that volume stores no light. A shaded point does not read a probe across a wall.

The pixel lamp ray is the only direct term. The probe field stores the light that leaves a hit toward other surfaces. The gather the pixel reads is the screen probe grid. A wide wall stores the lamp that leaves the hit. A small solid also stores the screen gather. The lamp on the shaded point stays in the pixel ray. The world cache fills a miss. Light that leaves a surface is the arriving irradiance times the surface color and one reflectance. A material can set a reflectance from 0 to 1. A missing reflectance is `1 / π`. A material can set how much of its color mixes into the bounce. A missing mix uses the full surface color. A lamp above an object colors the floor on every side. A lamp on one side does not color the far side. A lit wall adds its bounce to the floor in front of that wall through the interval merge. The floor behind that wall stays dark.

A red solid in a white lamp throws red light. A white floor shows that red next to the solid, on top of the white the lamp puts there directly. A colored receiver multiplies the bounce by its own color as well.

The GPU writes the world cache one band at a time, then two screen passes. The first screen pass stores the lamp on each hit. The second pass stores that same lamp on a wide wall. A small solid also adds the light gathered by the first pass, so an unlit face keeps the solid color. The pixel adds its own lamp ray. A hit stores its color even when that color is black.

A miss in the last 0.5 m blends into the world cache. A new build does not read the previous frame. The picture keeps the last finished field until that gather completes. The CPU uploads positions, colors, texture ids, and lamp parameters. It does not compute the lit color. The direct shadow stays the lamp ray.

A normal draw does not stall the game loop on that GPU work. A readback waits. Move or recolor an object and the next build follows the new scene.

## World probes

A world probe is the farthest cascade, not a camera probe and not a baked lightmap. Its grid is larger than the floor, so probes sit outside the floor as well as on it. Its rays start where the far interval ends. They carry the color of whatever material they hit, including a solid the nearer ranges cannot reach.

An empty nearer ray is the only place a world probe enters the merged field. A near hit is not replaced by a far probe.

## What is fixed, and what comes from the scene

The screen grid keeps a fixed probe count. The probes are a world lattice, so a turn does not move them. A miss stores radiance 0 and β = 1. A hit stores the outgoing light and β = 0. The merge is `L + β L_next`. A screen miss then reads the coarse world grid in that direction, so a bright lamp behind the camera still has a direction. The builder reads the camera for the screen grid. It does not read the example solids or the example light.

The CPU field used by the older checks still stores fewer directions in the near range than in the far range, and fewer in the far range than in the world range.

The example scene in `examples/camera/scene.rhai` is one scene. It is not the layout of the field.

## Budget

Four figures from the paper are the upper end: intervals that double, ray counts that jump by four, and a last cascade with hundreds of directions. That last jump is more than this ground-plane march needs.

The balance used here is three cascades.

- Cascade 0 covers the floor with a cell of about 0.28 m, 16 directions, and an interval of one cell. Spacing, direction count, and interval length double at each next cascade.
- The world cascade covers the floor plus four probes of margin on each side. An on-screen miss reads that cascade.
- Each probe stores the interval, not an average of its directions. The fragment shader merges the directions with the face normal, then multiplies by the albedo and `1 / π`.
- A lamp uses cosine over inverse-square falloff. There is no fill light. Empty space is cleared to black.

## What this cut does not do

Rays march on the ground plane. A full 3D volume of probes is not built yet. The GPU field stores one interval per direction. That field is not a lightmap. A new build does not read the previous frame. The engine keeps the finished field until the screen key changes, or a lamp, an occluder, or the light-affecting medium changes. A turn does not change that key.

Emitting particles add one fire light on the ground probes. The light uses the particle positions and colors. A lit particle takes that field on its card. A density puff uses optical depth and in-scatter on the view ray. That medium is not a 3D probe grid.
