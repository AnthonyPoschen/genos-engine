# genos-stress

Lighting stress test. One 25 m building section tiles into a grid, so the cost of
size reads as a delta between `small` and `big` and can be projected further.

```text
make stress                 # small, 5 lamps, half of them moving, sun running
make stress-big             # 16 sections, 100 lamps, 75% moving
make stress-bench           # the sweep below, 10 s per configuration
cargo run --release -p genos-stress -- --help
```

## The building

One section is 25 m × 25 m under a flat roof at 3 m (slabs 3.00-3.25 m). It has:

- **Room A** (8 × 8 m), the bounce-only room. No window, no roof hole; one 6 m opening
  faces south onto a 2.5 m light well with its own roof slot and a full-height screen
  wall. The sun always stands in the north, so its rays can never pass a south-facing
  opening, and no lamp slot has a straight line to the opening (a unit test checks
  every slot along its whole path). Room A is lit by the sun bouncing off the light
  well and the screen by day and by lamp light bouncing in from the hall at night.
  It does see a thin band of sky up through the light well's slot (steep rays under
  the opening's head), so by day it also takes a little sky light directly; at night,
  or with `--no-sky`, it is lit by bounce alone.
  A red box, a blue cylinder and a green wall panel are the colour-bleed checks.
- **Hall**: the open middle with a 5 × 4 m skylight, four pillars, a 1.6 m partition
  that light passes over, benches, a green box orbiting under the skylight and a red
  box sliding along the south end. Both spin.
- **Room C**: small windows only (on the building edge), a shelf and a crate.
- **Room B** (sunroom): wide window on the edge, its own skylight, a wide opening to the
  hall, a planter and a blue box sliding and spinning.
- A corridor between B and C with a narrow roof slot.

Outside walls carry windows of four sizes (narrow, high, normal, wide) and 3-4 m
doorways. Shared section edges get 4-6 m openings so the sections join into one open
building. The floor runs 8 m past the walls so the sun lands outside too.

| scale | sections | footprint | floor area | occluders | moving boxes |
|---|---|---|---|---|---|
| `small` | 1 | 25 m × 25 m | 625 m² | 87 | 3 |
| `big` | 4 × 4 | 100 m × 100 m | 10 000 m² (16×) | 1 068 | 48 |
| `N` / `NxM` | N × N / N × M | 25 m per section | up to 64 sections | | 3 per section |

From the benchmark viewpoint (section 0, the north-west corner) the far corner of
`big` is about 120 m away: past the 50 m probe tier radius and at the 120 m reach of
the world probe volume. `--scale 6` (150 m) is past both.

## Lamps

`--lights N` is the lamp count for the whole building (panel: 5/25/50/100).
`--dynamic PCT` is the share that moves (panel: 0/25/50/75/100%). Every lamp has a
fixed slot: a home and a path (an orbit or a sway) that passes through the home at
time 0. A still lamp sits at its home; a moving lamp follows its path. Which lamps
move is fixed by the lamp index (golden-ratio order), so raising the share only adds
movers and no lamp jumps when the share changes. The first five slots of a section are
placed by hand (hall west, room C, room B, hall south, hall north); the rest are
seeded.

`--layout spread` (default) puts lamp `k` in section `k mod S`, so the lamps spread
over the building. `--layout first` puts every lamp in section 0, where the benchmark
camera stands: the view then sees the same lamps at every scale, and a small → big
delta is the cost of size alone (more walls, more moving boxes, a bigger area to keep
lit). The panel switches it live.

All lamps share one total (`LAMP_TOTAL` = 1, `--power` scales it): five lamps are 0.2
each, a hundred are 0.01 each. A lamp's range follows its brightness
(`genos_render::lamp_range`), about 43 m at 0.2 and 10 m at 0.01.

## Sun

A directional light on a day/night cycle: 120 s per day by default
(`--day-seconds`), `--sun-speed`, `--freeze-sun`, `--time` (0 sunrise in the east,
0.25 noon, 0.5 sunset in the west, the arc leaning north as seen from Sydney). The
night half has no sun. The sun fades in over the first ~6° above the horizon (a stand-in
for the long air path; it costs nothing).

The sky (`Scene::sky`) follows the sun: (0.11, 0.14, 0.19) radiance at full day, so
open ground takes about a sixth of the noon sun's light from it; it brightens until
the sun is 30° up, glows through twilight to 6° below the horizon and is gone at
night. The windows and skylights show it, and probe rays that leave the building
bring it in. `--no-sky` or the panel's Sky row turns it off.

## Determinism

`--seed` (default 1) fixes the lamp slots and the box phases. With `--frames`,
`--shot` and `--bench` every frame steps the scene by exactly 1/60 s, so a run draws
the same frames on any machine. Interactive runs use the wall clock.

## Benchmark

`--bench SECONDS` sweeps `--sweep-lights` (default 5,25,50,100) × `--sweep-dynamic`
(default 0,25,100) × `--sweep-scales` (default small,big) from the hall viewpoint with
the sun running. A bench lays the lamps out `first` unless `--layout` says otherwise, so
the camera sees the same lamps at every scale and a scale delta is the cost of size. It
opens the proof window (it floats and takes no focus, so the compositor keeps its size)
at 2560x1440 unless `--size` says otherwise, follows any resize, and reports the size
each run drew at. Each configuration rewinds the clocks, draws `--warmup` frames
(default 30), then measures for SECONDS. It prints Markdown tables (and writes them
to `--bench-out`):

- point lamps whose range reaches into the view (the lamps the pixel loops can meet);
- frame time (wall clock per frame) and its p95;
- draw-call CPU time;
- raster GPU time (Vulkan timestamps around the draw; the light builds run on their own
  queue at the same time, so this span includes GPU time the two share);
- light build GPU time per frame and builds per second (`Renderer::take_light_builds`);
- the same split into the passes copy, world direct, world bounce and tier;
- the cost of one static and one dynamic lamp: the slope between the fewest and the
  most lamps at 0% and at 100% moving.

Each table has the change from the first scale to the others, in ms and %.

## Camera

The camera flies: WASD/IJKL along the view, Space up, C down, Shift ×4, mouse look
after a click, Escape releases. The engine's physics world holds 32 bodies, far
fewer than one section's walls, so there are no colliders. `--view hall|roomA` picks
a start pose; `--eye X Y Z YAW PITCH` sets any pose.
