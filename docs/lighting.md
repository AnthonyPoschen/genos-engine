# Lighting

Genos Engine lights a scene with radiance cascades. The notes here are the working description. The decision is [ADR 0008](adr/0008-lighting-uses-radiance-cascades.md).

## Sources

Read these before changing the field.

- Alexander Sannikov, *Radiance Cascades: A Novel Approach to Calculating Global Illumination*. Working paper for the game-lighting form. Repository: <https://github.com/Raikiri/RadianceCascadesPaper>
- PDF: <https://github.com/Raikiri/RadianceCascadesPaper/blob/main/out_latexmk2/RadianceCascades.pdf>
- Source: <https://github.com/Raikiri/RadianceCascadesPaper/blob/main/RadianceCascades.tex>
- Overview that points at the paper: <https://80.lv/articles/radiance-cascades-new-approach-to-calculating-global-illumination>
- Christopher M. J. Osborne and Alexander Sannikov, *Radiance Cascades: A Novel High-Resolution Formal Solution for Multidimensional Non-LTE Radiative Transfer*. arXiv:2408.14425. <https://arxiv.org/abs/2408.14425>

Sannikov shipped a screen-space form of this in Path of Exile 2. The picture uses that screen grid and rebuilds it with the camera. A 3D world volume keeps light from outside the view. A screen ray that misses reads that volume in the same direction.

## What a radiance cascade is

A radiance field is the light at each point, in each direction. A cascade is one distance range of that field.

A radiance interval `L(a, b)` at a point, along a ray, is the light that leaves the first surface the ray hits between distance `a` and distance `b`. A miss stores no light and keeps the next range. The ranges meet: the next cascade starts where the previous one ends.

The penumbra hypothesis is the reason the ranges are stored differently. Close to a surface you need many probe positions and only a few directions. Far from a surface you need few positions and many directions. A near cascade therefore has a smaller spacing and fewer directions than a far cascade. A world cascade is farther again, so it is coarser in space and finer in angle.

A probe stores one radiance interval per direction, and the transparency β of that interval. The picture cascades sit on the screen. Cascade 0 has the most probes and the fewest directions. Each next cascade has fewer probes, more directions, and a longer interval. The world probes sit in a 3D volume around the floor. They do not follow the camera.

## How the ranges merge

At a point, walk the directions of the nearest probes.

- If the near interval hit a surface, use that color.
- If the near interval missed, use the far interval in that direction.
- If the far interval missed, use the world probe in that direction.

The merged colors are blended between the four surrounding near probes. A lookup is not a copy of the single nearest square. The fragment shader reads that blend at the shaded point.

The picture gather builds three screen cascades. The near grid is the finest. A near miss keeps the farther screen range in that direction, then the 3D world probe. The pixel reads the merged near cascade. The screen field follows the camera. The world volume stays on the floor.

The world volume is a 2.5 m grid in X and Z, with three heights. Each probe stores 16 directions. A hit stores the light that leaves that surface. The next world pass adds one bounce from the volume. A screen miss reads that bounced volume. A hit does not.

The blend drops a probe behind the shaded face. It also drops a probe whose hit is much closer than the shaded point. An upward face reads probes outside its footprint. A probe inside that footprint stores no light.

The screen field rebuilds with the camera. A lamp change or an occluder change rebuilds it too. A turn does not rebuild it. Screen rectangles do not rebuild it. A hidden object that does not affect light is not part of that gather. A ray that starts inside a wall or a solid does not leave through the far side. That exit was painting the lit face onto the back face.

## Material color

Nothing is added in the dark. A point with no lamp and no bounced light is black.

A lamp reaches a point only when the straight ray misses every wall and solid. The ray uses those shapes. It is not a physics collision step. A wall taller than the ray stops the lamp. A ray that clears the top of a wall still arrives. Bounce rays already stop on the first surface in their interval.

