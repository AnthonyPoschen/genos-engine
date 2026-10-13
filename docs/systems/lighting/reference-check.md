# Lighting reference check

The picture check for lighting. `gi-check` stays the scripted gate. This check scores a picture against the CPU path trace and writes a folder a person can read.

## Goal

A lighting change is checked against ground truth for each test scene, instead of being judged by eye alone.

## Intent

- Ground truth is the CPU path tracer in `crates/debug/src/reference.rs`. It does not read the GI caches. Full-effort accumulation is the runtime at its maximum knobs. It is not a second ground truth.
- The candidate is the current build's lighting. `--candidate full-effort` spends those maximum knobs. `GENOS_LIGHTING_FULL_EFFORT` (any value except `0`, `false`, `off`, `no`) still overlays the scene, and it is applied last.
- A value inside the band around a fail line does not auto-pass. One sample per pixel does not auto-pass. Two metrics that disagree, neither a clear fail, need a person. The settle cap is written in the shot note and does not change a picture that already passed its gates. A fail stays a fail. A pass stays a pass. Box-step blur 0.075 and 0.078 is under the pass band and above the default settle line of 0.05, so the cap is reached and the picture gates stay a pass.
- Each checkpoint opens a fresh renderer. Lighting caches from the checkpoint before it are dropped, so the verdict does not depend on catalog order. `--warm` keeps one renderer and those caches, which is the chained run. Room-a after hall started at ratio 1.05 on frame 1. Room-a alone was 0.46 at frame 1, 0.61 at 60, 0.69 at 120, and 0.75 at 180, a fail. Cold-start frames are their own metric. A fresh renderer that crosses the settle line records that frame. One that is still over the line at the cap is a renderer issue and needs a person. The picture at the cap is still scored by the blurred error and the luminance ratio. The cap stays 180.
- The score is the blurred relative error and the mean luminance ratio. Relative error is `|Yc - Yr| / max(Yr, 0.25 * mean(Yr), 0.02)`. 0.25 is what still fails a missing bounce. It is not scaled by the reference noise and it is not a per-scene nudge. At `4c6ca67e` room-a's blurred error was 0.134, inside the band around the 0.12 fail line (0.102–0.138), so it needs a person. That is the accepted verdict. The fail line stays 0.12. Box-step at 0.075 and 0.078 passes. Hall (blur 0.303, ratio error 0.168), hall-moving (0.205 / 0.189), and corner (0.204 / 0.195) stay fails. Hell's blur 0.123 and ratio error 0.090 sit in the band, so hell needs a person.
- FLIP gates only `direct-only`. On a bounced picture it is computed, written in the table, left out of the verdict reasons, and does not change the verdict. The blurred error and the mean luminance ratio decide. At `4c6ca67e`, box-step (FLIP mean 0.462 and 0.445) and hell (FLIP mean 0.539, p99 0.913) failed only on FLIP, while blur and luminance were under the fail line or inside its band. At `1f8b1bcf` those FLIP rows were still listed as the verdict reason. A bounced verdict with FLIP mean 0.462 or 0.445, an in-band FLIP mean, a failing FLIP p99, or a FLIP regression against a baseline is the same verdict as that picture with FLIP cleared. FLIP's edge term treats a GI silhouette as a large error even when the smoothed brightness matches, so it is the wrong gate once bounce light is in the picture. Direct-only is the matched footprint: the primary ray and the renderer both take the pixel centre, antialias is off, and the `0d16c2b` picture read FLIP p99 0.84 with ratio 1.016 and blurred error 0.003. That tail passes at the 0.90 edge floor. A direct-only FLIP p99 above the floor still fails.
- `direct-only/hall` is the normalisation control. The path trace uses zero bounces (direct light at the first hit). The renderer uses bounce count 0 with ambient off, even when the candidate is full effort. The mean luminance ratio should sit near 1. A ratio outside the luminance gate is a direct-light mismatch and stays a fail. Bounce count 0 with ambient left on is a different picture and is not this control. `--bounces 0` on the hall checkpoint is that different picture: the renderer drops bounce light and the reference keeps it. On zanven-pc that was blur 0.550 and ratio error 0.333, a fail. The planted `bounces-zero` picture keeps that fail in the self-test.
- The scene clock is a fixed step, `--fixed-dt`, default 1/60 s. Lamp motion, box motion, and the day clock advance once per frame. Wall time is written in `frames.tsv` and does not choose the pose. Owner mode and lavapipe-small use the same frame indices. Lavapipe-small changes the picture size and the sample count.
- Reports are written in the current directory. The default is `./genos-debug/refcheck/<timestamp>/`. `--out` overrides it. The tool does not write to `$HOME`.
- Reference pictures are `./genos-debug/reference/<commit>/`, unless `--reference-dir` is set. `./genos-debug/` is gitignored.

## How to run

On the owner's GPU:

```sh
make lighting-ref-check BASE=<sha>
```

`BASE` is a commit. The file is `./genos-debug/reference/<sha>/metrics.json`. `lighting-ref generate`, and a check that traces references, writes it from the reference noise: raw error at that noise, blurred error after the same box, luminance ratio error 0. A later check judges regressions against it. A check that scores live pictures replaces the file with those candidate metrics. The absolute gate still runs when the file is missing. Regressions are not judged in that case.

