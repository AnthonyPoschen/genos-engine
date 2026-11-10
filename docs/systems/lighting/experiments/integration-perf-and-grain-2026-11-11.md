# Integration perf and the static-hall speckles

- **Date:** 2026-11-11
- **Result:** finding. The default High picture is unchanged. `coverage_first` and `cap_fill` default off; they exist so the pictures below could be shot.
- **Heads:** `origin/lighting-v3` `34cc80cb`, `cursor/lighting-integration` `30c47bb1`. The sample-count rule is #255 (`6a057068`), kept. The texels-view population record used for the traces is `f710e4cf` on that branch.
- **Machine for the pictures:** lavapipe, 640×360, Weston headless. Absolute milliseconds are ratios only. Anthony's RTX 4070 at 1440p is the absolute clock.

## What Anthony measured

Walking the hall at 1440p: `origin/lighting-v3` about 70 fps (14.3 ms), `cursor/lighting-integration` about 55 fps (18.2 ms). Integration adds about 3.9 ms while the camera moves. Both are far from 2 ms. With boxes and lights held still, dark pixels settle into a fixed pattern. The rel<0.55 count on a static hall (13×13 local mean, 6 px border) went 1452 on integration before #255 to 1501 after it.

## Per-pass diff, v3 against integration

High on v3 (`crates/scene/src/lighting.rs` `high_defaults`, `probes.enabled = true` at line 850, `denoiser.screen = false` at line 861, five iterations). A moving cache round (`lighting_dispatch.rs` 432–456) dispatches place, gather, trace `(probe_ray_count + 2*kept)/64`, direct `(pixels + 2*kept)/64`, gather again, filter ×3, cache, the hash wavelet, and compose. No maintenance compose.

High on integration (`probes.enabled = false`, `denoiser.screen = true`). The same round (`lighting_dispatch.rs` 445–458) dispatches compact over 1,048,576 slots, compact args, cache rays, direct `pixels/64`, hit, cache, the screen wavelet, then `push_picture` (386–390): maintenance pass 9 (`SH_PREP_PASS`, billed `B_CACHE`) and compose pass 7. Place, gather, and filter are not dispatched. ReSTIR DI is two `pixels/64` dispatches before the group loop, added into `B_DIRECT`. v3 High picks the lamp in the shade shader and has no separate ReSTIR passes.

`LIGHTING_MS` sums `times.lighting`, which is `[f64; 9]` (`gpu.rs` 9739). A pass id of 9 fails `pass < times.lighting.len()` (`gpu.rs` 4017) and is dropped from that total. Maintenance is on the `LIGHTING_DISPATCH` line only. The `shadows` bucket repeats direct plus hit.

1440p is 2560×1440 = 3,686,400 pixels, tile 16, so a picture dispatch is 320×180 = 57,600 groups. v3 probes are 160×90 = 14,400, 64 rays, `probe_ray_count` 921,600, strata kept 7,372,800. v3 trace groups `(921600 + 2*7372800)/64` = 244,800. v3 direct groups `(3686400 + 14745600)/64` = 288,000. Integration direct groups are 57,600. Compact is `1048576/64` = 16,384. v3's hash wavelet is that slot grid times the stages. Integration's screen wavelet is eight dispatches of the picture grid (`push_denoise`, 487–514): about four times the groups, and each group is a pixel neighbourhood.

### Lavapipe ratios, integration High, last steady window

Do not scale these milliseconds onto the 4070. They say which pass is the large one.

| Scene | Total | Atrous | Compact | Light (ReSTIR in direct) | Cache pass 6 | Cache rays | Compose pass 7 | Maint pass 9 (not in the total) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Hall, still | 1047 | 739 (70%) | 138 (13%) | 126 | 19 | 25 | 0.25 | 64 |
| Hall, yaw | 1364 | 870 | 159 | ~109 shade | 173 | (in the trace bucket) | 0.26 | 56 |
| Corner, still | 100 | 56 | 18 | 7.6 | 18 | 1 | 0.05 | 0.16 |
| Corner, lamp stops | 101 | (same shape) | | | 16 | |  | 0.20 |