Each face is lit from a point just outside that face, and only when that face points toward the lamp. A face that only grazes the lamp gets less of that lamp. Light from a lamp about one meter away stays below a flat white. Light a few meters from the lamp is dimmer than light next to the lamp. A unit white lamp about 7 m above a white floor stays bright enough to read. A floor point inside an object's footprint gets no lamp. A point on the lamp side, outside that footprint, stays lit. The shadow edge is the lamp ray that hits a wall or a solid. A sample at the center of a wall is inside the volume. That sample does not light the back face. A probe inside that volume stores no light. A shaded point does not read a probe across a wall.

The pixel lamp ray is the only direct term. The probe field stores the light that leaves a hit toward other surfaces. The pixel reads the merged near cascade. A near miss keeps the farther cascade. The lamp on the shaded point stays in the pixel ray. Light that leaves a surface is the arriving irradiance times the surface color and one reflectance. A material can set a reflectance from 0 to 1. A missing reflectance is `1 / π`. A material can set how much of its color mixes into the bounce. A missing mix uses the full surface color. A lamp above an object tints the floor around it. The shadow under the object stays dark. A lamp on one side does not color the far side. A lit wall adds its bounce to the floor in front of that wall through the interval merge. The floor behind that wall stays dark.

A red solid in a white lamp throws red light. A white floor shows that red next to the solid, on top of the white the lamp puts there directly. A colored receiver multiplies the bounce by its own color as well.

The GPU writes the 3D world volume, adds one bounce, then writes the screen cascades from far to near. A hit stores the light that leaves that surface. A miss keeps the next range. The pixel adds its own lamp ray. A hit stores its color even when that color is black.

A screen miss reads the world volume in that direction. A new screen build does not read the previous frame. The picture keeps the last finished field until that gather completes. The CPU uploads positions, colors, texture ids, and lamp parameters. It does not compute the lit color. The direct shadow stays the lamp ray.

A normal draw does not stall the game loop on that GPU work. A readback waits. Move or recolor an object and the next build follows the new scene.

## World probes

A world probe is a coarse 3D sample, not a camera probe and not a baked lightmap. The volume is larger than the floor, so probes sit outside the floor as well as above it. A ray from a world probe carries the color of the first surface it hits. That includes a lamp and a bounce the screen grid cannot see.

An empty screen ray is the only place a world probe enters the merged field. A screen hit is not replaced by a world probe.

## What is fixed, and what comes from the scene

The screen probes move with the camera. The world probes stay put. A miss stores radiance 0 and β = 1. A hit stores the outgoing light and β = 0. The merge is `L + β L_next` for each direction. A screen miss then reads the 3D world volume in that direction, so a bright lamp behind the camera still has a direction. The builder reads the camera for the screen grid. It does not read the example solids or the example light.

The CPU field used by the older checks still stores fewer directions in the near range than in the far range, and fewer in the far range than in the world range.

The example scene in `examples/camera/scene.rhai` is one scene. It is not the layout of the field.

## Budget

Four figures from the paper are the upper end: intervals that double, ray counts that jump by four, and a last cascade with hundreds of directions. That last jump is more than this screen gather needs.

The balance used here is three screen cascades plus one 3D world volume.

- Cascade 0 covers the screen with one probe per 8 pixels, 64 directions, and an interval of 0.55 m. The cap is 96 by 54 probes. Each next cascade halves the probe count, starts from 32 directions, and lengthens the interval.
- The world volume covers the floor plus two cells of margin on each side, at three heights. An on-screen miss reads that volume.
- Farther probes store one interval per direction. The near probe stores the merged irradiance. The screen ray fan is fixed in the surface frame. The fragment shader reads that irradiance, then multiplies by the albedo and `1 / π`.
- A lamp uses cosine over inverse-square falloff. There is no fill light. Empty space is cleared to black.

## What this cut does not do

Screen rays march in 3D from the surface they land on. The world probes are a coarse 3D volume. The GPU field stores one interval per direction. That field is not a lightmap. A new screen build does not read the previous frame. The world pass reads its own direct hit once, to add one bounce. The engine keeps the finished field until the next gather swaps in.

Emitting particles add one fire light on the probes. The light uses the particle positions and colors. A lit particle takes that field on its card. A density puff uses optical depth and in-scatter on the view ray. That medium is not a probe grid.
