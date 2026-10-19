# SUN — Small sun step

A directional sun that moves a little, while the camera, the meshes, and the point lamps hold, does not refresh every indirect sample. Indirect light catches up over bands. Direct sun and its shadows are traced and shown every frame.

This continues the lighting work-reduction cut. The picture compose stays the gi-v2 shader. A second compose pipeline is built the first time a small step needs it. Startup still compiles the nine lighting pipelines.

## What engages

The step is inside 2° of the last full refresh and inside 2% of that refresh's sun and sky irradiance. A frozen sun (the same direction bits and the same light) is not a step. A day-clock tick is about 0.05°. Treating that tick as "unchanged", then moving the stored sun forward, meant the delta never accumulated and the path never ran. Any change of the stored direction, still inside the budget, is a small step. The stored sun moves only when the frame is not amortising.

`GENOS_LIGHTING_FULL_EFFORT=1` or `--lighting-full-effort` turns the path off. Moving boxes or moving point lamps turn it off, because the view hash includes them. The default stress scene therefore stays on the full compose. A sun-only hold is `--still-boxes --dynamic 0`.

One probe-row band and one spatial page column (`page_x = cell.x >> 3`) refresh per frame, `frame % N`. The default N is 8. The sun compose is one full-grid dispatch. It writes this frame's direct on every pixel. It walks probes only for a pixel on that probe row or that page column, and for a pixel whose stored bounce is still empty. The first frame of a run clears that slice, so that frame walks once. There is no second dispatch and the dirty-block list is not used. The directional lamp's shadow epoch is not bumped. Gather and filter still run. Pixel direct reuse stays off for the step, so the shadow is not a frame behind; that cost sits in the direct pass, not in compose.

The neighbour mix is the compose that already exists: bilinear weights over 16 px, a plane-distance term, and normal to the eighth. A slice probe is mixed with neighbours that still hold the committed sun. There is no per-lamp irradiance delta layer, no probe hysteresis, no relocation, and no volume pause.

## Catch-up

S69C relights a kept hit on its stratum's frame and once halfway round, so a corner can sit for about fifteen frames after a point lamp stops. For `--lighting-catch-up` frames after the lamps hold (default 4, at most 16), the direct pass skips that early-out and shades every kept stratum. `0` leaves S69C as it is. The first frame a lamp is seen does not arm this. A frame that moves a lamp does not either. The next frame, with the lamps held, starts the count.

## Proof

`lighting_direct.comp` and the sun compose each write one stats word from a single thread. Bit 0 is the small-sun flag. Bit 1 is catch-up. The next word is the band. Those are the unfilled pad words (142 and 143). The read does not touch `light_quiet`.

`./genos-debug/sun/counter.txt` records `frame`, `sun`, `slice`, `catch`, `host_frames`, `catch_left`, `bands`, `full_effort`, `compose` (`full`, `list`, or `sun`), and `dispatches`. `--out DIR` writes `DIR/sun` instead. `GENOS_LIGHTING_DEBUG=1` also prints `lightingsun`. `host_frames` counts frames the CPU took the path. `sun` is the shader bit. `dispatches` is 1 on the sun path: the old path ran two compose dispatches inside one timestamp.

The file is written when the path has run, when full effort is on, or when debug is on. A still or moving bench never takes the sun path, so full effort and the default do the same work there and the file stays unwritten. A full-effort run with a moving sun writes `full_effort 1`, `sun 0`, `compose full`, and `host_frames` does not climb.

## Controls

The argument wins. The environment variable applies only when that argument was omitted.

| Argument | Mirror | Default |
|---|---|---|
| `--lighting-full-effort` | `GENOS_LIGHTING_FULL_EFFORT=1` | off |
| `--lighting-sun on\|off` | `GENOS_LIGHTING_SUN` | on |
| `--lighting-sun-bands N` (1..64) | `GENOS_LIGHTING_SUN_BANDS` | 8 |
| `--lighting-catch-up N` (0..16) | `GENOS_LIGHTING_CATCH_UP` | 4 |
| `--lighting-debug` | `GENOS_LIGHTING_DEBUG` | off |

