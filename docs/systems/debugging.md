# Debugging

## Goal

An agent or a person can see, change and measure any running engine system from
outside, the same way every time: over the MCP port, from a script file, or headless
on a box. Lighting quality is judged against ground truth, not by eye alone.

## Intent

- One set of commands. MCP tools, `POST /cmd/<name>`, and Rhai functions are the
  same commands with the same JSON arguments and replies.
- Engine-generic. `genos-debug` knows the renderer, the scene and the camera; an
  example adds only its own knobs (the `Host` trait) and four calls in its frame loop.
- Deterministic when asked. A script runs scene time on a fixed step from frame 0.
  The probe budget follows measured GPU time, so for repeatable light set
  `lighting(#{ tier_ms: ... })` and `wait_settled` before reading.
- Ground truth is a CPU path trace of the same frozen scene from the same pose, with
  the picture's lamp unit, paint albedo and tone curve. Until the reference has a
  longer record, look at every reference before using it as a target.

## Code

- `crates/mcp/src/command.rs`: the command queue. `register` lists tools, `call`
  blocks a client, `take`/`reply` run on the frame thread. HTTP: `POST /cmd/<name>`
  with a JSON body, `GET /cmd` for the list. MCP `tools/call` reaches the same queue.
- `crates/debug` (`genos-debug`):
  - `tools.rs`: `Tools`. `scene_dt` (pause, step, fixed step), `before_draw` (runs
    commands, puts back light/object/camera edits, sets the view), `overlay` (probe
    markers), `after_draw` (feeds waits, captures, flicker windows, references).
  - `commands.rs`: every command and its arguments (the MCP descriptions).
  - `reference.rs`: the path tracer (`render`, `render_cached`) and `compare`.
  - `image.rs`: captures, resampling, region luminance, diff, flicker.
  - `knobs.rs`: named knobs (`Knob`) and the `lighting.*` knobs.
  - `script.rs`: the Rhai runner and its report.
- `crates/render/src/debug_view.rs`: view modes, bounce limit, `LightingConfig`,
  `probe_report`, `lamp_report`.
- `crates/render/src/trace.rs`: the CPU surfaces and direct light the reference uses.
- `examples/stress/src/knobs.rs`: the stress knobs (time of day, sun, lights, ...).
- `examples/stress/scripts/*.rhai`: regression scripts. `reference.rhai` holds noon
  and night at the hall bench pose to ground truth; `reference_views.rhai` measures
  the hall, room A (bounce only), a doorway, the outside and room A's corner.

## Running

Interactive, then drive it over HTTP (any MCP client sees the same tools):

```sh
GENOS_MCP_PORT=8801 cargo run --release -p genos-stress
curl -s localhost:8801/cmd                                   # list commands
curl -s -X POST localhost:8801/cmd/time -d '{"pause":true}'
curl -s -X POST localhost:8801/cmd/camera -d '{"position":[12.5,1.7,20],"look_at":[12.5,1,8]}'
curl -s -X POST localhost:8801/cmd/view -d '{"mode":"probes:state"}'
curl -s -X POST localhost:8801/cmd/capture -d '{"mode":"direct","path":"/tmp/direct.png"}'
curl -s -X POST localhost:8801/cmd/lighting -d '{"bounces":1,"tier_ms":8}'
curl -s -X POST localhost:8801/cmd/compare -d '{"width":160,"noise":0.03,"prefix":"/tmp/noon"}'
```

Do not stop a `genos-stress` someone else bound to port 8765; pick another
`GENOS_MCP_PORT`.

Headless, a script with a report (exit code 1 when a check fails):

```sh
genos-stress --proof --no-panel --size 320x180 \
  --script examples/stress/scripts/smoke.rhai --report target/debug-reports/smoke
genos-stress ... --script FILE --trace   # also a screenshot after every changing step
```

On a box without a GPU this runs on lavapipe under a headless Weston
(`XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`). The first run after a shader change spends
minutes compiling pipelines.

The report directory holds `report.md` (checks, log, every step with its reply,
images), `result.json`, and every image the script saved.

## Scripts

