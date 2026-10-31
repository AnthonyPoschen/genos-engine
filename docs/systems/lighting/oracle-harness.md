# Lighting harness and oracle

Stage A of [compute-rethink.md](compute-rethink.md). It gives every later
lighting stage one fixed set of scenes and numbers to compare against master.

## Run it

```sh
make lighting-oracle                 # quick profile, out in genos-debug/oracle
make lighting-oracle PROFILE=full    # every frame traced, adds sun x1
scripts/lighting/oracle_run.sh quick /tmp/oracle   # same, explicit
make lighting-oracle PROFILE=lamps   # many-lamp spin and switch only
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
| `lamps-spin` | `lamps` profile: `spin` with 32 lamps in the hall section, a quarter of them moving | 240 |
| `lamps-switch` | `lamps` profile: half of the 32 lamps lit for 10 s, then the other half turns on in one frame, a quarter moving | 8 + 120 |

The main scene has 5 point lamps. Every pixel then has at most 5 lit lamps and
shades the exact sum (`LAMP_PICK_EXACT_LIT` is 8), so `spin` and `switch` never
grade the random lamp path. The `lamps` profile exists to grade that path. Its
script logs `LAMP_COVER`, the number of lamps whose reach holds the subject.
That number is an upper bound on `point_lit`. The script fails if it is 8 or
less.

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
* `grid_excess_worst`, `grid_excess_frame`, `grid_excess_longest_run`: for each
  frame step that has a reference at both ends, the live grid score minus the
  reference's grid score for the same step, clamped at 0. Real shadow motion
  scores about the same in both and cancels, so what's left is the checkerboard
  the live cache adds. Needs per-frame references:
  `ORACLE_REF_EVERY_FRAME="sun-x100" ORACLE_ONLY="sun-x100" scripts/lighting/oracle_run.sh quick OUT`
  (sun-x100 is 73 traces, about 40 min on lavapipe).

**Gate note (31 Oct 2026, pending Anthony):** at 160x90 and x100 sun the
reference path tracer itself scores `grid_worst` 0.45, because one shadow edge
crossing an 8x8 tile decides the score (PR #98 diagnosis). The ADR 0022 F1 gate
(sun-x100 `grid_worst` < 0.567, target <= 0.15) is proposed to move to
`grid_excess_worst`, recalibrated from base. Until Anthony decides, the gate
number is unchanged and `grid_worst` stays reported.
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
