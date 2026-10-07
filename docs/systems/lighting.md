# Lighting

The renderer lighting engine composes illumination. `Renderer::draw` passes the lights and the objects. The engine decides which lamp reaches which surface. A game does not pair a lamp with an object first.

## Goal

A player can see one lamp, a shadow where a wall blocks that lamp, and a bounce that takes the color of the hit material. A dark scene with no lamp and no bounce is black.

## Intent

The technique is radiance cascades. The working notes, the paper links, and the budget are in [Lighting notes](../lighting.md). Read that file before you change probe counts, interval length, or the merge rule.

A screen build does not read the previous frame. There is no lightmap and no ambient fill. The gather writes the 3D world volume, adds one bounce, then writes three screen cascades from far to near. A hit stores the light that leaves that surface. The pixel reads the finest cascade and adds its own lamp ray. A screen miss reads the world volume.

A miss merges with the next cascade in that direction. An average of nearby probes does not stand in for a bounce the gather does not run.

A direct ray stops when it hits a wall or a solid. The ray uses those shapes. It is not a physics collision step. A ray that clears the top of a wall still arrives. A bounce ray stops on the first surface in its interval.

Each face is shaded from a point just outside that face. The back of a wall stays dark when the lamp is on the other side. A brighter lamp does not pass through that wall. A lamp above an object tints the floor around it. The shadow under the object stays dark. The vertical faces of that object stay unlit by that lamp. A lamp on one side does not color the far side.

A lit wall adds its bounce to the floor in front of that wall through the interval merge. The floor behind that wall stays dark.

Hidden objects and objects outside the view stay in the field when `affects_light` is true. Culling removes them from the picture only.

Screen rays march in 3D from the hit surface. World probes sit in a volume past the screen ranges. They carry material color that the screen rays miss, including a bounce from off screen.

The two finest cascades sit on world cells. Cascade 0 is 0.5 m, with an interval of 0.55 m and 64 directions. The fan does not rotate per probe. Cascade 1 is 1 m, with more directions and a longer interval. A probe sits at the center of its floor or wall cell. Fine cells are filled inside 8 m and 1 m cells inside 12 m. A cell is dropped only after it leaves 14 m. The picture fades from the 0.5 m field to the 1 m field between 6 m and 12 m, then into the world volume by 14 m. Each solid has a face lattice on cascade 0 that follows the solid. The farthest screen cascade is cast again with the camera, at one probe per 32 pixels, with a cap of 96 by 54 on the finest screen grid. A miss reads the next cascade, then the 3D world volume. That volume keeps a direction for light behind the camera. The world volume stays on its grid.

The floor, the walls, and the solids read that same field. An upward face reads probes outside its footprint. A later draw reuses the field until a lamp moves by about a meter, or a lamp color, an occluder, or the light-affecting medium changes. Screen rectangles do not rebuild it. An object that is hidden and does not affect light is left out of the gather. The paper links, including arXiv:2408.14425, are in the lighting notes.

A camera move does not rebuild the visible mesh. The draw rebuilds that mesh when the shaded scene changes or the on-screen set changes.

## Code

The GPU compute shader `shaders/light.comp` builds the field. The fragment shader `shaders/scene.frag` shades each pixel. `Renderer::draw` does not bake the lit color on the CPU.

Packed object ranges, colors, and texture ids live in one GPU buffer. An object with no texture stores texture id 0. A later frame does not upload that buffer again when the mesh is unchanged.

The picture is three screen-space radiance cascades. The near grid is the finest. Each farther grid is coarser, with more directions and a longer interval. A miss keeps the farther screen range, then the 3D world probes. The world probes sit in a volume, not on the floor. They carry light the screen rays do not hit, including light from off screen. A readback finishes that gather before the picture.

The world volume covers the floor at 2.5 m in X and Z, on three heights, with 16 directions. Each gather writes that volume, then the screen cascades. A screen miss reads the volume, including light behind the camera.

A probe inside a wall or a solid stores a distance below zero and is left out of the blend. A hit stores the distance to that hit. The blend skips a probe behind the shaded face. It also skips a probe whose hit is much closer than the shaded point. The blend skips a probe when the segment to it crosses a wall.

## Game use

In the camera example, Control+2 draws the finest lattice and Control+3 the next lattice. Each probe draws a short tick along its normal. A probe outside the view is dimmer. A few probes also draw their interval rays. Control+4 draws the 3D world volume. Place a point lamp with `light(x, y, z, r, g, b)`. Place a directional light with `sun(x, y, z, r, g, b)`. The sun vector is the direction the rays travel. `sun(0, -1, 1, 1, 1, 1)` points down at 45 degrees toward +Z. A color of 1, 1, 1 is the unit lamp. The sun uses the brightness of that lamp at 7 m. Do not call the field builder from the game loop.

A new occluder is a wall or a solid in the scene. The direct ray and the bounce ray both see it. A mesh or a particle with `affects_light` becomes a stand-in solid in `World::light_scene`.

Fire does not use that stand-in. The stationary flame is one lamp in the same cascade. A wall blocks that lamp. The flame does not light the far side of the wall. The floor beside the flame stays warm with the scene lamp off. A lit particle uses the same shade path as a face: albedo times reflectance over π times direct plus bounce. A density puff is still optical depth and in-scatter on the view ray. Light that leaves a surface travels in the cascade until the next surface.

## Limits

The world probes are a coarse 3D volume. A gap smaller than that spacing can stay dark or can stay bright for the wrong reason.

The two finest cascades are world cells at 0.5 m and 1 m. Rebound past 14 m comes from the world volume. The fade into that volume starts at 10 m. The safety caps are 2048 and 1024 probes. The paper's heaviest ray counts are not the budget. The budget is in the lighting notes.

The soft edge of a shadow is the coarser angle step of the next cascade, merged only where β is 1. There is no painted halo around a light square. The fragment shader shades each pixel. The direct shadow edge is the lamp ray. The lamp side of an object is not darkened by a margin around its base.

## Decisions

- [ADR 0008](../adr/0008-lighting-uses-radiance-cascades.md) selects radiance cascades. That ADR says the screen grid rebuilds with the camera. The two finest cascades do not. They keep a probe at the center of each world cell, drop a cell only outside 14 m, and stop at a safety cap. The farthest screen cascade still rebuilds with the camera. A lamp, an occluder, or the light-affecting medium rebuilds the field. The cell set also updates when the camera window covers a new cell or a cell leaves the 14 m window. The world volume stays in place. A new screen build does not read the previous frame. Radiance at a probe is traced again.
- [Renderer](renderer.md) owns the draw that runs this pass.