Every command is a function taking a map: `capture(#{ mode: "albedo" })`. Short
forms: `wait(n)`, `step(n)`, `view("direct")`, `knob(name, value)`, `knob(name)`,
`shot(name)`, `shot(name, mode)`, `log(text)`, `out_dir()`. Checks: `check(bool,
msg)`, `require(bool, msg)` (stops the script), `check_lt(a, b, msg)`,
`check_gt(a, b, msg)`, `check_near(a, b, tol, msg)`.

```rhai
time(#{ pause: true });
knob("time_of_day", 12.0);
knob("sun", "freeze");
camera(#{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] });
wait_settled(#{ timeout_frames: 900 });
shot("noon-direct", "direct");
let c = compare(#{ prefix: "noon", width: 160, noise: 0.03, frames: 30 });
check_lt(c.mean_rel, 0.10, "under 10% of the light is wrong");
```

### Settling

`wait_settled` replies once every brick in view has no pass left, the build that
carried the last one is on screen, and the shown light has blended 95% of the way to
it (three `TIER_VIEW_SECONDS` of wall time), for `quiet_frames` in a row. The reply
gives `frames`, `wall_ms` and `light_gpu_ms` (GPU time of the light builds during
the wait); compare builds on the milliseconds, since frames favour whichever build
draws faster. `status` reports `picture_settled`.

The yardstick changed at the commit "wait_settled waits for the picture to stop
changing". Before it, `wait_settled` replied when the last pass was submitted, while
the picture still blended toward it; frame counts from before that commit
(light_settle, room_switch, reference_views settle frames) read lower and are not
comparable with later ones. `temporal_flicker` measured that blend tail as flicker.

## Views

`full`, `direct` (lamps and sun, shadowed), `bounce` (indirect only), `nobounce`
(= `full:0`), `bounce:N` / `full:N` (bounce limit 0, 1, 2 or inf), `near`, `far`
(near-field rays or far field only), `albedo`, `normal`, `depth`, `light:I` (one
lamp and its shadow), `probes:state|age|priority|spacing|changing` (screen markers
at brick centres: unlit grey, changing red, refining yellow, steady green).

## Ground truth

`reference` traces; `compare` traces (or reuses `reference: name`) and compares.

- Primary rays through each pixel centre from the camera pose (60 degree vertical
  field of view, the picture's aspect). Each sample gathers shadowed direct light
  from every lamp and the sun at each vertex and continues along a cosine-sampled
  bounce; the first bounce is stratified per pixel. Russian roulette ends paths
  after three bounces (`bounces` caps them, default 64; `bounces: 0` is direct only).
  A miss returns the sky colour.
- `geometry: "triangles"` traces the scene's triangle form (`SceneMesh`: the
  floor, roof, walls and solids as meshes, through the CPU BVH in `genos-mesh`)
  instead of the analytic shapes (`"shapes"`, the default). Both agree within the
  trace's noise (`triangles_match_the_analytic_reference`); GI v2 traces meshes only.
  Triangle traces are cached under a separate key.
- Passes of `spp` paths per pixel run on every core until the mean relative
  standard error of a pixel is at most `noise` (default 0.03), `max_spp` or
  `seconds` (default 300). The reply carries `spp` done, `noise`, `trace_seconds`.
- Traces are cached under `target/reference-cache` (or `$GENOS_REFERENCE_CACHE`;
  `cache: "off"`) by scene, pose, size and bounce limit.
- `compare` captures the live picture after the trace (`frames: N` waits at least N
  frames after the trace starts; `mode` picks the view; `live:` uses a saved
  capture), resamples it to the reference size and compares linear luminance (the
  tone curve undone) on `block` x `block` averages (default 2). Pixels where the
  primary ray met nothing are left out.
- Reply: `mean_rel` (sum of |live - ref| over sum of ref: the share of light that is
  wrong), `mean_abs`, `p95_rel`, `p95_abs`, `bias` (above 0: live too bright),
  `ref_mean`, `live_mean`, and `regions` (a `grid`, default 3x3, row-major, each
  `mean_rel` and `bias`). Files: `<prefix>-reference.png`, `-live.png`,
  `-heatmap.png` (red: live too bright, blue: too dark, full colour at 50 %).