The walking delta on lavapipe is the cache integrate (pass 6, 19 → 173) plus a fatter wavelet. Picture compose stays under 0.3 and the maintenance pass stays flat. v3 has no lavapipe timestamps in this note. The structural diff names the passes v3 still has and the passes integration dropped.

### Where the 14–18 ms goes, and the 3.9 ms

Anthony's 14.3 ms and 18.2 ms stay the absolute numbers. The lavapipe split says the steady hall is mostly the screen wavelet, then compact, then direct (ReSTIR shade is most of `light_px` and did not jump while yawing), then maintenance, which the `LIGHTING_MS` total hides. The extra cost while walking is the cache update growing when more cells are due, on top of the wavelet. The expected speedup from deleting the screen probes did not show up because the replacement is larger than the deleted work: maintenance over the picture, ReSTIR as its own dispatches, the screen wavelet instead of the hash wavelet, the same 1,048,576-slot compact, and a heavier cache integrate while the camera moves.

Top three costs, with the line that launches them:

1. Screen wavelet, eight full-picture dispatches. About 70% of steady hall lighting on lavapipe. `push_denoise` in `crates/render/src/lighting_dispatch.rs` 487–514. v3 High uses the hash wavelet (`denoiser.screen = false`).
2. Compact walks 1,048,576 slots every cache round. About 13% of the hall and 18% of the corner. `lighting_dispatch.rs` 451.
3. While the camera moves, cache integrate (pass 6, `lighting_dispatch.rs` 456) grows about ten times on lavapipe (19 → 173). Added against v3 and flat while still or moving: maintenance pass 9 (`push_picture` 386–390), about 64 ms of the hall on lavapipe, omitted from `LIGHTING_MS`.

Levers toward 2 ms, not done here. Fewer wavelet iterations, or only the blocks that changed. Do not walk a million slots every frame. One compose, not a second full-picture maintenance. Do not hash-walk every pixel. Scale ReSTIR down, or skip the spatial pass while the camera moves. The direct shadow ray per pixel is the likely large trace on a 4070. `docs/systems/lighting/research/path-to-200fps.md` is an older 1080p split and is not this budget.

## Static hall to convergence

640×360, High, boxes, lamps, and the sun still, camera (14.5, 1.7, 23.5), yaw −0.3554, pitch 0.08. Binary is #255 plus the population record. Marks at frames 40, 80, 160, 320, 480, 640.

Picture mean luminance: 0.271 at frame 40, 0.270 at frame 160, and it does not move after that (0.27026 at frame 640). Bounce residual standard deviation on the floor (13×13, the red box excluded): 8.80 at frame 40, 4.06 at frame 160, 4.07 at frame 640. Between frame 320 and 640 the bounce view changes by at most 1 code. The indirect term has stopped.

`lightingsc` while the picture was still filling: about 9,500–14,000 sets per frame and about 1.0–1.4 million rays, against about 60,000–90,000 live cells. Around lighting frame 210–250 the scheduler goes idle: sets fall to 0, `freed` stays 0, and `waiting` sits near 28,000. The slots are still allocated. They are past their interval.

### Set counts

Pixels the texels view actually wrote (the view differs from the picture), 227,696 of them:

| Frame | Mean sets | At 0 | At 64 |
|---|---:|---:|---:|
| 40 | 17.6 | 3.5% | 0 |
| 80 | 32.7 | | 0 |
| 160 | 55.7 | 8,012 | 0 (the mass is at 56) |
| 320, 480, 640 | 61.8 | 8,012 | 219,684 |

From frame 320 on, the histogram is identical. 96.5% of written pixels are at exactly 64 sets. The other 8,012 stay at 0 because the probe looks up the pixel centre and the bilinear corners are the keys compose claims (`lighting_cache_find_sized` in `lighting_cache.glsl` 920). A centre that is never a corner is a miss even while the pixel is shaded. Own-mean luma on the written pixels stays about 122/127 (the 7-bit probe saturates near 1). The inside-geometry bit is 0 on every written pixel. The seed byte is 0. The coarse bit is 0: the level is the 12.5 cm texel.

