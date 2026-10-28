# Lighting harness and oracle

Stage A of [compute-rethink.md](compute-rethink.md). It gives every later
lighting stage one fixed set of scenes and numbers to compare against master.

## Run it

```sh
make lighting-oracle                 # quick profile, out in genos-debug/oracle
make lighting-oracle PROFILE=full    # every frame traced, adds sun x1
scripts/lighting/oracle_run.sh quick /tmp/oracle   # same, explicit
```

The runner starts a headless Weston on lavapipe (the same setup as the
`noise-gate` CI job), so it needs no GPU. It writes `metrics.json` (compact
JSON, one object per scenario) plus the pictures it scored.

## Scenarios

| Name | What moves | Frames |
| --- | --- | --- |
| `sun-x100`, `sun-x10`, `sun-x1` | Sun, one full day from 05:00 (`sun-x1` in full only) | 73, 721, 7201 |
| `spin` | The box nearest the hall spinning at 2 rad/s, lamps on, at night | 240 |
| `switch` | Dark room at midnight (lamps off for 10 s first), every lamp turns on in one frame | 8 dark + 120 |
| `turn` | Camera turns 90 degrees in one second, then holds | 90 |

The `sun_speed` knob tops out at x64, so the script freezes the sun and steps
`time_of_day` itself, one step per frame. Camera paths and the fixed 1/60 s
step make every run the same. Boxes and lamps are held still unless a scenario
moves them.

The cache fades slowly after lamps go off (about 70% of the lit picture is
still there 60 frames later on lavapipe), which is why `switch` holds the
dark for 600 frames. The oracle pass measures `switch` on one switch event:
the trace pauses time, so it adds no settling.

## Two passes

* **temporal**: each scenario runs without stopping and saves every frame
  (every fourth for `sun-x1`). This feeds the grid metric and the
  unchanged-work counter.
* **oracle**: for each reference frame, rewind HIST = 30 frames, run them
  moving, pause, save the live picture and the depth view, then path-trace the
  same moment. The trace never gives the live cache extra settling time. The
  quick profile traces every 12th frame (every 60th for `sun-x10`, every 480th for `sun-x1`), the full
  profile traces every frame.

## Metrics

* `ref_err_near`: the near-camera-weighted error against the reference,
  `sum(w |live - ref|) / sum(w ref)` in linear luminance, with
  `w = 1 / max(distance, 0.5 m)` from the depth view. Mean and worst frame.
* `ref_err`: the same without the weight.
* `grid_worst`, `grid_longest_run`: the PR #94 checkerboard metric
  (`scripts/lighting/grid_metric.py`, copied unchanged).
* `unchanged`: recomputed-but-unchanged work. `px_share` is the share of 8x8
  tiles whose picture did not change while being recomputed; `refreshes` is
  the mean `cache_entries_refreshed` per frame and `refreshes_unchanged` the
  part that fell on unchanged tiles. This is a screen-space proxy, not a
  per-texel count.
* `switch` only: `frames_to_ref`, the first frame after the switch within
  1.1x of the settled error.
* `lighting_ms_p50_lavapipe`: lavapipe timing, only useful run against run on
  the same machine.

`python3 scripts/lighting/oracle_metrics.py --self-test` runs in `scripts/lint.sh`.

## Specular scenarios (stage S0)

`SPEC=1 make lighting-oracle` also runs `examples/stress/scripts/lighting_oracle_spec.rhai`:
`gloss-sweep`, `gloss-edge`, `gloss-offscreen`, `gloss-switch` and `gloss-band`, as set out in
`specular-v3-design.md`. `oracle_metrics.py` scores them with `spec_err_near` (near error over
pixels whose reference light is at least 20% specular, per roughness row on `gloss-sweep`),
`spec_pop`, `frames_to_ref` on `gloss-switch`, and `spec_ms`. The engine hooks it uses:
`object(#{index, roughness, metallic})` (0 to 1; unknown `object` fields are an error), which
only the reference tracer reads, since the live renderer has no per-solid material yet;
`reference(#{aov: "specular"})`, which also writes `<prefix>-reference-spec.png`, the GGX lobe
at the primary hit of the overridden solids, without fog (such traces skip the reference cache
and gpuref); and the `spec_ms` gi_metrics column, which reads 0 until the live specular pass
(ADR 0024) exists. Without a spec image the specular metrics report `spec_mask: missing`.
