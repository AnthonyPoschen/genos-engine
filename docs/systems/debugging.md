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

## gi-check

`cargo run --release -p genos-stress -- gi-check [--out DIR]` is the GI picture gate.
It runs this build of the stress test four times (one window at a time) and prints a
table of PASS or FAIL against fixed limits; exit status 1 on any FAIL.

- `settled`: the five diagnosis views (room-a, corner, contact, lamp-wall, hall) at
  640x360, clock paused, each compared with a 320x180 reference
  (`scripts/gi_check/views.rhai`).
- `moving`: the same views after 120 frames of the scene's own motion (lamps 50 %
  dynamic, boxes moving, fixed 60 Hz step, padded so every build reaches the same
  scene moment), paused on the last moving frame and compared with a reference of
  that frame.
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
