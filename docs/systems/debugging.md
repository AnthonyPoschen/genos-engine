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