`--lighting-debug` reads the GPU counters. It is not an fps run. While it is on, `./genos-debug/sun/catch.log` appends `frame N catch C catch_left L` for each frame, and the first write of the process truncates that file.

`genos-stress` and `genos-camera` accept the same flags. Preset, bounces, ambient, gpu times, and the unfilled flags are unchanged.

## Deviation

Schied et al., Spatiotemporal Variance-Guided Filtering, HPG 2017, and Wright et al., Lumen, SIGGRAPH 2022, recompute every pixel every frame. This step does not. Direct light does. The four catch-up frames also depart from S69C's halfway relight.

Not in this change: a sparse irradiance-delta layer, an a-trous stamp of that layer, a settled compose that only reads, an analytic first bounce, and collapsing a still scene onto one value. The confirmed-visibility shade that keyed off a cell bit stays out.

## Networking

- Replicated state: none. Irradiance caches are not replicated.
- Authority: the machine that draws owns the caches.
- Prediction / rollback: a rollback drops the caches. The next frame is a full refresh.
- Determinism: a settled picture may be stale. It is not bit-identical across machines.
- Headless server: it runs no lighting unless it is scoring a picture.

## Lavapipe

The check is that the shader bit turns on for a small step, stays off for a frozen sun and for full effort, that the band id changes, and that catch-up lasts four frames. Lavapipe does not support a millisecond claim.

## Owner GPU

Windowed 1920×1080, measured on a 4070-class GPU at 17dc9cd1. Compile and exit were clean. Moving compose was already about 3.0 ms on gi-v2; this branch did not buy that, and the 55.7 ms compose is not this cut's to claim. Still and moving times matched the default because those scenes move boxes or point lamps, so they never take the sun path. Full effort costs the same there for the same reason.

With GI and a moving sun the path did engage (`host_frames` 269/300, `bands` 8) and compose rose 2.78 ms to 3.17 ms, total 20.2 ms to 20.85 ms. The extra 0.39 ms was a second full-grid dispatch inside the compose timestamp. The first dispatch was the dirty-block list. A moving sun's probe row and page column mark most of that list, so it was already close to a full probe walk, and the second dispatch wrote every pixel again. A full compose that walks every pixel was the cheaper of the two.

That second dispatch is gone. One dispatch writes this frame's direct on every pixel and walks probes on the band: one probe row in eight, plus one page column. On the split above, the full walk is about 2.4 ms and a full write without the walk is about 0.39 ms. A quarter of the pixels still walking is about 1 ms if that split holds. That is an estimate from those two numbers, not a new measurement. Lavapipe cannot confirm it. Direct still retraces every pixel while the step is on, so the shadow is not a frame behind. That cost is outside the compose timestamp.

If the retest's compose time is not under the full-effort compose time, the sun step is slower than the pass it replaces and it should not merge.

`--lighting-full-effort` does turn the path off. The counter is written on that run even when `host_frames` stays 0. Expect `full_effort 1`, `compose full`, `sun 0`, `dispatches 1`. The sun-on run expects `compose sun`, `sun 1`, `dispatches 1`. Do not read a time off lavapipe.

```
GENOS_GPU_TIMES=1 cargo run --release -p genos-stress -- --still-boxes --dynamic 0 --size 1920x1080 --frames 120 --no-panel
GENOS_GPU_TIMES=1 cargo run --release -p genos-stress -- --still-boxes --dynamic 0 --size 1920x1080 --frames 120 --no-panel --lighting-full-effort
```

The corner lamp stops after 12 settle draws and 8 steps (`--lamp-stop`). Catch-up is the next four held draws. `catch.log` is truncated at the start of the process. Count the `catch 1` lines. There should be four.

```
GENOS_GPU_TIMES=1 cargo run --release -p genos-stress -- --scene examples/stress/scenes/bounce_corner.rhai --lamp-stop --lighting-debug --freeze-sun --eye 0.8 1.6 0.8 -0.785 0.1 --size 1920x1080 --frames 40 --no-panel
```

Moving boxes or point lamps force the full compose. That run is the control, not the sun-on case.