`lighting_under_trusted` reads 219,684 at the frame-40, 80, and 160 marks, and 0 at 320, 480, and 640. A fully reused frame clears the stats and does not walk the pixels, so the 0 is an idle compose. The texels view is the count that holds: the claimed texels are at 64.

### The rel<0.55 count

| Shot | rel<0.55 | of which are not the sky colour | sky pixels `(28, 36, 48)` |
|---|---:|---:|---:|
| Integration, frame 80, before #255 | 1452 | 384 | 2708 |
| #255, frame 80 | 1501 | 446 | 2708 |
| #255, frame 40 | 1815 | 310 | 2704 |
| #255, frame 80 | 1753 | 333 | 2704 |
| #255, frame 160 | 1736 | 292 | 2704 |
| #255, frame 640 | 1735 | 298 | 2704 |

`(28, 36, 48)` is `building::SKY_COLOR` `[0.11, 0.14, 0.19]` through `pack_rgb`. At the default time 0.12 the sky factor is already clamped to 1 (`building.rs` 75–79, 556–561), so the noon sky packs as those three bytes. 2,704 of them sit in a fixed band, x 92–466, y 158–216, the skylight and the window openings. They are the same bytes in the full, texels, path, and bounce views, from frame 40 through 640, and in the shots from before #255. The 4 spp path trace at the same screen point is the same triple. Compose does not write a pixel with no gbuffer surface (`lighting_compose.comp` 91–98, the comment at line 12: keep the raster colour). That colour is the sky. The interior of an opening matches its own local mean, so the metric only fires on the edge, where the lit wall pulls the 13×13 mean up. That edge is about 1,437 of the 1,735 rel<0.55 pixels at frame 640. #255 cannot move it.

Of the other 298 at frame 640: 254 have a centre-probe set count of 0, and 44 are at 64 sets with an own-mean byte of 127 (the cache is bright; the picture is just under 0.55 of its neighbourhood). 207 of the 298 are below y 250. They form clumps, not a salt noise. The largest is 108 pixels at x 246–253, y 282–348, picture about `(133, 20, 11)`. The next is 61 pixels at x 463–470, y 230–243, picture about `(9, 48, 11)`.

### Three pixels at frame 640

1. `(360, 165)`, in the opening band. Picture `(28, 36, 48)` from frame 40 on, and the same in the path trace. No cell, no sets, no seed, no rays: `lighting_surface` is false, so compose returns before the cache read. The value is the sky. Trust does not apply.

2. `(468, 230)`, the dark green clump. Picture `(8, 48, 11)`, rel 0.33 against a local mean of 112. Centre probe `(0, 0, 0)`: that key was never claimed. Path view `(255, 0, 0)`, which is shade code 8. Shade 8 is the unset code the ordinary `lighting_sc_shown` arm leaves in place (`lighting_cache.glsl` 4012 and 4332); it means the corner displayed through the normal shown path, not the empty-corner fallback (that fallback writes shade 4 or 7 at 4305). The green channel of the path view is 0 on every pixel, because `lighting_screen_probe_indirect` sets `lighting_px_probe_skipped = 1` on entry (`lighting_screen_probe.glsl` 94). The path trace at the mapped pixel is `(2, 12, 3)`, the same dark edge. This is the green box in shadow. The centre key has no slot.

3. `(486, 186)`. Picture `(51, 52, 49)`, rel 0.54 against a local mean of 95. Centre probe `(127, 0, 64)`: 64 own sets, own luma saturated, seed 0, ray origin not inside geometry, 12.5 cm cell. Trust is 1. The shown value is the filtered field (`lighting_sc_shown` returns `vec4(field, 1)` when the field has support, `lighting_cache.glsl` 2825). That field sits a few codes under the neighbourhood, on a texel whose own mean is saturated.

The inside-geometry bit is 0 on every written texel. The cache ray origin (`lighting_ray` steps 2 cm along the normal; `lamps_light` returns 0 from an origin `scene_inside` accepts) is outside the solid on the texels this view wrote.

### Can the ray budget reach 64?

