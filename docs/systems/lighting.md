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

A moved box marks two sets of world probes on the shared tier. One set is the sun-shadow shaft of that box, out to 80 m. The shaft uses the box silhouette. The other set is the probes that can see the box, inside the bounce margin.

A point lamp marks probes inside its own reach. That reach is the distance where its irradiance falls under 0.05. One small-stress lamp (peak 0.2) reaches about 17 m.

A slow sun marks probes that enter or leave a shadow shaft. It does not mark the whole tier. A camera turn does not mark probes.

Probe spacing follows how large a surface is from the camera, including a surface behind the camera. A large score stays at 1 m. Smaller scores step through 2 m, 4 m and 8 m, and the step stays inside the surface. A probe that is already finer, and whose faces stay inside the notice band, keeps that spacing. When the slot cap is full, an off-screen probe with a low score is dropped before a probe on screen. The debug view is off unless it is asked for, and asking for it does not place probes.

Lamps that sit in a small angle from a probe are one light on the CPU shade: one summed colour and one shadow ray. The same lamps next to the probe stay two lights. A sway under the lamp move limit keeps that choice. A larger move builds it again. The picture still shadow-tests the lamp list. It does not read that tree.

A sun climb is scored on the upward face, against the sun those probes last finished with. Probes outside the building take that score. Probes inside the rooms score zero. The higher score is gathered first, and it is gathered to the end of its passes before the next brick starts. When that new light differs from the picture by half or more, the picture shows it at once. It does not fade through grey. A small step still eases.

A run that is still in progress keeps its pass count. The next move waits until that run finishes. The finished run starts one new run. The picture keeps the last finished light until the run finishes.

A new face whose relative luminance changes by less than 2 percent keeps the stored face. The gather does not pull the new rays toward that face.

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

## Many lamps and occluders

Lamps and occluders sit in the tail of the scene storage buffer (8 MiB). The caps are `MAX_LAMPS` (4096) and `MAX_OCCLUDERS` (8192). A ray does not loop over every occluder. `build_grid` stores a cell only where a shape overlaps that cell. The cell stays 2 m. The box is centered on the camera and snapped to those cells.

The half-extent of the box equals the camera far plane. A change of that far plane resizes the box in the same frame. A ray jumps an empty 16 m brick. Inside a brick, a miss crosses the fine cells that repeat the shapes just tested, then the next cell is tested. A stored cell tests the analytic box slabs and the vertical cylinders. The walk stops when the hit is closer than the next cell. A ray that leaves the box misses.

A shape outside the box is absent. The floor plane and the roof plane stay single tests. The walk reads its step cap from the scene block.

Each point lamp has a range: the distance where its brightest channel on white paint falls to half a display step (`LAMP_CUTOFF`, 0.5/255). `lamp_range` gives it: about 97 m for a unit lamp and about 22 m at 0.05. A second grid of 4 m cells lists each lamp in every cell its range circle meets. A pixel visits the suns, then the lamps in its cell that reach it.

That range is a shortcut. Cost: light past the range is dropped. Over many lamps in an open area that tail can add up to a visible step; walls usually cut it first. Gain: the per-pixel lamp loop follows the lamps near the pixel, not the lamp count of the level.

A wall has a `base`. Above the floor it is a lintel, a sill, a beam or a roof slab, and the rays, the raster and the probe tier use that box. A square solid has a `yaw` about +Y. The rays turn into its frame, so a spinning box casts a turned shadow.

A sun is a directional lamp. It lights the pixel directly and lights the probe tier as a directional `TierLight`, so its bounce reaches rooms the sun does not. It has no range and is never culled.

## Sky

`Scene::sky` (`Sky { color }`) is the light from outside the scene: a ray that leaves the scene without meeting anything brings `color` back as radiance. A floor open under it takes π × `color`; the stress example's day sky (0.11, 0.14, 0.19) gives open ground about a sixth of what the noon sun gives, as on a clear day. A world probe ray (8 m) or tier probe ray (64 m) that met nothing looks on with `sky_beyond`. The occluder walk stops at the camera box, so that extra span tests only stored cells inside the box. Sky light counts in every bounce order, like the light of a lamp at a hit, and the picture takes it through the probes like any bounce. A pixel that meets nothing shows the sky (the clear colour is the sky through the tone curve). A sky change relights the tier like a sun. With no sky, a miss brings nothing, as before.

The sky is one radiance in every direction, below the horizon too. Gain: one clear colour and one extra ray per miss, and the picture and the light agree. Cost: no sun disc, no horizon glow, no gradient, and a floor that ends shows sky below it and takes that light from beneath; a scene that wants ground to the horizon gives its floor the reach.

## Limits

