# Surface cache: light through walls in gpu_frame

Since ba9adca0 made the surface cache (world mode 3) the default, five gpu_frame
tests fail on the 4070. With `GENOS_GI2_WORLD=0` (the probe path) they pass. This
note records what causes each failure, measured on master bf82a177.

## 1. Stale texels after a light change (a_bright_lamp, outside lamp and fire)

`a_bright_lamp_does_not_cross_an_opaque_wall` draws two scenes on one renderer.
The second scene moves the lamp from beside the west face to above the wall.
After the move, the west face reads 191.4 and the floor reads 189.6, with no
falloff. The east face reads about 0. Turning off any single SC flag does not
change those readings. They are the lit values from the first scene.

- A `reset_lighting_cache()` call before the second scene makes the test pass.
- A test build that cleared the cache whenever the light key changed passed both
  this test and `a_close_lamp_does_not_cross_the_wall`.
- Forcing `young_only` off did not help: west was still 191.4.

So the texels are not wrong. They are out of date. A light change sets
`light_quiet = 0`, and the full sweep that follows is spread over frames in
batches of `LIGHTING_CACHE_BATCH`. A `readback` draw is one frame, so it reads
the cache before the sweep reaches the face. The probe path relights inside one
frame.

That clear on a light change cannot ship. `lighting_scene_key` hashes the fire's
strength and every puff, so a fire scene changes the key every frame, and the
cache would be wiped every frame. The supported way to switch scenes is still
`reset_lighting_cache()`, which `sky.rs` already calls. A real fix needs one of
two things:
- a key for scene identity, separate from the light key;
- a sweep that completes before a `readback` frame is shot.

## 2. Light through a thin wall in a single scene (close lamp, lit_views)

These tests draw one world from the start, so stale texels cannot be the cause.

- `lit_views`, lamp (-1.2,1.3,-3), x=-1.5: the base reads 61.4, over the cap of
  43.8 (8% of the lit face, 547.6). This also fails with the clear above.
- `a_close_lamp`: the back corner reads 63 on the first run and passes on a
  later run, so it sits near its cap of 45 and is flaky.

The wall is 0.4 m thick and the samples sit 0.2 m above the floor, so this looks
like a texel or footprint that spans the wall's base. Not yet found.

## Status

The leak has no fix that is both contained and safe yet, so there is no PR. The
next steps:
1. Split the scene-identity key from the light key and clear on an identity change.
2. Find the base leak with `GENOS_LIGHTING_SC_TRACE` on lit_views lamp 1.
