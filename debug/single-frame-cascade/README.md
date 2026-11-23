# genos-single-frame-cascade

A CPU prototype of the sparse cascade traced in one frame with split rays (Freeman, Sannikov and Pitt-Francis, arXiv 2607.20384). It is not the renderer and it draws nothing on a GPU. It answers two questions for the lighting-stress building:

1. How many rays does one frame need for the cascade to converge with no temporal mean?
2. How far is that one frame from a path-traced reference?

Every cascade ray goes through `genos_render::Surfaces`, the tracer interface, and is counted there. The count holds for any tracer behind that interface: the software tracer, hardware ray queries, or a tracer that walks the probes.

## What it does

One frame, as the paper's Algorithm 1 and Algorithm 3:

1. One visible point per pixel. Its level of detail keeps a cascade-0 probe `--probe-px` pixels wide on screen.
2. The nearest probe of every cascade to every visible point goes into a hash map. Cascade `n` has probe spacing `2^n`, `2 T^2 4^n` directions (`--theta T`), and ray length `t0 4^n`.
3. `--spp` rays per pixel from the visible point, on the R2 sequence, with probes under one coarser probe taking a contiguous run.
4. Each ray is split: the cascade whose interval holds the hit takes its radiance, and every cascade under it records that the way was clear.
5. The cascades merge from the last one down, with sparse trilinear reads.
6. A pixel's bounce irradiance is the cosine-weighted sum of the cascade-0 directions around it. With `--bin-cosine mean` (the default) each direction is weighed by the cosine of the luma-weighted mean direction of the light it holds, not of its centre (see Phase 0 below).

Two modes:

- `nee`: a ray hit is shaded by every lamp that faces it, one shadow ray each, and the cascade carries one bounce. This is the paper. The shadow rays are counted apart, because they grow with the lamp count.
- `emissive`: each point lamp is a glowing sphere of `--lamp-radius`, surfaces are black, and the cascade carries the lamps' own light. No shadow ray. This is "all light is indirect".

The reference is 1024 cosine rays per compared pixel, shaded the same way as a cascade hit (`nee`), or the visible share of each sphere's disc (`emissive`).

## Arguments

`genos-single-frame-cascade --help` lists them. It reads no environment variable. It writes `frame.png`, `mean.png`, `grid-reference.png` and `grid-frame.png` under `./genos-debug/single-frame-cascade/`, or under `--out DIR`. `--dump` adds `samples.csv`, one row per compared block (pixel, point, normal, level of detail, distance, reference, frame).

`--scene` takes a `kind stress` scene or a `kind gltf` scene (`sponza`), the second with the ray tracer only. `--block B` compares B x B pixel means, as `capture.compare` grades (default 4); the reference's `--ref-samples` rays are split over the block's pixels.

## Phase 0, 24 Nov 2026

The baseline the cone tracer is graded against. Ray tracer, one frame, 8 px probe, 4x4 blocks, 16,384 reference rays a block (1,024 left the dark corner's reference itself 9% noisy):

| View | Gate | `--bin-cosine centre` (the paper) | `--bin-cosine mean` |
|---|---|---|---|
| Room no lamp lights | under 10% | 29.5%, bias +10.7% | 5.3%, bias −2.6% |
| Hall | under 7% | 5.7% | 5.5% |
| Sponza atrium | under 10% | 5.6% | 5.5% |
| Sponza dark corner | under 10% | 9.4% | 6.1% |

The cause of the room's 30% was not the three gaps listed below. With one cascade and rays to 200 m (no merge), and with one-pixel probes and 64 rays a pixel (no spatial sharing), the floor was still 50 to 69% too bright and the wall facing the camera 18 to 23% too dark. The room's light comes from a few bright patches. A cascade-0 direction is one of 32, and the shade weighed the light in it by the cosine of the direction's centre. A patch near the floor's horizon fell in a direction whose centre is well above it, so the floor took too much of it; a wall facing the patch took too little. Four times the directions halved it.

The fix keeps, for each direction, the sum of each deposit's luma times its direction, merges that sum as the cone is merged, and shades with the cosine of its mean direction, blended toward the centre's cosine as the light in the direction spreads out. The weight that normalises the sum is still the centre's cosine. Cost: three numbers more per direction. This is a departure from the paper (section 4, equation 2 with the direction's centre).

Not done, and not needed for the gate: the blend across the level-of-detail seam, and depth-aware weights in the sparse trilinear read.

## Results, 23 Nov 2026

1280×720, 32 directions at cascade 0, 4 cascades, `t0` 1.6, 1 ray per pixel, 8 frames with new jitter. Error is the mean absolute difference over the reference mean. Noise is the standard deviation over the mean across the 8 frames, per compared pixel.