At frame 40, yes, about 95% of the picture is under 64 sets (under-trusted 219,684 of 230,400; written-pixel mean 17.6). The batch does close that gap. About 9,500–14,000 sets per frame against 60,000–90,000 live cells puts the bulk on 64 by frame 160–320, which is what the histogram shows, and the scheduler then idles. The full-sets bar is reachable for a texel compose claimed. The 8,012 centre misses are keys the bilinear never claimed. After the idle, `waiting` near 28,000 with `sets` at 0 is that remainder sitting on its interval. The batch itself still has room.

The 4 spp hall compare finished in 0.91 s with `live_mean` 0 and `noise` 0.25, so it cannot stand as a per-pixel reference. The reference PNG is enough to check that a screen point is sky or a dark surface, which is how the three pixels above use it. A full trace is the owner's command (rubric row 11). Downsampled to 160×90, the frame-640 picture's floor residual (5×5) is 8.6. The 4 spp reference's floor residual is 34.

What is still moving after the cache stops: the full picture, frame 320 against frame 640, changes on 139,803 pixels, 16,969 of them by 20 codes or more, max 71. The bounce view's max change is 1. The indirect term is frozen. The direct term is not. Two colours swap on those pixels, for example `(164, 147, 128)` against `(193, 192, 191)` near the top of the frame, and a similar pair on the floor. That is the grain a still screenshot can look like it has settled, while the next hundred frames still move the direct light.

## What #255 fixed, and what it did not

#255 is the right rule and stays. A seed is not samples. Resolved, the perceptual stop, the ray-cut hold, and the empty-corner test key off the own-set count, and trust below 64 is `sets/64`. The claimed texels do reach 64, and their own mean stays bright while they get there, so they were not freezing at a black zero.

The rel<0.55 count did not fall because about four fifths of it is the sky opening's edge, and the rest is geometry the path trace also shades dark. Neither is a texel that converged to black.

## Candidate fixes

Each is one sentence. This note does not implement them. No blur.

1. Stop scoring the noon sky and the path-traced shadow edge as stuck cache texels; the rel<0.55 count will not move until the metric ignores `SKY_COLOR`.
2. The direct term is still changing by tens of codes after the bounce has frozen (frame 320 against 640), so the remaining grain is the direct shade and its shadow ray, not the world cache.
3. The screen wavelet is about 70% of steady hall lighting on lavapipe (`push_denoise`, `lighting_dispatch.rs` 487–514); fewer iterations, or only dirty blocks, is the largest cut toward 2 ms.
4. Compact walks 1,048,576 slots every cache round (`lighting_dispatch.rs` 451); a still scene does not need that walk once the slots are settled.
5. Maintenance pass 9 is a second full-picture compose (`push_picture`, 386–390) and it is missing from `LIGHTING_MS` (`gpu.rs` 4017); one compose covers the picture.
6. Keep the #255 count rule. It is what stops a zero-mean texel from looking resolved, and the histogram shows the claimed set does arrive at 64.

## Set cap, steady cost, and coverage-first

Anthony's cutoff: about 10% less detail is acceptable if the picture still looks good and the cost is about half. These shots are the integration scheduler (this branch), not #255. `cap_fill` (default off) makes `world_cap` the settle point: the 8-set perceptual stop does not fire, and a texel at the cap goes to the trickle list instead of taking a new whole set. `world_cap` 0 with `cap_fill` off is still 64 sets and the perceptual stop, which is High. The pictures use `cap_fill` on.

Static hall, Anthony's pose (eye 14.5, 1.7, 23.5, yaw −0.3554, pitch 0.08), 640×360, High, boxes still, lamps still, sun frozen, dynamic 0. The shot is after 200 frames (240 for the 64-set cap), past the lamp ring. Through that ring the cache sits near 613k rays, then spikes to the 1.40M batch ceiling, and only then falls to the steady count below. Lavapipe. Ray counts are `lightingsc` `rays`. The lighting-total milliseconds are `GPU_MS` `lighting=`. The per-pass `LIGHTING_MS` line printed only on the opening frame, so it is not the steady split.

The path trace of that pose is `img/hall-path-reference.png`: 640×360, 72 spp, 254 s, noise 0.138. It did not reach the 0.06 noise target. Error below is near-weighted and unweighted relative luminance against that PNG, sky pixels `(28, 36, 48)` masked. Near weight is the depth view (white is the eye, black is 50 m). The reference noise is large enough that caps 8 through 64 land on the same error. The difference from the 64-set picture is the number that still separates them.

