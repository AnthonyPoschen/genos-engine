# Scripted verdicts

Agents were re-judging the same GPU pictures and frame times in prose (about 33
hand-judged verify runs on 14 Oct). A verdict script makes that judgement once.
The agent reads one line. The pictures, diffs, and numbers stay on disk.

These checks are on **probation**. Review a verdict against its evidence
directory before trusting it. A miss, a false pass, or a false fail becomes a
test in `scripts/verdict/tests/test_misses.py` before the threshold changes.
A check graduates only after a run of correct verdicts.

## Contract

Every script prints one line and nothing else on stdout:

```text
PASS|FAIL|UNSURE|ERROR <short reason> evidence=<dir>
```

| Exit | Meaning |
|---|---|
| 0 | PASS |
| 1 | FAIL |
| 2 | UNSURE. Read the evidence. Ask a person only if that still does not decide. |
| 3 | ERROR. The check itself broke. Not a product fail. |

Evidence (screenshots, diff images, CSV, logs) goes to
`./genos-debug/verdict/<kind>/<sha>/`, or to `--out`. Both are resolved to an
absolute path from the caller’s cwd before anything runs. Capture paths passed
to the app are absolute too. A relative path would be joined onto the app’s
shots directory and the picture would land somewhere else. The line is also
`verdict.txt` in that directory. Stdout does not dump the evidence.

Intent comes from the command line. `GENOS_VERDICT_OUT`, `GENOS_VERDICT_DISPLAY`,
`GENOS_VERDICT_SHA`, `GENOS_VERDICT_CACHE_DIR`, `GENOS_VERDICT_BASE`, and
`GENOS_VERDICT_HEAD` are mirrors: each is read only when its flag was omitted.
A flag wins when both are set.

`--dry-run` checks the arguments, writes `args.txt`, and exits 0 with
`PASS dry-run`. It does not launch a window.

A window is never placed on the session compositor. A script that draws
requires `--display` (a Wayland display name). It sets the child
`WAYLAND_DISPLAY` to that name and drops the inherited session display.
`--display` is added to the argument list only when `binary --help` documents
the flag. A base from before that flag, including gi-v2, rejects it, so the
display stays in the environment and the caller places the window. The binary
opens with `--proof` or `--headless` (the proof window floats and does not
take focus). Quit is MCP `quit`, then SIGTERM and a wait. The scripts do not
send SIGKILL.

`./genos-debug/` is gitignored. Nothing is written under `$HOME`.

## Precheck

`scripts/verdict/precheck.py` runs before anything uses the GPU.

```sh
python3 scripts/verdict/precheck.py --base gi-v2 --key noise-floor --display wayland-1
```

1. `git merge-tree --write-tree` against `--base`. A conflict is `FAIL`.
2. GPU and lock probe. Another `genos-stress` process, a live pid in
   `./genos-debug/verdict/gpu.lock`, or a GPU busier than `--gpu-busy-percent`
   (default 50) is `UNSURE busy`. An unreadable sensor is recorded and is not
   itself busy.
3. Verdict cache, keyed on the commit sha plus the hash of `--key` and
   `--input` files. A hit reprints that verdict and skips the rerun. The
   judging scripts write the cache. Precheck only reads it, and only after
   the merge and the busy probe are clear.

`--process-file` and `--gpu-util-file` are fixtures for the probe. The live
probe reads `/proc` and, when present, `gpu_busy_percent` or `nvidia-smi`.

## Noise floor

`scripts/verdict/noise-floor.py` repeats one unchanged commit `--repeats` times.
It writes `repeats.csv`, `numbers.csv`, and `spread.json` (min, max, mean, and
the spread of frame time, plus picture spread when captures exist).

A cold start that does not settle within `--settle-frames` (default 180) is
`FAIL`. One sample is `UNSURE`. Two settled repeats are `PASS` and the spread
is the noise floor later scripts compare against.

```sh
python3 scripts/verdict/noise-floor.py \
  --base gi-v2 --repeats 3 --scene stress --checkpoint hall \
  --display wayland-1
```

A live run calls precheck first. It launches `genos-lighting --headless`.
The Wayland display is `WAYLAND_DISPLAY`, and `--display` is passed only when
that binary's help lists it. It posts `wait_settled`, `timing`, and `capture`
with `"ui": false`, then MCP `quit` or SIGTERM. `--fixture DIR` scores a saved
`repeats.csv` and does not open a window.

Picture error is mean absolute channel error, blurred relative luminance, and
the in-tree FLIP (the same departures as `genos-lighting-ref`, not NVIDIA's
`flip` tool).

## A/B verdict

`scripts/verdict/ab-verdict.py BASE HEAD` builds both commits when `--live`
is set. A live run calls precheck first, so a merge conflict, a busy GPU, or
a cache hit never starts a build. When `--noise-floor` is omitted it then
runs `noise-floor` on the BASE binary (3 repeats of stress/hall unless
`--scene`, `--checkpoint`, and `--repeats` say otherwise) and uses that
spread. HEAD is built after that floor exists. Each catalog view is compared
with it. Frame time uses `head - base`. A delta at or under the spread
passes. A delta under twice the spread (`noise_multiple` in
`crates/lighting-ref/lighting-ref.conf`) is `UNSURE`. Past that is `FAIL`.
Picture mean absolute error, FLIP, and blurred relative luminance use the
same rule against their spreads. `PASS` means no view is slower beyond the
noise and the pictures match.

Stdout is the overall line. `scenes.csv` is one verdict per scene.
`views.csv` is one verdict per checkpoint. A measured floor is
`noise-floor/spread.json` in the evidence directory. `--fixture` scores a
saved `views.csv` and `spread.json` and does not build.

```sh
python3 scripts/verdict/ab-verdict.py gi-v2 HEAD --display wayland-1 --live
```

## Checkpoint catalog

`scripts/verdict/checkpoint-catalog.py` scores blur and FLIP for each catalog
checkpoint. The fail lines and `human_band` come from
`crates/lighting-ref/lighting-ref.conf`. A value inside the band is `UNSURE`.
One repeat is `UNSURE` (one sample is not a pass). The repeat spread is
`checkpoints.csv`.

Reference images are not written. A golden taken during churn would turn the
next run into noise. The rows are the repeats of this run, not a file under
`./genos-debug/reference/`.

```sh
python3 scripts/verdict/checkpoint-catalog.py --fixture run/ --repeats 3
```

## Fixed pose

`scripts/verdict/pose-diff.py` compares two captures of one checkpoint. The
capture command is `{"ui": false}`: the debug menu, the menu tab, the
frame-rate plate, and tool overlays stay out of the picture. A diff image and
`numbers.csv` (mean absolute error, FLIP, blurred relative luminance) are the
evidence. The blur and FLIP fail lines decide. The band is `UNSURE`. The first
capture, with no previous picture, is `UNSURE`. The previous picture is not a
locked golden.

```sh
python3 scripts/verdict/pose-diff.py --scene stress --checkpoint hall \
  --display wayland-1 --before previous.png --after pose.png
```

## PR watch

`scripts/verdict/pr-watch.py` prints the open PRs whose head sha changed since
the last run. The first run records a baseline and passes, so a watcher does
not wake on history. Later, `PASS no new heads` means stay asleep. `UNSURE new
heads count=N` means wake, and `new-heads.txt` lists `number sha url`. State
is `./genos-debug/verdict/pr-watch/state.json` unless `--state` is set.

```sh
python3 scripts/verdict/pr-watch.py
```