Checks that hold the tracer honest (`cargo test -p genos-debug --release`):
a floor under a white sky returns its albedo (furnace); a shut white room keeps
1 / (1 - 0.8) = 5x its direct light; a lamp falls off with the square of distance
and the cosine; a red wall bleeds red onto a white floor. `known_scenes` (ignored;
`GENOS_REFERENCE_OUT=dir`) renders a Cornell room, a lamp over a floor and a sunlit
wall to look at. In the stress scene the engine's `direct` view matches a
`bounces: 0` trace to about 2 %.

GPU reference (`genos-gpuref`, `GENOS_REFERENCE_GPU=1`): the same paths on the
4070's hardware ray queries (Vulkan `VK_KHR_ray_query`, headless, no GI v2 code):
the scene's triangle form in two BLAS (one-sided culled, two-sided), the same
Lambert materials, lamps (72 units, 10 cm), sun and sky, light sampling at every
vertex, Russian roulette from the second bounce, f32 per path and f64 sums, two
half buffers for the noise estimate, no denoiser. Rays start 1e-5 from a surface
(the CPU proof's 1e-3 lets a shadow ray from a concave corner skip the cross wall:
the column of white specks along the room's long-wall seam in the old references).
Validated against the CPU proof at five stress poses (160x90, 1024 paths a pixel):
the difference is 0.47-0.82 of the CPU's own noise, bias at most 0.0003
(`gpu_reference` test in genos-stress, ignored). About 21x the CPU's paths a second
at 8 threads (5x on the sky-heavy outside view); the room references at noise 0.01
(320x180, about 150k paths a pixel) take about 260 s each. Cached as the CPU traces
under their own key. The CPU proof runs at nice and half the cores
(`GENOS_REFERENCE_THREADS`).

## gi-check

`cargo run --release -p genos-stress -- gi-check [--out DIR]` is the GI picture gate.
It runs this build of the stress test four times (one window at a time) and prints a
table of PASS or FAIL against fixed limits; exit status 1 on any FAIL. A few rows
are targets GI v2 is working towards (the room's 48 px swim and light structure):
a miss prints MISS and fails nothing.

- `settled`: the five diagnosis views (room-a, corner, contact, lamp-wall, hall) at
  640x360, clock paused, each compared with a 320x180 reference
  (`scripts/gi_check/views.rhai`).
- `moving`: the same views after 120 frames of the scene's own motion (lamps 50 %
  dynamic, boxes moving, fixed 60 Hz step, padded so every build reaches the same
  scene moment), paused on the last moving frame and compared with a reference of
  that frame.
- `repro`: the moving-camera brightness repro (`scripts/gi_check/repro.rhai`,
  bounce view): settle at the contact pose, then three 24-frame clips that each end
  on it (a 1e-5 rad wiggle, a 1.2 m slide, a 20° turn), every frame saved. Rows:
  |mean of each clip's last frame / settled mean - 1| (limit 0.03). The same check
  runs on lavapipe as `gi_v2_moving_camera_keeps_its_brightness` (gi2_frame.rs).
- `walk`: the corner walk at 1280x720, every frame of the bounce view saved, cold then
  warm (`scripts/gi_check/walk.rhai`).
- `flicker`: `temporal_flicker_aligned.rhai` at 1280x720, with a heatmap per pose.
- `camera`: GI while only the camera moves (`scripts/gi_check/camera.rhai`, 640x360,
  bounce view). Moving pass: the corner path at 6 m/s (the fly speed) with a 90°
  turn at 6° a frame, then a turn on the spot at 3° a frame; one pose a frame at a
  fixed 60 Hz step, every frame saved, no pause or script call between frames, so
  the GI gets one frame per pose as live. Then the camera stops on the last pose for
  60 more frames, all saved. Reference pass: the camera stands at every `--stride`-th
  pose (default 1, and the last) until the GI settles, one capture each; this pass
  steps and stops, but it only makes the answers. Rows: mid-move error (each moving
  frame against the settled picture at its pose, mean), mid-move worst tile (the
  worst 10x10 px tile of any frame), mid-move flicker (frame-to-frame change beyond
  the settled pictures' own change), post-stop error (the first 10 frames after
  stopping against the last pose's settled picture) and post-stop frames to settle
  (until the error stays under 1 %). It also prints the rays each probe held while
  moving.
- `trail`: the trail a moving box leaves (`scripts/gi_check/trail.rhai`, 640x360,
  bounce view, camera still). Moving pass: the box nearest the contact view's target
  slides 1.2 m over 40 frames (about 1.8 m/s) and stays 30 frames, every frame saved,
  one pose a frame as live. Reference pass: the box stands at every `--stride`-th of
  those poses until the GI settles (bounce and depth views). Rows: trail error (each
  moving frame against the settled one inside the space the box left over the last
  10 frames, from the depth views, grown 6 px at 320x180, minus where it stands now;
  mean over frames) and trail worst frame.

Probes are spaced as the 16 px tile at 1440p in every run (`GENOS_GI2_TILE` =
height / 90 unless set).

Scores (all in linear luminance, shares of the light; the five-view mean is checked):
mean error (as `compare`); blob, the band-pass (sigma 2 to 10 px) residual on flat
evenly lit surfaces minus what the reference's own noise puts through the same band;
contact error within 2 px of lines where surfaces meet; open-wall error more than 20 px
from them; corner-walk pop, the worst 10x10 px tile (at 320x180) where a cold frame
differs from the warm frame at the same pose; flicker frozen `max_delta` and moving
`mean_delta`. Limits: mean 0.04, blob 0.02 (worst view 0.03), contact 0.045, open wall
0.03, pop 0.05, frozen 0.004, moving flicker 0.0005. Every FAIL line names the picture
to look at (the worst view's heatmap, the worst walk frame, the worst pose's flicker
heatmap). Results land in `target/gi-check` with `gi-check.txt`; `--score-only`
rescores a folder, `--only settled,walk` picks runs (settled, moving, walk, flicker, camera, trail), `--gi v1` runs the old path.
References trace to 0.05 noise or 2048 paths per pixel (`--ref-noise`, `--ref-spp`)
and are cached as for `compare`, so only the first run traces them (about 4 minutes
a view on 16 cores).

### Tiers

`--tier quick|mid|full` picks the runs; every tier stops at the first run with a
FAIL. Iterate on quick, run mid when quick passes, run full only before pushing.

- `quick`: repro; the contact and lamp-wall views and the close-up settled; the
  trail (reference every 4th); the flick run; the turn, room, mix and spin runs;
  640x360.
  No moving views: each needs a fresh reference of its last frame, minutes each
  and too noisy when capped; the camera clip and the trail are quick's moving
  checks.
- `mid`: repro; all five views settled and moving; the whole camera path, the
  trail (references every 2nd pose), the flick, turn, room, mix and spin runs.
- `full`: repro, settled, moving, camera, trail, flick, turn, room, mix, spin and flicker,
  references at every pose. The corner walk (`--only walk`) is no longer in a tier.

Cut 2026-10-10 (Anthony: cut what adds no value; keep what caught a regression
or is the only coverage of a situation):
- quick's 90-pose camera clip (45 s): the turn run covers camera motion (turning
  both ways, walking, stopping, returning) in 15-25 s, against the pose settled,
  and caught the turn swim the clip passed. The whole camera path stays in mid
  and full (the only long mixed path; its worst-tile row caught the per-ray
  gather bug).
- full's corner walk (`walk`, 19 s): its pop row failed on every GI v2 build
  (0.44-0.49, worst 10x10 tile of cold against warm, noise-bound) and never
  pointed at a fix; walking is covered by the turn run's walk segment and the
  camera path. Still runnable with `--only walk`.

Kept: repro (caught both brightness bugs), settled (the only check against
path-traced references), moving views (caught the moving mean), camera (per-ray
gather bug), trail (the only check of the space a moving box leaves), flick
(caught the box-crossing flicker in 5 s), flicker (the only frozen-picture
check), turn.

### Moving tests: soak, move, return

Rule (Anthony, 2026-10-10) for every moving test: soak the scene until the
probes report settled (`wait_settled`, the app's settled signal, never a fixed
frame count), save that settled frame as the baseline, then start the motion.
Score the onset (the first moving frames against the same poses settled: a pop
when motion starts) and, where the path ends at a pose that was already settled,
the return to the baseline (a state that motion leaves behind and that never
recovers). Status: the turn run does all three (rows: arrival, onset pop,
return to the soaked baseline). camera, trail, flick, flicker and the moving
views already soak with `wait_settled` before moving; saving their baseline and
scoring onset and return is the next step.

The camera flicker row needs references at consecutive poses, so only full has it.
Quick's two views are a pair whose mean sits near the five-view mean (corner alone
fails the five-view limits even when the five pass).

