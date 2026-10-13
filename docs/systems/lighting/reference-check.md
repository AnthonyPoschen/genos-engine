# Lighting reference check

The picture check for lighting. `gi-check` stays the scripted gate. This check scores a picture against the CPU path trace and writes a folder a person can read.

## Goal

A lighting change is checked against ground truth for each test scene, instead of being judged by eye alone.

## Intent

- Ground truth is the CPU path tracer in `crates/debug/src/reference.rs`. It does not read the GI caches. Full-effort accumulation is the runtime at its maximum knobs. It is not a second ground truth.
- The candidate is the current build's lighting. `--candidate full-effort` spends those maximum knobs. `GENOS_LIGHTING_FULL_EFFORT` (any value except `0`, `false`, `off`, `no`) still overlays the scene, and it is applied last.
- A value inside the band around a fail line does not auto-pass. One sample per pixel does not auto-pass. Two metrics that disagree, neither a clear fail, need a person.
- Reports are written in the current directory. The default is `./genos-debug/refcheck/<timestamp>/`. `--out` overrides it. The tool does not write to `$HOME`.
- Reference pictures are `./genos-debug/reference/<commit>/`, unless `--reference-dir` is set. `./genos-debug/` is gitignored.

## How to run

On the owner's GPU:

```sh
make lighting-ref-check BASE=<sha>
```

`BASE` is a commit that already has `./genos-debug/reference/<sha>/metrics.json` from an earlier run. The absolute gate still runs when that file is missing. Regressions are not judged in that case.

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

`--mode lavapipe-small` is the small run. When `VK_ICD_FILENAMES` is unset and `/usr/share/vulkan/icd.d/lvp_icd.json` exists, that mode selects lavapipe. `--skip-capture` scores no live frames (the self-test and the path traces still run). `--config <file>` replaces `crates/lighting-ref/lighting-ref.conf`.

Exit status:

| Code | Meaning |
|---|---|
| 0 | Pass. Lavapipe-small with no live frames also exits 0 when the self-test passed. That run did not score a picture. |
| 1 | Fail. The self-test missed a planted defect, or a scored picture failed. |
| 2 | Needs a person. Owner mode with no live frames is 2, so a missing window is visible. A one-sample compare does not take the pass path. |

The report is `summary.md` and `summary.html`, plus `metrics.json`, and for each scored checkpoint `reference.png`, `candidate.png`, `side-by-side.png`, and `heatmap.png`. The planted-defect report is `self-test/summary.md`.

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
| sponza | atrium | Skipped. No Sponza asset in this repository. |

Owner pictures are 320×180, at least 8 samples per pixel, up to 64, 8 bounces, 20 seconds, noise 0.05. Lavapipe-small is 32×18, 1 sample, 2 bounces, 3 seconds. A stored reference is reused when the picture size matches. A different size traces again into the same commit directory.

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
- One sample has no noise estimate, so a small error cannot auto-pass.
- Hell-100 is one 25 m section with 100 lamps.
- Full effort is not independent of a lighting change.
- Lavapipe is a wiring check. Quality is the owner GPU run.
- GPU frames are not cross-device deterministic. The path trace is deterministic for a given commit, scene, and sample count.
- A held stale frame on a still scene with no box step matches a converged picture and is not caught.
- Owner mode draws until the picture settles or 120 frames. Lavapipe-small does not wait. Neither calls `wait_settled`.

## Code

- `crates/lighting-ref` (`genos-lighting-ref`): metrics, verdicts, the report, the self-test. No window and no Vulkan.
- `examples/stress/src/lighting_ref/`: scenes, the path-trace driver, live capture, the `check` command.
- `make lighting-ref-check` runs `genos-stress lighting-ref check`.

## Networking

- Replicated state: none. Pictures and metrics stay in the directory of the run.
- Authority: the machine that runs the check.
- Prediction / rollback: none.
- Determinism: the path trace is deterministic for a commit, a scene, and a sample count. Live GPU frames are not cross-device deterministic.
- Headless server: it does not run this check. Lavapipe-small is a local wiring run.