Cloud wiring check, small pictures, one sample per pixel:

```sh
make lighting-ref-check MODE=lavapipe-small
```

The same command without `make`:

```sh
cargo run --release -p genos-stress -- lighting-ref check \
  --mode owner \
  --base <sha> \
  --out <dir>
```

`--mode lavapipe-small` is the small run. When `VK_ICD_FILENAMES` is unset and `/usr/share/vulkan/icd.d/lvp_icd.json` exists, that mode selects lavapipe. `--skip-capture` scores no live frames (the self-test and the path traces still run) and the check fails. `--config <file>` replaces `crates/lighting-ref/lighting-ref.conf`.

Frame control, arguments first. Each name has an environment mirror (`GENOS_LIGHTING_REF_FIXED_DT`, `GENOS_LIGHTING_REF_WARMUP_FRAMES`, `GENOS_LIGHTING_REF_SETTLE_FRAMES`, `GENOS_LIGHTING_REF_SETTLE`, `GENOS_LIGHTING_REF_SETTLE_CAP`, `GENOS_LIGHTING_REF_CAPTURE_FRAMES`, `GENOS_LIGHTING_REF_BOUNCES`, `GENOS_LIGHTING_REF_WARM`). A flag wins over its variable. `GENOS_LIGHTING_REF_WARM` values `0`, `false`, `off`, and `no` leave the fresh renderer.

| Flag | Default | What it does |
|---|---|---|
| `--fixed-dt SECONDS` | 1/60 | Scene seconds added on each frame. |
| `--warmup-frames N` | 0 | Frames held on the starting pose before the approach or the event. |
| `--settle-frames N` |  | Score the Nth frame on the scored pose. |
| `--settle THRESHOLD` | 0.05 | Keep drawing until blurred relative luminance is at or under the line, or the cap. The line is this threshold raised by the checkpoint's reference noise after the blur. |
| `--settle-cap N` | 180 | Frame cap for `--settle`. |
| `--capture-frames LIST` | `event+1,+2,+4,+8` | Frames after an event. `1,2,4,8` is the same list. |
| `--bounces N` | scene default | Renderer bounce count: `0`, `1`, `2`, or `unlimited`. The path trace keeps the profile's bounce count. `direct-only` still forces bounce 0 and ambient off. |
| `--warm` | off | Keep one renderer, and its lighting caches, across checkpoints. The default opens a fresh renderer for each checkpoint. |

`--settle-frames` and `--settle` together are an error. The same is true of the two environment variables when neither flag is set. These flags belong to `lighting-ref check`. They are not lighting settings. The live stress binary's fixed step is `--fixed-dt` and MCP `time`, and manual stepping is `--step` (Space, or MCP `step`).