`--keep-going` (after `--tier`) runs every run of the tier even past a FAIL.
Without `--tier`, `--only` picks the runs (default settled, moving, walk, flicker)
and nothing stops early.

### Validation

Hand-checked on 2026-10-10 by looking at the pictures behind every row:

- The metrics match the earlier Python tool (`splotch_metrics.py`) to the fourth
  decimal on the same pictures.
- GI v2 settled (gi-v2 7065edb): all view rows PASS (mean 0.037, blob 0.014, worst
  view lamp-wall 0.027, which shows faint floor blotches). Right: this is the known
  good state.
- GI v2 with no still averaging (the moving state): all view rows FAIL (blob 0.049,
  corner 0.078, visibly blotched walls). Right.
- GI v1: mean, contact and open-wall FAIL (0.13 / 0.16 / 0.10: the corners and
  bounce-lit walls are visibly too dark), blob PASS (0.019: v1 is smooth). Right.
- Camera and trail runs (2026-10-10, 4070, 640x360, tile 4): GI v1 is the
  known-good picture in motion: mid-move error 0.028, worst tile 0.29, flicker
  0.034, post-stop 0.003 in 0 frames. GI v2 before the per-ray gather fix: 0.092 /
  0.63 / 0.12, its moving frames visibly blotched and 5 % too bright (14 % with
  the 5x5 strata reach). The worst-tile and flicker limits (0.35, 0.05) are set
  from v1 with a margin; the error limit stays 0.03. The mid-move worst-tile row
  is the mean over frames of each frame's worst 10x10 tile, over the tile's
  reference light or the frame's mean tile, whichever is larger. Fixed 2026-10-10: the
  corner path used to walk on to z 18 at the 1.6 m eye height, through the top
  edge of the hall's low partition (z 16.9-17.1, 1.6 m high), so frames 155-159
  showed the partition face filling the lower half and frame 159 (through it in
  the moving pass, not yet in the settled one) was every build's worst camera
  frame and the corner walk's worst pop. The path now stops at z 15 and turns
  there (camera.rhai, walk.rhai: 178 and 138 poses). Trail: v1 0.109 (its world probes lag), v2 0.057.