A pixel does not march bounce rays. The probe is the bounce, and the lamps are computed at the pixel. At each probe hit, one ray out to one probe spacing meets a nearby surface and takes that surface's light for its direction. A face stores the sum of radiance times cone weight, divided by the sum of those weights. Each pass turns the ray set, so a narrow opening is not missed by the same directions every time. On a dim face, one cosine ray adds direct light that direction of the cube does not already hold. A bright face keeps the cube. A probe just above the floor starts its steep upward rays on the floor, so that face is the floor's view of the ceiling. Shallow rays stay at the probe. The fine shell depth moves with the GPU budget, between 4 m and 48 m. When the sun or the sky disappears, every filled probe drops the old bounce, then takes one pass per frame for six bounces. The picture shows each pass at once, then holds. An open yard matches the ray trace from that field. On the last of those passes, a dark floor probe keeps about a fifth of the mean from a lit floor probe two metres away, so the ground beside a wall still sees the doorway. A sun that only turns takes six blend passes on the probes in the changed shafts. A lamp that comes back takes four replacing passes at the full ray count for the bricks in view. A lamp that goes out takes those four, then two more that repaint only a face that is still dark, at twice the ray average, so the box face meets the trace without lifting the floor. A lamp that goes out drops the old bounce on the first of those passes, and that result keeps a heavy weight so the next refine rays do not paint it bright again. A lamp that comes back drops that heavy weight, so the new direct light replaces the dark result. A high camera keeps the ground 4 m inside the tier window, so the yard does not fade to the coarse world probes. Ground on the window edge reads the world volume. When the sky is gone, that open ground is under the ray trace, and the picture multiplies the world-volume part by 1.8. While the tier has bricks, the world volume is not rebuilt for a moved box. It is rebuilt when the tier is idle, and when the sun or the sky changes. A live frame stays inside the tier time budget, including the first frame. A readback runs until the tier is finished. A brick the camera does not see takes a normal update when a lamp goes out. When that lamp comes back, the replace also runs on the bricks off screen, so the yard is the new light before the camera turns toward it. When the camera is outside the building and far enough to see all of it, a box or a lamp inside the building does not relight the probes. The sun and the sky still do.

A gap smaller than the world-probe spacing can stay dark or can stay bright for the wrong reason.

The two finest cascades are world cells at 0.5 m and 1 m. Rebound past 14 m comes from the world volume. The fade into that volume starts at 10 m. The safety caps are 2048 and 1024 probes. The paper's heaviest ray counts are not the budget. The budget is in the lighting notes.

The soft edge of a shadow is the coarser angle step of the next cascade, merged only where β is 1. There is no painted halo around a light square. The fragment shader shades each pixel. The direct shadow edge is the lamp ray. The lamp side of an object is not darkened by a margin around its base.

The per-pixel cost grows with the lamps in range: each one is a shadow ray. A dense cluster of lamps costs a ray each, wherever the level is. The world volume is at most 48 × 3 × 48 probes at 2.5 m, so it covers 120 m from the floor corner and does not follow the camera. A moving occluder marks the probes in its sun shaft and the probes that can see it. A lamp that moves 0.1 m marks probes inside its reach. A slow sun marks the shaft wedge. With boxes moving all the time those probes stay in their local runs. `examples/stress` measures these.

The picture is linear radiance through the tone curve into an 8-bit UNORM target: no exposure and no display encoding. A room lit by bounce alone sits at 5 to 15 % of a sunlit wall, so 10 to 40 of 255, and reads near black next to a sunlit patch. That is the light, not a gap in the bounce.

## Status of the local-update plan

The local dirty rule is on the shipped tier update. A moved box marks its sun shaft and the probes that can see it. A lamp marks its own reach. A slow sun marks the shaft wedge. A run finishes its passes before the next move starts one new run. A sun climb gathers outside probes first. When the new light differs from the picture by half or more, the picture snaps. It does not fade through grey.

The sparse 2 m camera box is in. ADR 0011 records it. ADR 0008 is unchanged. The picture walk is the shader in `scene_rays.glsl`. An incremental DDA, a nested cell loop, and a cached brick lookup make the NVIDIA compiler exhaust memory, so those shapes are not shipped.

Probe detail is scored by prominence, then one spacing is chosen for the whole window. It is not yet a separate 1 m, 2 m, 4 m, and 8 m lattice per surface. The debug view is off unless asked, and it does not draw spacing, age, or the skip reason.

On the small stress scene, release, 600 frames, two moving runs were 319 and 328 frames per second. One idle outside run on the same binary was 583. The eye was 12.5 m, 6 m, 48 m, looking back at the section. At a 60 degree view that distance is what frames the 25 m south face. A closer eye leaves the sides outside the view. Moving is about 55 percent of that idle run. The bar is 80 percent, and the two moving runs within 15 percent of each other. The hall raster was about 2.7 ms. A walk that does not test shapes misses and then marches the whole ray, and the raster gets slower. One shadow ray per pixel, which drops the other lamps, was still about 2.1 ms. That band is not met.

## Decisions

- [ADR 0011](../adr/0011-occluders-use-a-sparse-camera-box.md) stores occluder cells in a box centered on the camera. The cell stays 2 m. A ray jumps to the next stored cell. The floor plane and the roof plane stay single tests.
- [ADR 0008](../adr/0008-lighting-uses-radiance-cascades.md) selects radiance cascades. That ADR says the screen grid rebuilds with the camera. The two finest cascades do not. They keep a probe at the center of each world cell, drop a cell only outside 14 m, and stop at a safety cap. The farthest screen cascade still rebuilds with the camera. A lamp, an occluder, or the light-affecting medium rebuilds the field. The cell set also updates when the camera window covers a new cell or a cell leaves the 14 m window. The world volume stays in place. A new screen build does not read the previous frame. Radiance at a probe is traced again.
- [Renderer](renderer.md) owns the draw that runs this pass.
