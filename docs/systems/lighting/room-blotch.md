# Room blotch (ADR 0039 stage 2)

The stage 2 gate is a cache-only blotch mean of at most 0.014 on the room, with 25 cm cells. The gi-check room row is a different number: its coded limit stays 0.023, and a PASS there is not this gate.

`genos-stress room-blotch` scores room-a and room-b with the existing `metrics::blotch`. It does not change that function, the 0.023 limit, the renderer, or the reference estimator.

## What it measures

- Scene: `examples/stress/scenes/room.rhai`. The sun line in that file is commented out.
- Poses: eye (0.664, 1.699, 9.629), pitch -0.12 rad. room-a yaw -0.679. room-b yaw -36 degrees. room-c and the close-up are not operands.
- Picture: 320×180. The grade undoes the display tone curve and uses Rec.709 luminance, the same way gi-check does. The saved PNG is 8-bit. The linear samples stay in the `.ref`.
- Mean of the two blotch scores. At or under 0.014 is `adr-pass`. Above that, with every guard clear, is `physical-fail`. The legacy word (0.023) is printed and is not the verdict.
- Live picture: `cells_only`, `surface_cache.sh_cells` on, `cache_async` off, world cell 0.25 m, world mode 3, `sc_settle` 0. In-frame only. `sh_cells` has no command-line flag; the capture sets `GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS=1` on the child. The argument `--lighting-toggles cells_only` and `--lighting-cache-async off` are the supported controls for the other two. A non-zero `sc_settle` changes the world-mode 3 still-frame wait, so the resolved value must be 0.

## Why this is not the gi-check room script

`scripts/gi_check/room.rhai` also calls compare for room-c and the close-up. A cache miss traces during the capture, so a hit (2 frames after settle) and a miss (the whole trace) are not the same picture. `wait_settled` returning settled false at 900 frames is not an error there, and the script continues. The host scene's `lighting.time_of_day` advances during those frames. The CPU tracer ignores that clock, but `cache_key` hashes the scene Debug text, so the key moves. The key also omits the toggles, `sh_cells`, `cache_async`, the noise target, the seconds, and the spp cap.

This command does not call compare and does not path-trace during the grade. A missing reference stays missing. `wait_settled` must report settled true (the existing quiet-frame check). The same job then draws exactly two more presents and saves the last one at 320×180. `settled_frame` is the settle present. `frame` is the saved present, and the pixels are that present. The settle `frames` count does not include those two. A `status` read and a later `wait` are not that pair: `status` returns while the next present is already open, so the next command is taken one present later. The schedule does not depend on a reference worker.

## Identity

Each operand is `room-a.ref` plus `room-a.json`, and the same for room-b. The json is the identity the grade trusts: scene sha256 of `scenes/room.rhai`, pose bits, 320×180, 64 bounces, batch 64, noise target 0.01, max 262144 spp, the requested seconds stop, CPU analytic, shapes, sun frozen at the stress start hour, and the live-picture flags. The native cache key is recorded and is not the file name. Two different keys can still pass when the json matches. The key does not include the seconds stop, so a 900 s file and a 2400 s file can share a key. The sidecar budget is what the grade matches.

A file is accepted only when the samples are finite and the header noise is finite and at most 0.01. Reaching 262144 spp does not accept a noisier file. The old 4194304 spp reuse does not apply. The `.ref` format does not store seconds. `read_cached` sets `seconds` to 0. `seconds_bits` in the json is the writer's reported duration, kept as provenance. The grade requires that field to be present, finite, and not negative. A zero report is not a budget mismatch, and a positive report is not a file clock read from the operand. `seconds_budget_bits` is the stop this invocation requested. Omitted `--seconds` means 900. The only other accepted request is 2400. A sidecar written at the other stop is `budget-mismatch`. The grade copies that sidecar and does not rewrite it. A non-finite budget field is `nonfinite-reference`. The requested stop is provenance of the request, not a measurement that the process ran that long. Open-floor references (160×90, noise 0.02, the known floor hashes, or 187216 bytes) are a scene mismatch and are not renamed. A numeric `adr-pass` is the blotch mean of the supplied pictures. A flat stand-in can meet it. That is not a physical room acceptance.

## Sun freeze and E11

`--freeze-sun` holds `time_of_day` at the stress start hour (`--time` 0.12, hour `((0.12 * 24) + 6) mod 24`). Sun height reads directional lights only. This room has a point lamp and no sun, so freezing does not change the lamp illumination. It stops the host scene clock from moving the cache key during settle.