- Close-up (the hall's west bench at 1.3 m, settled only, own rows): GI v2
  1129d5d mean 0.030, contact 0.026, open wall 0.028; the picture shows the
  bench face, top and foot with their contact shading. Limits 0.045 each.
- Flick run (2026-10-10, 4070, 12 s): the doorway pose while lamps and boxes move,
  40 frames, upper third, per 4x4 block at 320x180 the frame-to-frame change
  less the net change, per frame (light going back and forth, not the lamps'
  steady change). GI v2 1129d5d, where kept rays a moving box crossed were
  dropped until their stratum came round: 0.0005 (all change 0.00058), its
  frames show the walls around the boxes shimmering. With the re-trace in the
  frame (defa10a): 0.0001 (all change 0.00016). Limit 0.0003. The full tier's
  moving flicker row on the same builds: 0.0008 FAIL, 0.0004 PASS.
- Turn run (2026-10-10, 4070, about 20 s; turn.rhai): Anthony's live pose in the
  hall (11.59, 1.7, 23.52, yaw -1.0, pitch -0.166, evening, lamps only), bounce
  view at 640x360, clock fixed, sun, lamps and boxes still, so only the camera
  moves and every path ends at the soaked pose. ref.png is the pose settled from
  the process start (the baseline). Segments: turn (15 frames at 3 degrees a
  frame, his real rate, arriving at the pose), walk (15 frames at 5 cm, the
  control) and back (out 45 degrees at 3 a frame and straight back); each holds
  30 frames and then soaks until settled (end.png). Rows (blob: both blurred 8 px,
  mean |difference| over the reference's mean light, linear): arrival frame of
  the turn and of the walk against ref.png; onset pop, the first 5 moving frames
  of back against those poses settled (the first saved moving frame already
  stands one step in, checked by matching); the soaked end frames and the frame
  held after back against ref.png. GI v2 10ce430, whose turn Anthony saw swim
  live and in our scripted turn: turn arrival 0.0131 / 0.0108 (two runs), walk
  0.0067 / 0.0073, onset 0.0147 / 0.0133 (rising from 0.009 on the first moving
  frame: the pop when motion starts), end and return 0.0001. The arrival and onset
  frames show the floor and ceiling blotched where the settled picture is
  smooth. With the fill and the pattern cells moved in from the surface: turn
  0.0046 / 0.0050, walk 0.0037 / 0.0038, onset 0.0051 / 0.0050, end and return
  0.0001. Limits: arrival 0.008, onset 0.009 (10ce430 fails both, walking
  passes), end and return 0.002. No stuck light was found after soaking on either
  build (end against a fresh render in a second process: 0.0027-0.0039 on
  10ce430, the process-to-process floor 0.0027-0.0029).

- Room run (2026-10-11, 4070, about 20 s plus three path-traced references
  the first time; room.rhai): Anthony's `make run` spot in the default room
  (`genos-stress --scene scenes/room.rhai`: one point light, nothing moves) at
  (0.664, 1.699, 9.629), pitch -0.12, turning on the spot at 3 degrees a frame
  from yaw -38.9 to -16, to -57 and back, then held and soaked. Anthony saw the
  dim bounce-lit walls and floor swim there while turning. Graded on numbers
  alone, every score relative to the light where it is (blurred luminance plus
  5 % of the picture's mean, so a dim wall counts as much as a lit one):
  turn swim (metrics::world_swim: each moving frame turned onto the next by the
  camera's rotation, which moves a still world exactly, low-frequency change
  blurred 8 px, mean over the turn); grid redraw (the same between the pose
  settled and settled again 0.23 degrees, half a probe, further round: only the
  probe grid slid); blotch (metrics::blotch: the settled picture against the
  path-traced reference at two poses, the error band between 4 and 24 px at
  320x180, so an even bias does not count); end against the soaked baseline.
  Cause: the settled picture itself was mottled with 10-30 px blotches (the
  path-traced reference is smooth), and the pattern depended on where the
  screen-locked probe grid landed, so a turn redrew it every frame: the half-probe
  redraw (0.0036) was as large as a 3-degree turn settled (0.0036); kept strata
  added the rest (0.0044 moving; keeping no strata gave 0.0036). Two sources:
  the spatial filter's 3x3 neighbours one probe apart left blotches a few probes
  wide, and the light cache's 16 fixed rays a patch gave every patch an error
  of its own that probes carried onto the walls (the dim half; 64 rays halved
  it, 256 did no better). Tried and dropped: a plane-aware filter weight
  (streaks on slanted walls), a 5x5 neighbourhood (no gain), probes anchored to
  world-lattice pixels and shorter kept-stratum reach (no gain), cache patches
  averaging their relights with turned rays (blotch down, swim doubled: the
  cache flickers until it converges). Fix: the filter's three rounds take
  neighbours 2, 4 and 8 probes apart (a-trous), and 64 rays a cache patch.
  edffa81: swim 0.0045, redraw 0.0037, blotch 0.0270 (all FAIL; the pictures
  show the blotched walls against the smooth reference). Fixed: swim 0.0033,
  redraw 0.0019, blotch 0.0181. The blotch moves from process to process
  (edffa81 0.0250-0.0270, fixed 0.0176-0.0206 over three runs each), swim and
  redraw by 0.0001. Limits 0.0039, 0.0027, 0.023; end 0.002. The
  path-traced pair 3 degrees apart scores 0.0030 swim at 320x180 (its own noise).
- Mix run (mix.rhai): the hall turn pose while lamps and boxes move, 30 frames
  at 1/40 s, camera still and then turning at 3 degrees a frame; each moving
  frame against the same frame's scene state settled (clock knobs), relative
  error (metrics::rel_error, pixels off by more than 0.1 dropped, blurred 8 px).
  edffa81: still 0.0043, turning 0.0070; fixed (below and the room fix): 0.0021,
  0.0040. Limit 0.0055 for both (the still camera is the control).
- Spin run (mix.rhai, spin segment): camera still on the green box spinning
  (13, 1.6, 16.5), 24 frames; box-surface error and frame-to-frame flicker
  against each frame settled, where the settled picture differs from the
  baseline. Cause of the lag and artefacts on spinning boxes: probes on a box
  kept strata traced from where its surface was (the re-trace from there lands
  off the surface or inside the box), and the light cache around it lagged (new
  patches read 0 until relit). Fix (each on unless its variable is 0): a stratum
  traced from inside a change sphere is filled instead of kept
  (`GENOS_GI2_MOVED_DROP`); patches within 1 m of a change are relit first
  (`GENOS_GI2_URGENT`); a patch not lit yet reads its lit neighbours
  (`GENOS_GI2_BORROW`). Moving a patch to its reader's point was tried and made
  it worse. edffa81: error 0.0338, flicker 0.0302; fixed 0.0196, 0.0119.
  Limits 0.027, 0.02.

- Room grades against the GPU reference (2026-10-11, room run, references at
  noise 0.01): turn swim at 48 px (the same world swim blurred 48 px: the big soft
  blobs that slide; target 0.0006, GI v1 0.0003, edffa81 0.0024, 839d449 0.0026);
  light structure (metrics::structure: both pictures blurred 8 px at 320x180
  within each surface, the reference split at its sharp edges, mean |log ratio|
  with each surface's own mean removed, so a blotch in the wrong place counts even
  when the mean is right; gradient correlation shown beside it; target 0.010:
  GI v1 0.0595 / 0.76, edffa81 0.0218 / 0.94, 839d449 0.0147 / 0.97, the light
  cache read directly at 12.5 cm and 1024 rays 0.0092); close-up definition
  (metrics::definition: the 1.5-6 px band error against the reference at a pose
  1 m from the bounce-lit corridor wall, and how much of the reference's detail
  is kept; limit 0.013: 839d449 0.0106, GI v1 0.0154). The first pose by the red
  box was too dark to converge and was moved.
- World cells (GENOS_GI2_WORLD, experiment, off): 1, pixels read the light cache's
  patches interpolated: with 64 rays a patch the picture is blotched (structure
  0.0685) but as stable as GI v1; with 1024 rays at 12.5 cm structure 0.0092,
  swim 0.0006 / 0.0002 at 48 px, blotch 0.014, close-up 0.0149 (blocky). 2,
  probes average their irradiance into world cells: swim halves (cap 64: 0.0015 /
  0.0008) but the settled picture depends on the path there (settled per pose
  0.0066, end 0.0013-0.0021), with or without jittered probe placement.

## Limits

- The reference knows floors, ceilings, walls and solids. It does not trace fog,
  particle cards, textures or per-material reflectance and colour mix (the stress
  scene uses the defaults). It does not cut lamps at their range as the engine does.
- Stress at 160x90 needs about 3000 paths per pixel (4 minutes on 8 cores) for 3 %
  noise. Interiors lit only by bounce light converge slowest.
- `nobounce` is `full:0`: sky light reaches the picture only through probes, so
  "direct + sky" is not a separate view. Bounce limits apply to tier probes; the
  world volume beyond them is unlimited.
- `near` is black unless `lighting.near_rays` is set (the shader default is 0).
- Probe ray counts are constants; only the near rays and the tier budget are live.
- Light range is derived from colour and is read-only. Capture size is a CPU resample.