One bounce (`nee`):

| View | Probe | Rays a frame | Shadow rays a frame | Error, 1 frame | Bias | Noise, median | Noise, p90 |
|---|---|---|---|---|---|---|---|
| hall (lit by lamps) | 8 px | 910,660 | 4,489,799 | 6.4% | −1.4% | 2.8% | 11.6% |
| room no lamp lights | 4 px | 921,600 | 4,237,229 | 33.9% | +6.1% | 19.4% | 71.9% |
| room no lamp lights | 8 px | 921,600 | 4,238,651 | 30.0% | +10.9% | 3.0% | 10.1% |
| room no lamp lights | 12 px | 921,600 | 4,237,175 | 29.8% | +11.0% | 2.0% | 4.8% |

Lamps as glowing spheres (`emissive`), hall, 8 px probe, 910,660 rays a frame, no shadow rays:

| Lamp radius | Error, 1 frame | Error, mean of 8 | Noise, median | Noise, p90 |
|---|---|---|---|---|
| 5 cm | 54% | 32% | 130% | 283% |
| 10 cm | 33% | 24% | 72% | 186% |
| 20 cm | 23% | 20% | 34% | 100% |
| 40 cm | 16% | 14% | 14% | 53% |

What the numbers say:

- One ray per pixel is enough for one frame of bounce light with no temporal mean, when the probe is 8 pixels or wider: 2 to 3% noise. At 4 pixels a probe gets fewer rays than it has directions, and 15% of the directions a pixel needs have no data.
- The error in the room no lamp lights is 30% and does not fall with more frames, so it is this prototype and not noise. Known gaps against the paper: no blend across the level-of-detail seam, no depth-aware weights in the trilinear read (light crosses a wall where probes on both sides are read), and a direction that straddles the horizon takes its whole cosine.
- The lamp shadow rays are 4.6 for each cascade ray with 5 point lamps and a sun. That is the cost that grows with the lamp count.
- Lamps as glowing spheres need no shadow ray, and at one ray per pixel they are far too noisy at lamp sizes under about half a metre. This is the emitter-size limit the paper states: a lamp narrower than the gap between rays at its distance is missed.

## The cone tracer

`--tracer cone` replaces every cascade ray with a cone marched through a sparse voxel hierarchy of surface points (6.25 cm to 8 m). No scene trace runs in the frame. `--voxels world` builds the voxels from every surface of the building; `--voxels screen` builds them from the points the picture shows.

- A voxel keeps, for each of the six axis directions, the surface area that faces that way and that area's outgoing light. A lamp is a sphere's disc area and its light, spread over the voxels the sphere fills.
- A cone's width in a cascade's interval is that cascade's direction spacing (`--cone-width` scales it). It steps one voxel at a time, at the voxel level as wide as the cone, and reads the eight voxels around the point (trilinear).
- Only surfaces that face the cone cover it and light it. Cover adds up along the cone and stops at 1.
- Each interval deposits its light and the share that passes through into its cascade, as a split ray does. The merge and the shade are the same code as for rays.
- In `nee` mode a surface point's outgoing light is its direct light, one sample for each 6.25 cm patch, cast once while nothing moves. In `emissive` mode surfaces are black and the lamps are in the voxels.

Results, 23 Nov 2026, same settings as above (8 px probe, 1 cone per pixel):

| Case | Tracer | Error, 1 frame | Bias | Noise, median | Scene rays a frame | Voxel reads a frame |
|---|---|---|---|---|---|---|
| hall, one bounce | ray | 6.4% | −1.4% | 2.8% | 910,660 + 4,489,799 shadow | 0 |
| hall, one bounce | cone, world voxels | 11.6% | −7.9% | 1.3% | 0 | 195 million |
| hall, one bounce | cone, world voxels, width 0.5 | 8.3% | −4.9% | 1.8% | 0 | about 330 million |
| hall, one bounce | cone, screen voxels | 23.0% | −10.8% | 1.1% | 0 | 337 million |
| room no lamp lights, one bounce | ray | 30.0% | +10.9% | 3.0% | 921,600 + 4,238,651 shadow | 0 |
| room no lamp lights, one bounce | cone, world voxels | 76.2% | +64.7% | 1.3% | 0 | 175 million |
| hall, lamps as 5 cm spheres | ray | 54% | +2.5% | 130% | 910,660 | 0 |
| hall, lamps as 5 cm spheres | cone, world voxels | 21.4% | +5.6% | 9.7% | 0 | 195 million |
| hall, lamps as 20 cm spheres | ray | 23% | −1.5% | 34% | 910,660 | 0 |
| hall, lamps as 20 cm spheres | cone, world voxels | 15.7% | +6.0% | 7.6% | 0 | 194 million |