E11's 0.014 is the W7 gi-check blotch at 12.5 cm. This grade does not claim that score. The live view is cells-only with SH cells, room-b is a fresh settle in the same window after room-a rather than the post-turn settle, and the capture waits two frames after settled instead of waiting on a reference worker.

## Guards

These are not a physical fail. The first one is the verdict. The json lists all of them. Exit 0 is only `adr-pass`.

| Class | When |
|---|---|
| missing-reference | `.ref` or `.json` absent |
| noisy-reference | finite noise above 0.01 |
| nonfinite-reference | noise or reported seconds is not finite, noise or reported seconds is negative, or a sample is not finite |
| scene-mismatch | wrong scene, size, estimator, hash, or a floor file |
| pose-mismatch | eye, yaw, or pitch bits differ |
| budget-mismatch | bounces, batch, noise target, max spp, spp, or `seconds_budget_bits` differ from this run's `--seconds` (900 when omitted, or 2400) |
| unsettled | settled is not true, including a false result at frame 900 |
| wrong-flags | toggles are not exactly cells_only, sh_cells is off, cache_async is on, the cell is not 0.25 m, world mode is not 3, or sc_settle is not 0 |
| sun-not-frozen | the resolved hour is not the frozen start hour |
| missing-capture | no capture log |
| missing-live | a 320×180 live PNG is absent |
| capture-schedule | the saved present is not the settle present plus 2, the present id is not that saved present, or the live PNG is another size |

A settled true at frame 900 is a real settle on the last frame.

## Commands

```sh
genos-stress room-blotch grade --refs DIR --out ./genos-debug/room-blotch
genos-stress room-blotch grade --refs DIR --out ./genos-debug/room-blotch --capture
genos-stress room-blotch trace --pose room-a --threads 4 --out DIR [--promote DIR]
genos-stress room-blotch trace --pose room-a --threads 4 --seconds 2400 --out DIR [--promote DIR]
```

`--out` defaults to `./genos-debug/room-blotch`. It does not write under `$HOME`. `--refs` is required. Grade never writes a `.ref` and refuses a destination path that contains `reference-cache`. `--threads` wins over `GENOS_REFERENCE_THREADS`; when both are omitted the trace uses 4. `--seconds` is 900 or 2400. Omitted means 900 on grade and on trace. There is no environment variable for it, so a parent environment cannot change the stop. `--promote` copies only after the isolated file passes the same judge, including the same `--seconds`. `GENOS_REFERENCE_GPU=1` is refused.

`--capture` runs one proof window at 640×360 with `--freeze-sun`, `--lighting-toggles cells_only`, and `--lighting-cache-async off`, then grades. The child drops `GENOS_REFERENCE_GPU`, every `GENOS_LIGHTING_*` name, every deprecated `GENOS_GI2_*` alias, `GENOS_GI_DEBUG`, and `GENOS_COMPOSE_SC_WRITES`, then sets `GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS=1`. A parent value in that set does not reach the child. `GENOS_COMPOSE_SC_WRITES` is included because the renderer reads it outside the lighting settings catalogue and it changes surface-cache compose writes. Names outside that set, such as `GENOS_GPU_TIMES` and `GENOS_SCENE_DELTA`, are left as the parent set them.

The trace is one CPU analytic pass: 64 bounces, 320×180 shapes, noise 0.01, max 262144 spp, and the `--seconds` stop (900 unless the command says 2400). It refuses a dirty worktree so the sidecar commit matches the binary. An isolated file is reused only when it passes the judge for that same stop. A 900 sidecar does not pass a 2400 request, and a 2400 sidecar does not pass a 900 request. Reuse does not rewrite `seconds_budget_bits`. Noise above 0.01 stays unaccepted at either stop, including at 262144 spp.

## Output

`verdict.json` holds the class, the mean, the legacy word, the scene sha256, and every guard. The grade also writes the derived reference PNGs, copies of the `.ref` and `.json`, and the capture report when a capture was given. The capture directory keeps `resolved-room-a.txt` and `resolved-room-b.txt`, the full lighting settings text of each pose, including the line `sc_settle=0`. The report log repeats that text on one line. `lifecycle_frames_presented=` in the child log is the present count when the window prints it.

## Networking

Replicated state is none. Authority is the machine that runs the grade. Prediction and rollback are none. The CPU trace is deterministic for a commit, the room scene, and a sample count. A headless server does not run this grade.