| Cap | Cache rays | Lighting ms | Near error | Flat error | Error vs the 64-set error | Near difference from the 64-set picture | Frame-to-frame luma |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 4 | 28,517 | 376 | 0.0995 | 0.1027 | +19% | 0.082 | 3.35 |
| 8 | 28,008 | 351 | 0.0833 | 0.0858 | 0% | 0.031 | 2.95 |
| 16 | 26,600 | 605 | 0.0837 | 0.0862 | 0% | 0.026 | 2.92 |
| 32 | 1,574 | 551 | 0.0837 | 0.0862 | 0% | 0.026 | 2.90 |
| 64 | 104 | 595 | 0.0835 | 0.0860 | baseline | 0 | 2.87 |

Steady cache rays fall as the cap rises. A 4-set cap is about 274× the 64-set ray count, an 8-set cap about 269×, 16 about 256×, 32 about 15×. The 64-set texel is the cheap one: once it is full it leaves the trickle list (about 100 rays a frame, plus a 4,096-ray scout every eighth frame). A texel parked at 4, 8, or 16 sets stays noisy, so it stays due, and the trickle list stays full at about 27–28k rays. The lighting-total timestamp does not follow that. It sits between 350 and 600 ms at every cap, because on lavapipe the wavelet and compact dominate the clock. Do not read the 376 ms at 4 sets as a saving.

Cap 4 is the one that looks worse: the back wall and the pillar are blotchy, and the near-weighted error is 19% above the 64-set picture's error. Caps 8, 16, and 32 are inside the reference noise of the 64-set picture. Against that picture itself they still differ (3.1% of the near-weighted light at 8 sets, 2.6% at 16 and 32). The floor speckle is in every cap, including 64, at about 3 codes of frame-to-frame luma. That is the direct term, which this sweep does not touch. No cap pops. Cap 4 blotches.

No cap meets the 10%-for-half-cost line. The caps that look close to 64 cost more rays in the steady hall, not fewer. Leave the 64-set bar. Leave `LIGHTING_DN_MIN_SETS` at 8: a floor of 4 is the blotchy picture, and it does not save rays.

Full frames: `img/hall-cap-4.png`, `img/hall-cap-8.png`, `img/hall-cap-16.png`, `img/hall-cap-32.png`, `img/hall-cap-64.png`, `img/hall-path-reference.png`.

Contact sheets, 2× nearest, one region each. The label is the cap and the steady ray count against the 64-set baseline.

- `img/hall-sets-floor.png`
- `img/hall-sets-back-wall.png`
- `img/hall-sets-pillar.png`

### Coverage-first, one yaw frame

`coverage_first` (default off) waits until the near-camera-weighted mean of on-screen fine texels reaches 16. While any on-screen fine texel is under 8, those texels are listed ahead of refinement and each takes one 16-ray quarter, not a whole 64-ray set. Refinement of texels already at 8 waits.

Same hall, 80 frames still, then a yaw of 0.03 rad per frame for 40 frames. Both schedulers sit on the batch ceiling, 1,404,928 rays, through the turn. The peak is not cut, and the lighting total stays near 1.0 s on both. The saved frame is the end of that yaw. Current (frame 181) is still at 1,404,928 rays. Coverage-first (frame 177) has fallen to 69,248 rays, 95% fewer on that frame. The floor of that frame is still speckled on both.

- `img/hall-yaw-current.png`
- `img/hall-yaw-coverage-first.png`
- `img/hall-yaw-floor.png`
- `img/hall-yaw-back-wall.png`
- `img/hall-yaw-pillar.png`

The yaw sheets are that turned frame. The camera has moved about 1.2 rad, so the near floor, the far wall, and the pillar are not the same pixels as the still sheets.

Leave `coverage_first` off. It shortens the tail after a turn. It does not stop the frame-time spike while the new texels are arriving. A moving lamp plus moving boxes was not part of this shoot.

The two flags are local. Nothing is replicated. The client that has the flag is the authority for its own cache. There is no prediction or rollback. The scheduler is not deterministic across the flag. A headless server does not run this cache.