A still checkpoint warms up, then holds the catalog pose. A moving checkpoint warms up at time 0, steps the lamp and box clocks by `--fixed-dt` until they reach the catalog time, and holds there. The day starts early by that approach and arrives at the catalog day on the scored pose, then holds. An event warms up on the pose before the jump (`box-step-before`'s clocks) and switches on the next frame. `event+1` is that frame. The candidate for the blurred error, the luminance ratio, and FLIP is the settle frame. Flicker uses that frame and the next held frame. Convergence uses the capture offsets, scored after the same blur. `frames.tsv` in the report records frame index, sim time, wall time, `picture_settled`, the raw mean relative luminance, the blurred error, and the luminance ratio for every frame.

Exit status:

| Code | Meaning |
|---|---|
| 0 | Pass. Every requested checkpoint was scored and passed. |
| 1 | Fail. The self-test missed a planted defect, a scored picture failed, or no picture was scored. `--skip-capture` and a missing window exit 1. |
| 2 | Needs a person. A one-sample compare does not take the pass path. A run that scored some checkpoints and missed others is 2 when those scored pictures did not fail. |

The report is `summary.md` and `summary.html`, plus `metrics.json` and `frames.tsv`, and for each scored checkpoint `reference.png`, `candidate.png`, `side-by-side.png`, and `heatmap.png`. The planted-defect report is `self-test/summary.md`.

## Scenes

| Scene | Checkpoint | What it holds |
|---|---|---|
| stress-still | hall | Hall bench at noon, boxes held. Still-camera flicker. |
| stress-still | room-a | Room A at noon. |
| stress-moving | hall-moving | Hall, boxes and a share of the lamps at a moment of motion. |
| stress-moving | box-step-before | Hall just before the boxes jump. |
| stress-moving | box-step | Hall after the boxes jump and hold. Error at +1, +2, +4, and +8 frames. |
| bounce-corner | corner | Inside room A, aimed at the corner the opening lights by bounce. |
| hell-100 | night-hall | Night, 100 lamps, most of them moving. One 25 m section, the stress panel's 100-light case, not the 4×4 building. |
| direct-only | hall | Hall bench. Path trace with zero bounces. Renderer with bounce count 0 and ambient off. The mean luminance ratio is the normalisation control. |
| sponza | atrium | Skipped. No Sponza asset in this repository. |

Owner pictures are 320×180. The path trace runs in batches of 16 samples until the noise estimate is 0.05, 256 samples, or 120 seconds, with 8 bounces. The achieved noise is stored beside each picture (`trace.txt`) and the score uses it. Lavapipe-small is 32×18, 1 sample, 2 bounces, 3 seconds. A stored reference is reused when the picture size matches. A different size traces again into the same commit directory.

## Thresholds

One file: `crates/lighting-ref/lighting-ref.conf`. `fail.*` is the absolute line. `regress.*` is how far a metric may worsen against `BASE`. `human_band` is the fraction around a line that needs a person.

## Defect self-test

`cargo test -p genos-lighting-ref selftest` plants pictures and does not open a window. The check writes the same run under `self-test/`.

| Plant | Verdict | What catches it |
|---|---|---|
| clean copy | pass | every gated metric under its band |
| brightness ×1.4 | fail | mean relative luminance |
| brightness ×1.075 | needs human | inside the band; not a pass |
| held stale frame after the box step | fail | error at +1 and +8 frames |
| 4×4 black block on a lit field | fail | unfilled-pixel share |
| still camera alternating 0.5 and 0.3 | fail | flicker; the average matches, so mean error stays quiet |
| lit half matches, shadow loses the red bounce | fail | dark-region relative luminance |
| bounce count 0 against a bounced picture | fail | blurred relative luminance and the luminance ratio |

A picture verdict of fail in that folder means the checker caught the plant. The self-test itself passes only when each of those verdicts holds.

## Blind spots

Printed in every summary:

- The FLIP number is not NVIDIA's `flip` tool. The paper is Andersson, Nilsson, and Akenine-Möller, "FLIP: A Difference Evaluator for Alternating Images", PACMCGIT 3(2), 2020, doi 10.1145/3406183. The kernel and the color difference differ (`crates/lighting-ref/src/flip.rs`). A disagreement between FLIP and relative luminance is a human review.
- The path tracer does not trace fog, particle cards, or textures.
- Sponza is skipped until an asset is in the repository.
- Unfilled pixels are holes: candidate luminance near black where the reference is lit. The surface-cache set counter is not read.
- Flicker is temporal variance on a still camera. A moving camera is not scored for flicker.
- Convergence is only +1, +2, +4, and +8 frames. A defect that heals at frame 3 and returns at frame 16 is missed.
- Mean error can hide a speck smaller than the p99.
- One sample has no noise estimate, so a small error cannot auto-pass. With two or more samples, raw per-pixel error at or under the stored reference noise is not a fail. FLIP gates only the direct-only picture. A bounced picture reports FLIP, leaves it out of the verdict reasons, and decides on the blurred relative error and the mean luminance ratio. FLIP p99 at or under 0.90 is the direct-only edge floor (measured 0.84 on a matched picture). The reference and the renderer already share the pixel centre, and antialias is off. The denominator is `max(Yr, 0.25 * mean(Yr), 0.02)`. It is not scaled by the reference noise. Room-a's blurred error of 0.134 needs a person. The blurred-error fail line stays 0.12. Hall, hall-moving, and corner stay fails. Hell's blur and ratio sit in the band.
- Hell-100 is one 25 m section with 100 lamps.
- Full effort is not independent of a lighting change.
- Lavapipe is a wiring check. Quality is the owner GPU run.
- GPU frames are not cross-device deterministic. The path trace is deterministic for a given commit, scene, and sample count.
- A held stale frame on a still scene with no box step matches a converged picture and is not caught.
- The direct-only hall is the normalisation control: path trace with zero bounces, renderer with bounce count 0 and ambient off. A mean luminance ratio far from 1 is a direct-light mismatch and stays a fail. Bounce count 0 with ambient left on is not this control.
- Owner mode and lavapipe-small share one frame plan (fixed step, warmup, settle, capture offsets). The settle line is blurred relative luminance, raised by that checkpoint's reference noise. Wall time is logged. The scene pose follows the frame index. `picture_settled` is logged and ignored on the first frame of a pose. The check does not call `wait_settled`. The settle cap is recorded in the shot note. A fail stays a fail. A pass on the picture gates stays a pass. Each checkpoint opens a fresh renderer unless `--warm`. Cold-start frames are a metric. A fresh renderer that does not cross the settle line within the cap is a renderer issue. The cap stays 180.

## Code

- `crates/lighting-ref` (`genos-lighting-ref`): metrics, verdicts, the report, the self-test. No window and no Vulkan.
- `examples/stress/src/lighting_ref/`: scenes, the path-trace driver, live capture, the `check` command.
- `make lighting-ref-check` runs `genos-stress lighting-ref check`.

## Networking

- Replicated state: none. Pictures and metrics stay in the directory of the run.
- Authority: the machine that runs the check.
- Prediction / rollback: none.
- Determinism: the path trace is deterministic for a commit, a scene, and a sample count. Live frames share one index list and one fixed scene step, so a slow machine reaches the same pose as a fast one. The pixels are not cross-device deterministic.
- Headless server: it does not run this check. Lavapipe-small is a local wiring run.