What the numbers say:

- A cone finds a small lamp. A 5 cm lamp through cones has 9.7% noise in one frame, against 130% through rays, with no shadow ray and no cost for each lamp.
- Bounce light through cones is quieter than through rays (1.3% against 2.8%) and less exact in a lit view (11.6% against 6.4%). A narrower cone is more exact and reads more voxels.
- The room no lamp lights is 65% too bright through cones. That is light leaking: a voxel wider than a wall holds the lit surfaces behind it, and the trilinear read reaches them before the wall has covered the cone. A narrower cone did not fix it (width 0.5 and 0.25 gave +85% and +91% before the lighting patches were made finer). This is the accuracy problem Sannikov names for tracing probes.
- Voxels from the picture alone double the error in the hall (23%): the surfaces the camera does not see send no bounce.
- The cost is voxel reads: about 215 for each cone, 27 steps of 8 reads. On a GPU with a filtered 3D texture that is 27 samples a cone, about 25 million a frame at 720p. A hash table needs all eight reads. Neither is measured on a GPU.
- Two fixes made during this run, both in this prototype and not in the method: one direct-light sample for a 25 cm patch lit the dark side of a wall (now 6.25 cm), and a lamp wider than a voxel over-filled one voxel and lost half its light (now spread over the voxels it fills).

## GPU timing

`--gpu-bench` times the trace step of both tracers on the GPU and stops. It opens its own headless Vulkan device, as `genos-gpuref` does.

- **Cones:** `shaders/cone.comp`, the CPU march line for line, one cone a pixel. The voxel levels are a clip volume around the eye: 96 x 96 x 96 texels a level, eight levels stacked on z, in seven RGBA16F 3D textures (one for each axis direction and one for the lamps, 378 MiB). The hardware's trilinear filter is the prototype's eight-voxel read. A cone reads the lamp texture and the three direction textures that face it at every step.
- **Rays:** `genos-gpuref`'s path shader on hardware ray queries, one path a pixel with one bounce: a camera ray, a bounce ray, and a shadow ray to every lamp at both hits. One cascade frame is one ray a pixel and the shadow rays at its hit, about half of that. `Output::dispatch_ms` is the dispatch alone, without the host's sums.

RTX 4070, 1280x720, hall, 23 Nov 2026:

| Trace step | 6 lamps | 49 lamps |
|---|---|---|
| Cone march, one cone a pixel | 11.8 ms | 10.2 ms |
| Hardware rays, one cascade frame (estimate: half a path) | 3.3 ms | 19.7 ms |
| Today's renderer, all of lighting (`lighting.gi_metrics`) | 11.4 ms | 35.9 ms at 48 |

The cone march takes 29.9 steps and 104.8 filtered texture reads a cone in the hall (25.9 and 90.4 in the room no lamp lights, 9.8 ms). The two cone runs differ by run-to-run noise: the march does not read the lamp count.

What the numbers say:

- The cone march costs the same at 6 and at 49 lamps. The ray version's shadow rays grow with the lamps: six times the cost at eight times the lamps.
- At 6 lamps hardware rays are about three and a half times faster than this cone march. The march is not tuned: four textures a step, full resolution, one cone a pixel.
- Neither number is a frame. Not timed: building and lighting the voxels (and updating them when something moves), the probe insert, the deposit, the merge and the shade. The paper gives 1.2 + 0.4 + 2.2 + 1.0 + 2.5 = 7.3 ms for those five on an RTX 3080 laptop.
- The software tracer is not timed.

Not measured: Sponza or any glTF scene (the prototype takes a `kind stress` scene only), the cost of the deposit, merge and shade steps on a GPU, and the software tracer's rays per millisecond. The offline hardware path tracer (`crates/gpuref`) traced at least 400,000 rays per millisecond in this building on an RTX 4070, a lower bound.

## Phase 1, 24 Nov 2026

The cone tracer against the leak into the room no lamp lights. The gate (room within 7.95%, bias under 10%) missed; the plan's section 11 has the table and what was tried. Arguments added for it:

- `--voxel-order none|front` (default `front`): a voxel also keeps, for each of its six directions, the centroid of the area that faces that way; with `front` that area shows to a cone only when the cone starts on the side it faces. The sunlit top of a roof no longer lights the ceiling under it.
- `--refine-level L` (default 3): a step at a coarser level that meets cover is walked again at level L (12.5 cm), whose read cannot reach across a 30 cm wall. 9 is off.
- `--max-level L`: the coarsest level a cone reads.
- `--probe-pixel X,Y`: one pixel's ray and cone in 1024 cosine directions, and the directions where they differ most.
- `--voxel-check`: the finest voxels' stored light over the exact direct light of the pixels in them, by orientation.
