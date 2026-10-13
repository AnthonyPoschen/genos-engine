# Lighting

Main lighting document. Architecture and component notes are below; every experiment (tried, pushed, reverted, rejected, shelved or planned) has its own file in [experiments/](experiments/), indexed in the table. Before trying an idea, check the index: if it was tried, read how and what we found, and whether it is marked for a retest.

Sources for the experiment files: the bisect log `~/.cache/gi2diag/steps.md` and patches in `~/.cache/gi2diag/g78/` (zanven-pc), git history of `gi-v2`, `gi-v2-prefill` (22bcd47f) and tag `gi-v2-checkpoint-2026-10-12` (6b4fe308), the postmortem log, and the GI v2 conversation with Anthony. Figures not in a source are written "not recorded".

## Missing history (as of 2026-10-12)

- Engine chat read from an export covering positions 33-3359 (Sat 10 Oct 11:53 to Mon 12 Oct 22:52). Not read: positions 0-32 and the gaps 80-94, 160-162, 240-248, 320-349, 480-488, 720-728, 800-804, 960-961, 1040-1052, 1200-1208, 1280-1281, 1360-1365, 1440-1447. Tool results were skipped; user messages, messages to Anthony and subagent reports were used. Long reports were read in part, so some figures inside them may be missing.
- Before 10 Oct and Chief-side decisions: from Chief's notes (git, Chief's brief and memory; Chief's executor had no memory search, items marked [uncertain] are kept as such).
- Chat from about 05:40 to 10:35 on 12 Oct (S63-S68 work) is mostly absent from the export; those entries rest on git and the doc.
- S75, S76, S77 detail now comes from the chat reports; S70A-C and S71 have git only.
- Scene, GPU and resolution are recorded only where an experiment file says so.
- Screenshots and videos are referenced by name only, not copied.

## From Chief's notes: decisions and rules

- GI v2 design and ADR 0012 (7d35edd, 10 Oct 03:12; Anthony's answers b7b8ea7 03:27): Lumen-informed but aim to beat Lumen; stop when complete so Anthony can judge; lighting ~200 MB VRAM soft budget (Ultra may exceed); hardware floor newest 80% of the Steam survey, Low tuned for GTX 1060 / RX 580, Vulkan 1.3, 4 GB VRAM minimum; Steam Deck a target but untested; test machines zanven-pc (4070) and his MacBook; mesh SDFs and cards prebuilt at build time (configurable); emissive surfaces required, including texture-driven partial-mesh emission (639ab44).
- v2 design rules: only finished values are published; fixed ray patterns; each ray answered by exactly one system; screen probes plus surface cache; software SDF plus hardware ray queries; world probes for the far field only.
- GI v2 lives on gi-v2; phase-1 commits (13e02fe glTF, a22b1c8 BVH, 977ea71, e5cb9ba SDFs/cards, 84c303d) stay on master; master and gi-v2 compared head to head before any merge. Precomputed transport for master dropped.
- v2 became a separate frame path (1a2229f, ec658c5, ~05:30 10 Oct) because the NVIDIA compiler hung with the tracer inside the existing shaders.
- Proposed, not answered [uncertain status]: rewrites on their own branch with master fixes-only (P7); design doc before a new subsystem (P6). Approved (P9): visual results come with screenshots someone looked at; numbers help bots judge faster. Postmortems go to the shared file (P10).
- zanven-pc etiquette: bench lock ~/genos-bench.lock and split/lock_run.sh, one time-limited window; low GPU priority, short submissions; must not freeze the desktop; exclusive fps windows Fri-Mon 05-10 and Tue-Thu 05-15 AEDT; never touch his input or processes.
- Open (P3, unanswered): per-light layers, a 3-6 bounce slider, three proposed lighting locks [which three uncertain], neural radiance caching as an optional RTX 20+ tier (ruled out as v2's core on 1060-class cards, 9 Oct), weekly paper reading. Also open: Anthony's tail estimate for remaining bounces, last x r/(1-r), kept only if it lowers reference error; what the first-principles research track covers [uncertain].
- Backlog (ge-* todo items) [dates uncertain]: fast settling (100-200 ms, all lights in one trace, GPU timing breakdown, bright corners and box feet); stress scene controls; settings struct and presets; 16-bit float pipeline, HDR10, bloom and glare; per-mesh PBR metal/rough (glTF-compatible) with saving loaders; probe specular and rough reflections; froxel fog, volumetrics and god rays; planar mirrors, portals, water with SSR plus probe fallback; 500+ fps from 4070 timings; smart layering for static lights; box-foot contact strip.

## Current state (2026-10-12)

`gi-v2` at 8c763bb2 (S78P): still 8.21 ms / 105 fps, moving 18.7 ms / 51 fps (RTX 4070, stress scene). Pipeline: two-layer surface cache (layer 1 traced texels with MSME accumulator; layer 2 SVGF a-trous + pull-push, never fed back), world radiance cache (25 cm cells) for ray hits, sparse screen probes with held strata and check rays, Lumen-style texel direct-light cache for GI hits, per-pixel traced direct light reused in quiet frames, A-SVGF same-seed checks gating rays.

## Principles and rules (Anthony)

- Quality first; a change that saves no measurable time is not pushed, and no change may add visible noise or tearing.
- Every step: benchmark still and moving, pop, brightness, VRAM, and a moving-shadow smoke test with screenshots actually looked at (postmortem after S77B).
- Pop-in is fine if the screen fills near-immediately; no display smoothing (S71E/F). Lighter smoothing is dropped for now.
- Camera decides residency and update order only, never texel size (S57).
- Uniform low or uniform high beats a patchwork (S72J); layer 2 never feeds layer 1 (S72P).
- Must scale down to a GTX 1060-class GPU: nothing depends on a minimum ray count (S72S).
- Follow the papers; any departure is flagged as a deviation and listed below.
- Checks run every frame; no "nothing changed" skip without a check (Anthony 16:11, S74A).
- Always spend some budget on spread-out checks so quiet areas that should not be quiet are found.
- Probes must not be woken by the shared averaged field; only by real neighbour change, spread by ring tests with group-test splitting ([WAKE](experiments/WAKE-neighbour-wake-design.md)).
- Pre-fill uses spare base budget only. The S-curve lag bound stays deferred ([DEFER-SCURVE](experiments/DEFER-SCURVE-offscreen-lag-scurve.md)).
- Test rules (10 Oct): every moving test soaks and records a settled frame first, ends soaked, and returns movers to the start; short tests for one situation; a test must fail on the build that shows the problem before it is trusted; hand-validate new checks, then spot-check ~1%.
- GI v1 is the stability floor, not the goal; the target is the path-traced reference (11 Oct).
- GPU-resident design: frequently used data stays on the GPU at fixed locations; the CPU sends only small changes (12 Oct).
- Whole-image consistency beats local sharpness; quality changes only at logical boundaries (12 Oct).
- Lighting VRAM may grow if it cuts per-frame compute; total should stay fixed rather than scale with the scene (12 Oct).
- Never skip lighting on an assumption; verify with an extraordinarily cheap check every frame (12 Oct).

## Deviations from the papers (summary)

- Pixels read surface-cache texels directly (Lumen reads the cache only at ray hits).
- Running-mean / MSME texels to a cap instead of Lumen's 4-frame accumulation.
- IC placement as a trace rate on a power-of-two grid, not records in space; no R_geo; grad E from the filtered field; constants inferred (S72V).
- A-SVGF lambda drives ray counts (A-SVGF has no ray-count rule); checks are measurement only (S74A, S76).
- Shadow visibility shared per 12.5 cm cell for hits, invalidated per lamp range (S74A).
- 1-in-16 push refresh floor (S78B); kept-hit bounce refreshed with its re-trace (S78D); held strata keep exact directions (S78K).
- Going fully quiet (DDGI and Lumen always spend a fixed budget).
- Compose redraws only 8×8 blocks whose inputs changed, and while the camera is held a filtered field stays on screen past S72V10's two-frame stamp (the push idle and a frozen frame do not rewrite it). SVGF and Lumen composite every pixel every frame.
- Guard band of primary rays just outside the frustum (Lumen has no guard band) (S73A).
- Pre-fill spends only spare base budget and never enters the priority histogram or the boost (Lumen always spends a fixed budget) (S73G).
- MSME only once a texel is shown. A never-shown texel accumulates over at most 4 updates, as Lumen radiosity, with the MSME firefly cap (S73D, S73M). On first show the count becomes 1 and the next 15 on-screen sets are a plain mean (`k = 1/(t+1)`), so the pre-fill does not stay anchored. MSME has no such handoff.
- Rotating start on the pre-fill list (S73G).
- Scout feedback claims and stamps LastUsed without the irradiance read, so a scout hit does not seed an empty texel (S73K). The pixel path still seeds.
- Planned: change-scaled ring-test wakes with group splitting (original combination).

## Experiment index

E-numbered entries (E01-E25) come from the Engine chat and cover work with no S-number or chat-only findings and decisions; [history.md](history.md) is the chronological record of Anthony's feedback per build.

| ID | Date | Idea | Result | Commit / patch |
|---|---|---|---|---|
| [E01](experiments/E01-splotch-diagnosis.md) | 2026-10-10 | Splotch diagnosis: which cause makes v2 blotchy (cache resolution, per-texel albedo, probe rays, still-scene a | finding | none (throwaway worktree; report /workspace/shots/gi2diag/SUMMARY.md on the box) |
| [E02](experiments/E02-stored-probe-hits.md) | 2026-10-10 | Probes keep 512 hits (8 rotating strata of 64) and re-light them every frame; no reset on lamp/sun/sky change | pushed | ae829f41 |
| [E03](experiments/E03-per-ray-projection-neighbour-borrow.md) | 2026-10-10 | Project each kept ray along its own direction; moving/uncovered probes borrow strata from last frame's 5x5 nei | pushed | 52712f9b, 0fb4294e |
| [E04](experiments/E04-lumen-probe-filter.md) | 2026-10-10 | Lumen-style direction-space probe neighbour filter, plus quick/mid/full gi-check tiers | pushed | 1c3d6d1f, a6432375, 46550f1 |
| [E05](experiments/E05-kept-ray-retrace-on-movers.md) | 2026-10-10 | When a moving box crosses a kept ray, re-trace it in the same frame instead of dropping it | pushed | defa10af |
| [E06](experiments/E06-contact-term.md) | 2026-10-10 | Full-resolution short-range contact term (Lumen-style) | rejected | not pushed |
| [E07](experiments/E07-hex-probe-grid.md) | 2026-10-10 | Hex-offset (triangular lattice) probe grid | parked (Anthony) | none |
| [E08](experiments/E08-turn-fill-and-cell-offset.md) | 2026-10-10 | Turn swim fix: trace missing strata from the probe in the same frame; pattern cell looked up half a cell in fr | pushed | 7ebb886, e14317f, edffa81 |
| [E09](experiments/E09-room-swim-spin-fixes.md) | 2026-10-11 | Room swim / spinning boxes: filter rounds 2/4/8 probes apart, 64 rays per light-cache patch; strata traced fro | pushed | b5fa8eb7, 1b1101e2, 839d4495 |
| [E10](experiments/E10-gpu-reference.md) | 2026-10-11 | GPU path-traced reference on hardware ray queries (independent of the engine) | pushed | 5bb08892, 86e90392, 6ce7fd48 |
| [E11](experiments/E11-surface-cache-stand-in.md) | 2026-10-11 | Stand-in test: pixels read a converged world light cache (world=1) to decide whether a surface cache is worth  | finding (go) | none (experiment runs W6/W7) |
| [E12](experiments/E12-flag-transport-cap.md) | 2026-10-11 | Bug: SC flags were capped at 127 on the Rust side (and lost precision in head w) | fixed (S19, 8526e44e) | 8526e44e |
| [E13](experiments/E13-adaptive-blend.md) | 2026-10-11 | World-pinned adaptive blur sized by noise, capped for near/large surfaces | on hold (Anthony), then judged unnecessary | none |
| [E14](experiments/E14-item1-change-aware-cells.md) | 2026-10-11 | Survey item 1: GIBS change-aware cells (dormant cells fire few change-detection rays) | pushed (flag 1048576, see S26-S41) | 6ce97ac6..d8101b09 |
| [E15](experiments/E15-item4-movers-gpu.md) | 2026-10-11 | Item 4: object-local texels for moving/spinning boxes | pushed as default with ba9adca0; targets still failing | f13d56ba, ba9adca0 |
| [E16](experiments/E16-few-rays-gibs-lumen-bundle.md) | 2026-10-12 | Lumen update order + GIBS extra rays + few rays per relight built together (S58A, S59C), then settle fixes (S5 | pushed | a4a7b23d, e7803342, eedd6671, c4c787d1 |
| [E17](experiments/E17-wrc-seeding-prefill.md) | 2026-10-12 | World space radiance cache first draft: running average of 32 rounds, scout rays around the camera pre-fill ce | pushed | aa1b245e |
| [E18](experiments/E18-denoise-perceptual-stop.md) | 2026-10-12 | Variance-guided surface-space denoise plus perceptual stop rule (S62A) | pushed, later superseded (S72C reverted the always-on version) | 052fba94 |
| [E19](experiments/E19-rate-based-scheduling-design.md) | 2026-10-12 | Rate-based updates: each surface's rate follows how much its light changes; budget is a ceiling, not a target | pushed (S64-S65) | 70d9017f..b5a72df4 |
| [E20](experiments/E20-sun-sky-share.md) | 2026-10-12 | Cache keeps sun/sky share separate from lamp light; sun/sky change rescales it at once (S63A); night->day blac | pushed | 0f288334, bfdb8900, 27afd41e |
| [E21](experiments/E21-perf-transfer-audit.md) | 2026-10-12 | Code audit: is CPU<->GPU transfer the bottleneck? | finding (no) | none |
| [E22](experiments/E22-s69c-kept-hit-relight.md) | 2026-10-12 | Kept probe hits relit only on their stratum's frame plus one rotating stratum (S69C) | pushed | 2f35eaa2 |
| [E23](experiments/E23-cost-waste-analysis.md) | 2026-10-12 | Profiling at the checkpoint: how much traced work changes the picture | finding | none (worktree ~/.cache/ge-perf at 6b4fe308) |
| [E24](experiments/E24-s72-fill-speed-direction.md) | 2026-10-12 | Fill-speed direction for S72: sparse fill, reuse, denoise, then regions refine together | decision | S72A-S72S |
| [E25](experiments/E25-s77-perf-steps-feedback.md) | 2026-10-12 | S76B-S77 one-at-a-time perf steps and Anthony's verdicts | record | dbb4777a..9e242987 |
| [P6](experiments/P6-probe-path.md) | 2026-10-10/11 | Screen probes (16 px tiles, 64 rays/frame in 8 strata), kept hits re-shaded every frame, Lumen-style probe fil | pushed (later default off; `GENOS_LIGHTING_WORLD=0`) | 6ce7fd48 and earlier (ae829f41, a6432375, b5fa8eb7, 1b1101e2) |
| [S26-S41](experiments/S26-S41-surface-cache-flags.md) | 2026-10-11 | Surface cache: world texels pixels read directly, running mean to cap, flag experiments (239882, 524288 camera | pushed (default from S54K) | 21cb8a41..d8101b09 |
| [S42-S54K](experiments/S42-S54K-movers-object-texels.md) | 2026-10-11/12 | Movers in the surface cache: mover expiry 240 frames, pixels near movers, object-local texels, mover upload fi | pushed | 1e7573a3, f13d56ba, ba9adca0 |
| [S56](experiments/S56-band-crossing-inherit.md) | 2026-10-12 | Texels at a new camera-distance size inherit converged light with full count; 10% hysteresis | superseded by S57 same day | 0624ee53 |
| [S57](experiments/S57-no-camera-distance-sizes.md) | 2026-10-12 | One texel size (12.5 cm pixels, 25 cm hit patches) at every distance | pushed (decision, Anthony) | 1dbbca64 |
| [S58A](experiments/S58A-lumen-update-priority.md) | 2026-10-12 | Lumen LastUsed - LastUpdated priority replaces round-robin sweep | pushed | a4a7b23d |
| [S59](experiments/S59-few-rays-gibs-settle.md) | 2026-10-12 | Few rays per relight with GIBS extra rays; higher settle bar (64 sets, 0.15 sd) | pushed | e7803342, eedd6671, c4c787d1 |
| [S60A](experiments/S60A-probe-hit-lamp-picks.md) | 2026-10-12 | Probe hits take at most 2 shadow rays, lamps picked systematically in proportion to unshadowed light | pushed | a2b41c52 |
| [S61-S62](experiments/S61-S62-world-radiance-cache.md) | 2026-10-12 | World radiance cache (WRC, 25 cm cells) for hits; variance-guided denoise; WRC refresh budget; global light re | pushed (S62E reverted) | aa1b245e..4eb10b86 |
| [S63-S65](experiments/S63-S65-sun-share-rate-tiers.md) | 2026-10-12 | Sun/sky share per texel; rate-based scheduling (batch is a ceiling); two tiers (trickle under change 0.173, wh | pushed | 0f288334..b5a72df4 |
| [S66-S69](experiments/S66-S69-anti-lag-boost-relight.md) | 2026-10-12 | Budget boost after turns (S66A), neighbourhood clamp/anti-lag (S67A), read-since-relight slack (S68A), seed we | pushed (S68B reverted) | 782fe38b..2f35eaa2 |
| [S70](experiments/S70-gpu-driven-dispatch.md) | 2026-10-12 | Indirect dispatch for cache rays (S70A), no GPU->CPU counter reads in frame loop (S70B), resident scene buffer | pushed | 8758d467, 76245067, b0675659 |
| [S71E](experiments/S71E-display-ramp.md) | 2026-10-12 | Display ramp to hide convergence pop | reverted (S71F) | 3457f4b2 / revert 4aeeced6 |
| [S72A](experiments/S72A-sparse-fill.md) | 2026-10-12 | Only key texels get rays; others bilinear from keys | pushed (superseded by S72V) | d38d58ec |
| [S72B](experiments/S72B-restir-gi-texel-reuse.md) | 2026-10-12 | ReSTIR GI reservoir reuse on texels | reverted (S72H) | b0e76a3b / 43690446 |
| [S72C](experiments/S72C-denoiser-primary-stop.md) | 2026-10-12 | Denoiser as primary filter + stop on denoised noise | reverted (S72G) | 4c23e3e0 / ca01a371 |
| [S72D](experiments/S72D-coarse-to-fine-fill.md) | 2026-10-12 | Coarse-to-fine sparse fill levels | pushed (superseded by S72V) | e7de24d8 |
| [S72E](experiments/S72E-still-pixel-direct-reuse.md) | 2026-10-12 | Still frames reuse per-pixel direct light | reverted (S72F) | c35f8927 / d53603d1 |
| [S72I](experiments/S72I-confidence-spread.md) | 2026-10-12 | Confidence-weighted sparse spread, edge ratio 1.3 | pushed (superseded) | 597e31dc |
| [S72J](experiments/S72J-region-uniform-pages.md) | 2026-10-12 | 8x8-texel pages refine together | pushed (superseded by S72V) | 6d055836 |
| [S72K](experiments/S72K-smooth-page-bar.md) | 2026-10-12 | Smooth cross-page bar, count-based handover | pushed (superseded) | b802fa3b |
| [S72L](experiments/S72L-wide-anchoring.md) | 2026-10-12 | Wide significance-shrunk anchoring | pushed (superseded) | 2f1123fc |
| [S72M](experiments/S72M-first-reveal-seed.md) | 2026-10-12 | Seed first-reveal texels from probe light | pushed | 3db494ca |
| [S72N](experiments/S72N-seeds-apart-dark-fill.md) | 2026-10-12 | Seeds kept apart from own samples; growing WRC ranked by priority | pushed | cf4487e7 |
| [S72O](experiments/S72O-decorrelation.md) | 2026-10-12 | Per-texel commit timing decorrelated; stored-mean rectification | superseded by S72P | 5cc933d6 |
| [S72P](experiments/S72P-two-layers.md) | 2026-10-12 | Two layers: traced data (layer 1) never shown; shown field (layer 2) rebuilt per round | pushed (core design; layer 2 later SVGF in S72V) | f582015e |
| [S72Q](experiments/S72Q-smooth-stops.md) | 2026-10-12 | Layer-2 transmittance stops | pushed (superseded) | 61df885c |
| [S72R](experiments/S72R-msme-verbatim.md) | 2026-10-12 | GIBS MSME accumulator verbatim | pushed | 6f769a77 |
| [S72S](experiments/S72S-budget-shift-scaling.md) | 2026-10-12 | Budgets scale by GENOS_LIGHTING_SC_BUDGET_SHIFT; MSME warm-up | pushed | ec3d5e74 |
| [S72V](experiments/S72V-svgf-layer2-ic-placement.md) | 2026-10-12 | Layer 2 as SVGF a-trous + Gortler pull-push; irradiance-caching placement (V-V11 fixes) | pushed (checkpoint 6b4fe308) | d23a9218..6b4fe308 |
| [S73A](experiments/S73A-offscreen-prefill-guard-band.md) | 2026-10-12 | Guard-band scout rays claim fine texels just outside the frustum | ported (0dc6c4b); darkening fixed, see [plan](experiments/S73-prefill-root-cause-and-plan.md) | 097d67de, 0dc6c4b |
| [S73B](experiments/S73B-lumen-feedback-prefill.md) | 2026-10-12 | All scout hits claim fine texels and stamp LastUsed | ported (71547c9); darkening fixed, see [plan](experiments/S73-prefill-root-cause-and-plan.md) | 22bcd47f, 71547c9 |
| [S73C](experiments/S73-prefill-root-cause-and-plan.md) | 2026-10-12 | Pre-fill claims stamp LastUsed only; no shown stamp, no reveal burst | pushed | af449f6 |
| [S73D](experiments/S73-prefill-root-cause-and-plan.md) | 2026-10-12 | Never-shown texels accumulate over at most 4 updates; MSME starts once shown | pushed | 6f54914 |
| [S73E](experiments/S73-prefill-root-cause-and-plan.md) | 2026-10-12 | Storage and per-round count for the pre-fill list | pushed | 1071524 |
| [S73F](experiments/S73-prefill-root-cause-and-plan.md) | 2026-10-12 | Never-shown texels wait on the pre-fill list only | pushed | 8ac033e |
| [S73G](experiments/S73-prefill-root-cause-and-plan.md) | 2026-10-12 | Pre-fill takes only spare base budget | pushed | 12387cc |
| [S73I](experiments/S73I-prefill-turn-and-ratios.md) | 2026-10-12 | Turn fill-in: spare budget and an unfinished set; ratio mean is outliers | counters + set completion + lavapipe turn | ffaba86, 15d3e1e |
| [S73K](experiments/S73K-scout-feedback-claim.md) | 2026-10-12 | Scout feedback claims without the irradiance read | lavapipe turn passed: 18335 prefilled, 4547 shown | d4a666f |
| [S73L](experiments/S73L-unfilled-on-screen.md) | 2026-10-13 | Count composed pixels whose surface-cache texel has no finished set | lavapipe turn ended at 0.92% unfilled | 8b78df0 |
| [S73M](experiments/S73M-prefill-handoff.md) | 2026-10-13 | Firefly-cap the pre-fill, then a plain-mean handoff on first show | keeps one set so the texel stays filled | S73M |
| [S74A-cam](experiments/S74A-cam-last-camera-offsets.md) | 2026-10-12 | Fix: last-frame camera read from stale scene-block offsets | pushed | 81c00734 |
| [S74A-asvgf](experiments/S74A-asvgf-asvgf-change-adaptive-rays.md) | 2026-10-12 | A-SVGF same-seed 1-ray checks on 1/9 resolved texels, lambda drives alpha and rays | pushed | 81c00734 |
| [S74A-shadow](experiments/S74A-shadow-static-lamp-shadow-reuse.md) | 2026-10-12 | Hit shadow rays kept per 12.5 cm cell/lamp, invalidated by lamp epoch | pushed | 81c00734 |
| [S74B](experiments/S74B-pixel-direct-reuse-segments.md) | 2026-10-12 | Pixel direct light reused when point unchanged and no change bound touches its shadow segments | reverted (6a0debae) | bf0f2d2b |
| [S75](experiments/S75-lumen-direct-light-cache.md) | 2026-10-12 | Lumen surface-cache direct lighting for GI hits (TDL); A-SVGF SE 0.05 | pushed | d591abfa |
| [S76](experiments/S76-check-is-the-gate.md) | 2026-10-12 | Variance-aware lambda, deferred checks, lambda-proportional rays, hints only nominate | pushed | 78637784 |
| [S76B-D](experiments/S76B-D-confirm-escalate-ring-nominations.md) | 2026-10-12 | TDL and WRC confirm-then-escalate; persistent ring nominations, fair service, global check boost | pushed | dbb4777a |
| [S77A](experiments/S77A-wrc-16ray-check.md) | 2026-10-12 | WRC change refresh as 16-ray check | pushed | c146bf1c |
| [S77B](experiments/S77B-pixel-shadow-reuse.md) | 2026-10-12 | Per-pixel shadow reuse with check and escalate | reverted (9e242987) | cc0b4726 |
| [S77C](experiments/S77C-compact-what-changed.md) | 2026-10-12 | Compact visits unread texels with no check due 1 round in 8 | pushed | 14f1b644 |
| [S77D](experiments/S77D-dirty-page-atrous.md) | 2026-10-12 | Dirty-page a-trous with per-level storage | pushed | bc63443d |
| [S78A](experiments/S78A-list-driven-atrous.md) | 2026-10-12 | List-driven a-trous (indirect dispatch over active texels) | rejected (stash) | git stash in ge-merge |
| [S78B](experiments/S78B-atrous-change-driven.md) | 2026-10-12 | A-trous variance and push driven by change | pushed | a6e2c628 |
| [S78C](experiments/S78C-exact-head-reads.md) | 2026-10-12 | Float +0.5 rounding killed probe reprojection | pushed (bug fix) | c660c0db |
| [S78D](experiments/S78D-kept-hit-halfway-gated.md) | 2026-10-12 | Kept-hit halfway relight only with a reason | pushed | aaf7bb86 |
| [S78E](experiments/S78E-tdl-age-check.md) | 2026-10-12 | TDL past age is checked, not recomputed | pushed | 75810842 |
| [S78F](experiments/S78F-stale-wrc-check.md) | 2026-10-12 | Stale WRC cells take a due check | pushed | 6474a941 |
| [S78G](experiments/S78G-stop-at-cap-trickle.md) | 2026-10-12 | Stop at cap + 64/round trickle for unconverged (G2: stop at cap only) | rejected | g78/S78G.patch |
| [S78H](experiments/S78H-wrc-growth-trickle.md) | 2026-10-12 | WRC growth trickle 1024/round | rejected | not saved |
| [S78I](experiments/S78I-graded-lamp-change.md) | 2026-10-12 | Lamp change graded by its bound vs shown light | rejected | g78/S78I.patch |
| [S78J](experiments/S78J-pixel-direct-quiet-reuse.md) | 2026-10-12 | Per-pixel direct reuse in quiet frames | pushed | 18894551 |
| [S78K](experiments/S78K-held-probe-strata.md) | 2026-10-12 | Held probe strata + 1 check ray/probe | pushed | a6559e5e |
| [S78L](experiments/S78L-noise-aware-tdl-check.md) | 2026-10-12 | Noise-aware TDL check (sigma EMA) | rejected | g78/S78L.patch |
| [S78M](experiments/S78M-one-tdl-owner.md) | 2026-10-12 | One TDL owner per texel per frame | rejected | not saved |
| [S78N](experiments/S78N-pixel-reuse-with-changes.md) | 2026-10-12 | Pixel reuse in frames with changes (segment test) + 1/16 verify | rejected | g78/S78N.patch |
| [S78O](experiments/S78O-kept-hit-texel-index.md) | 2026-10-12 | Kept hits remember their texel | rejected | g78/S78O.patch |
| [S78P](experiments/S78P-gather-shared-memory.md) | 2026-10-12 | Gather bins in shared memory, one workgroup per probe | pushed | 8c763bb2 |
| [S78Q](experiments/S78Q-restir-di-pixels.md) | 2026-10-12 | Weighted lamp picking per pixel (ReSTIR DI): Q fused spatial, Q2 temporal-only, Q3 +RIS, Q4 RIS 8 cand + tempo | shelved (Anthony) | g78/S78Q4.patch |
| [S78R](experiments/S78R-held-probe-gather-skip.md) | 2026-10-12 | Gather skip for held probes | rejected | g78/S78R.patch |
| [S78S](experiments/S78S-counters-still-churn.md) | 2026-10-12 | Debug counters: what keeps still probes awake | pushed (debug tooling) + finding | S78S |
| [S78U](experiments/S78U-check-ray-noise-test.md) | 2026-10-12 | Held check ray is a test, not a rewrite | pushed | S78U |
| [S78V](experiments/S78V-se-stop-rule.md) | 2026-10-12 | SE stop rule for texels | rejected (noise) | g78/S78V.patch |
| [S78W](experiments/S78W-stats-device-memory.md) | 2026-10-12 | Shader stats in device memory; per-dispatch GPU times | pushed | S78W |
| [S78R3](experiments/S78R3-gather-hold-on.md) | 2026-10-12 | Gather hold on (re-test) | pushed (rough; idle A/B 6am) | S78R3 |
| [S78Y](experiments/S78Y-quiet-filter.md) | 2026-10-12 | Quiet probe filter | pushed (rough; idle A/B 6am) | S78Y |
| [S78X](experiments/S78X-atrous-work-lists.md) | 2026-10-13 | A-trous levels run dirty work lists | pushed (rough; idle A/B) | S78X |
| [S78X2](experiments/S78X2-atrous-dense-fallback.md) | 2026-10-12 | Dense a-trous when the dirty fraction is high | pushed | S78X2 |
| [S78Z](experiments/S78Z-verify-ring-wakes.md) | 2026-10-13 | Probe verification + impact-tested ring wakes | not pushed (no fix; postmortem) | g78/S78Z-verify-ring-wakes.patch |
| [S78T2](experiments/S78T2-texel-verify.md) | 2026-10-13 | Texel verification: rotating A-SVGF checks + ring wakes | not pushed (no healing; postmortem) | g78/S78T2-texel-verify.patch |
| [S78S2](experiments/S78S2-settle-floor-nbhd.md) | 2026-10-13 | Settle floor 16 sets @95% + neighbourhood re-check (A-SVGF adaptation) | not pushed (offset unchanged; postmortem) | g78/S78S2-settle-16-nbhd.patch |
| [S78B2](experiments/S78B2-msme-time-floor.md) | 2026-10-13 | Time-based MSME catch-up floor | not pushed (no healing; postmortem) | g78/S78B2-msme-time-floor.patch |
| [S78H2](experiments/S78H2-sharc-read-16.md) | 2026-10-13 | SHaRC read rule 16 sets for fine short hits | not pushed (no effect; postmortem) | g78/S78H2-sharc-read-16.patch |
| [WAKE](experiments/WAKE-neighbour-wake-design.md) | 2026-10-12 | Anthony's wake design: no wakes from shared field; real change wakes neighbours by coarse-to-fine ring tests w | planned (approved by Anthony) | none yet |
| [DEFER-SCURVE](experiments/DEFER-SCURVE-offscreen-lag-scurve.md) | 2026-10-12 | S-curve allowed quality lag for off-screen pages by priority | deferred | none |
| [DUMP](experiments/DUMP-gpu-dump-tool.md) | 2026-10-12 | Debug dump of GPU data from two consecutive frames per component (direct, bounce, coarse/fine WRC, probe resul | in progress | none yet |
| [RAYCEIL](experiments/RAYCEIL-world-cache-need-ray-ceilings.md) | 2026-10-12 | World-cache cells stop at IC error bound instead of fixed 128 samples | queued | none |
| [QUIET](experiments/QUIET-quiet-paths-compose-filter-compact.md) | 2026-10-12 | Compose redraws 8×8 blocks whose direct, probe SH, or filtered field changed | pushed (compose only; perf on the owner GPU) | this commit |
| [LSMOOTH](experiments/LSMOOTH-lighter-smoothing.md) | 2026-10-12 | Lighter smoothing | dropped (Anthony) | none |

# Architecture and component notes (history kept as written)

How GI v2, the lighting engine, works: direct light, shadows, global illumination, the reference it is checked against, and the checks. Each component lists our method, the published method it compares to (Lumen and papers, with links), how and why ours differs, current scores and open issues. The plan and budgets are in [GI v2 design](../gi-v2-design.md); the gate is in [Debugging](../debugging.md). Update this file whenever a method changes, in the same commit as the code.

Scores are gi-check numbers (linear luminance, share of the light) against the GPU path-traced references, at the commit named in each section. "P6" is the probe path (6ce7fd4: screen probes, light cache in push mode, `GENOS_LIGHTING_WORLD=0`); it was the gi-v2 default until S54K. Default now (no env, S54K): GI v2 with the surface cache, `GENOS_LIGHTING_WORLD=3`, `GENOS_LIGHTING_WORLD_CAP=64`, `GENOS_LIGHTING_SC_FLAGS=6007050`, `GENOS_LIGHTING_SC_LOCAL=1`; P6 back with `GENOS_LIGHTING_WORLD=0`. GI v1 was removed on 2026-10-12 and there is no lighting-selection switch: plain `make stress` / `make run` run this lighting with the tuned defaults (see "GI v1 removed" below). "SC" is the surface cache (`GENOS_LIGHTING_WORLD=3`, branch `surface-cache`, flags in `GENOS_LIGHTING_SC_FLAGS`).

## Names (2026-10-12)

The lighting has one name scheme, `lighting`, everywhere; "GI v2" survives only in history and design notes. The old `gi2` names map one to one:

| Kind | Now | Was |
|---|---|---|
| Rust module | `crates/render/src/lighting_gpu.rs` (included into `gpu.rs`) | `gi2_gpu.rs` |
| Shaders | `lighting_place`, `lighting_trace`, `lighting_direct`, `lighting_gather`, `lighting_filter`, `lighting_compact`, `lighting_cache`, `lighting_compose`, `lighting_check` (`.comp`); `lighting_common`, `lighting_pack`, `lighting_cache`, `lighting_cache_slots` (`.glsl`) | `gi2_*`; `lighting_direct.comp` was `gi2_light.comp` |
| Rust items | `Lighting` (the GPU state, `Gpu::lighting`), `lighting_*` functions, `LIGHTING_*` constants | `Gi2`, `gi2_*`, `GI2_*` |
| GLSL items | `lighting_*` functions, `LIGHTING_*` constants and offsets | `gi2_*`, `GI2_*` |
| Renderer API | `lighting_still_frames`, `lighting_probe_samples`, `settle_stats`, `picture_settled` | `gi_v2_*`, `tier_stats` |
| Log lines | `LIGHTING_MS`, `lightingsc` (surface cache trace), `lighting:` | `GI2_MS`, `gi2sc`, `gi2:` |
| Test | `crates/render/tests/lighting_frame.rs` | `gi2_frame.rs` |
| Env | `GENOS_LIGHTING_*` | `GENOS_GI2_*`; `GENOS_LIGHTING_DEBUG` was `GENOS_GI_DEBUG` |

`gi-check` keeps its name (it is the tool), as does `GENOS_GI_CHECK_BOUNCES`.

No environment variable is needed: `make stress` and `make run` run the tuned defaults baked into the code (`LIGHTING_DEFAULT_WORLD` = 3, the surface cache; `LIGHTING_DEFAULT_SC_FLAGS` = 6007050). There is no lighting-selection switch (`GENOS_GI` is gone). The remaining variables are optional debug and tuning knobs:

- Debug and traces: `GENOS_LIGHTING_DEBUG` (per-round counters; reads GPU counters back, so not for fps runs), `GENOS_LIGHTING_SC_TRACE` (surface cache claims and relights per readback), `GENOS_SCENE_DELTA` (resident scene and mesh field delta runs). The dump, the counters, the MCP commands, the stuck and pop checks, the headless path, and the overlays are inventoried in [the debug standard](debug-standard.md). That file is the reference other systems match.
- Tuning (A/B only): `GENOS_LIGHTING_WORLD` (0 probe path P6, 3 surface cache), `GENOS_LIGHTING_SC_FLAGS`, `GENOS_LIGHTING_SC_LOCAL`, `GENOS_LIGHTING_SC_SETTLE`, `GENOS_LIGHTING_WORLD_CAP`, `GENOS_LIGHTING_TILE`, `GENOS_LIGHTING_CACHE_HZ`, `GENOS_LIGHTING_CACHE_PICKS`, `GENOS_LIGHTING_FILL`, `GENOS_LIGHTING_YOUNG_ONLY`, `GENOS_LIGHTING_MOVED_DROP`, `GENOS_LIGHTING_URGENT`, `GENOS_LIGHTING_BORROW`, `GENOS_LIGHTING_ASYNC` (async compute light cache, off by default), `GENOS_LIGHTING_ATROUS_DIRTY` (a-trous work-list dirty fraction, default 0.25; `0` dense sweep, `1` lists up to the cap).

Older entries below quote the names of their time (`GENOS_GI2_*`); read them through the table.

## Sources

- [W21] D. Wright, "Radiance Caching for Real-time Global Illumination", SIGGRAPH 2021 Advances. https://advances.realtimerendering.com/s2021/index.html
- [W22] Wright, Narkowicz, Kelly, "Lumen: Real-time Global Illumination in Unreal Engine 5", SIGGRAPH 2022 Advances. https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf
- [KN] K. Narkowicz, "Journey to Lumen", 2022. https://knarkowicz.wordpress.com/2022/08/18/journey-to-lumen/
- [DOC] Lumen Technical Details (UE5 docs). https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine
- [SRC] UE 5.4 cvar defaults (mirror). https://indxzero.github.io/ue544cvarwiki/
- [ReSTIR GI] Ouyang et al., "ReSTIR GI: Path Resampling for Real-Time Path Tracing", HPG 2021. https://research.nvidia.com/publication/2021-06_restir-gi-path-resampling-real-time-path-tracing
- [DDGI] Majercik et al., "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields", JCGT 8(2), 2019. https://jcgt.org/published/0008/02/01/
- [SHaRC] NVIDIA, "Spatially Hashed Radiance Cache" (RTXGI 2.x). https://github.com/NVIDIA-RTX/SHARC
- [NRC] Müller et al., "Real-time Neural Radiance Caching for Path Tracing", SIGGRAPH 2021. https://research.nvidia.com/publication/2021-06_real-time-neural-radiance-caching-path-tracing
- [SVGF] Schied et al., "Spatiotemporal Variance-Guided Filtering", HPG 2017. https://research.nvidia.com/publication/2017-07_spatiotemporal-variance-guided-filtering-real-time-reconstruction-path-traced
- [ReBLUR] D. Zhdan, "ReBLUR: A Hierarchical Recurrent Denoiser", Ray Tracing Gems II ch. 49, 2021. https://link.springer.com/chapter/10.1007/978-1-4842-7185-8_49
- [GIBS21] H. Halén et al., "Global Illumination Based on Surfels", SIGGRAPH 2021 Advances. https://advances.realtimerendering.com/s2021/index.html
- [GIBS24] Apers, "Shipping Dynamic GI in Frostbite", SIGGRAPH 2024 Advances. https://advances.realtimerendering.com/s2024/content/EA-GIBS2/Apers_Advances-s2024_Shipping-Dynamic-GI.pdf
- [MSME] T. Stachowiak, multi-scale mean estimator (kajiya renderer, the estimator GIBS cites). https://github.com/EmbarkStudios/kajiya
- [A-SVGF] Schied et al., "Gradient Estimation for Real-Time Adaptive Temporal Filtering", HPG 2018. https://cg.ivd.kit.edu/atf.php
- [NRD] NVIDIA Real-time Denoisers (ReBLUR, ReLAX). https://github.com/NVIDIA-RTX/NRD
- [IC] Křivánek et al., "Radiance Caching for Efficient Global Illumination Computation", IEEE TVCG 2005.
- [LT] Conty Estevez, Kulla, "Importance Sampling of Many Lights with Adaptive Tree Splitting", HPG 2018. https://doi.org/10.1145/3233305

Research notes with the cvar values: `/workspace/research/lumen-digest.md` (agent box).

## Frame overview

One frame, passes in order (`crates/render/src/lighting_gpu.rs`, shaders `lighting_*.comp`); every pass has at most one trace site:

1. `lighting_place`: one screen probe per 16 px tile on the G-buffer.
2. `lighting_trace`: this frame's stratum of each probe's rays, re-traces of kept rays that cross a change, fills for newly seen surfaces, and the light cache's rays (async queue).
3. `lighting_light`: direct light with traced shadows at every pixel and every hit.
4. `lighting_gather`: each probe bins its kept rays by direction.
5. `lighting_filter`: three probe-space filter rounds, then SH.
6. `lighting_compact` / `lighting_cache`: the light cache (or surface cache) relights a batch.
7. `lighting_compose`: pixel = emitted + direct + interpolated probe (or cache) irradiance. The picture stays in the output buffer. A frame whose camera, meshes and lamps match the last one (surface cache on, picture small enough for the block list) dispatches only the 8×8 blocks a producer marked: the direct pass when the surface point, direct light or emission changed; the probe filter when a probe's SH changed; the surface-cache push when a texel's filtered field changed and that texel was read by a block. The list is an indirect dispatch. Nothing marked means no groups. `GENOS_LIGHTING_COMPOSE_ALL=1` forces a full compose. While that camera holds, a filtered field keeps being shown after S72V10's two-frame stamp would have dropped it (see Deviations).

## Scene representation and tracer

- **Ours:** one software tracer over mesh SDF instances (`mesh_field.glsl`): sparse 8³ bricks of byte distances near the surface, one coarse byte per brick elsewhere, sphere-traced in object space after clipping to the instance bounds. All GI v2 rays (probe, cache, shadow) use it.
- **Reference:** Lumen software RT uses per-mesh SDFs near and a global SDF far; hardware RT uses a BVH [W22, KN].
- **Differences:** no far global SDF yet; the design wants world-anchored far bricks instead of camera clipmaps (clipmaps change with the camera). No hardware path yet.
- **Open:** far field (long rays) and hardware RT are not built.

## Direct light and shadows

- **Ours:** `lighting_direct.comp` lights every lamp at every pixel and at every ray hit with one traced shadow ray each, so pixels, kept probe hits and cache hits share one loop and see this frame's light. No shadow maps, no light history.
- **Reference:** Lumen uses virtual shadow maps for pixels and shadow-map lookups plus traced rays for surface-cache texels, up to 8 lights per 8×8 card tile [W22]; MegaLights samples many lights stochastically and denoises. Many-light trees [LT].
- **Differences:** traced shadows match the reference tracer exactly and need no denoiser; cost grows with lamp count (fine for the stress scene, five lamps). The design's deterministic light-tree cut per tile and cached shadow masks are not built.
- **Scores (P6):** direct view matches a `bounces: 0` trace to about 2 %.
- **Open:** light tree for many lamps; area lights for small bright emitters.

## Screen probes (final gather)

- **Ours:** one probe per 16 px tile (smaller below 720p, about 45 rows), placed on the G-buffer at the tile centre or the first of four inner points that has a surface. 64 rays per frame per probe, in 8 strata (frame f traces stratum f mod 8), so a probe sees 512 directions. Pixels take the 4 nearest probes weighted by plane distance and normal (`lighting_compose`).
- **Reference:** Lumen places probes on G-buffer pixels on a 16 px grid, refines adaptively down to 4 px where interpolation fails, jitters placement and directions every frame, 8×8 octahedral rays, BRDF-guided product importance sampling (about 4×), and a 10-frame temporal filter [W21 s.33-52]. Irradiance caching [IC] is the origin of the adaptive refinement.
- **Differences:** fixed strata instead of per-frame jitter (no crawling noise; error is a still pattern). No importance sampling and no adaptive refinement yet.
- **Scores (P6):** settled mean 0.033-0.042 per view (limit 0.043 with GPU refs); room 48 px swim 0.0025 (target 0.0006, MISS); light structure 0.0143 (target 0.010, MISS); close-up 0.0106 PASS.
- **Open:** bounce swims while the camera turns (Anthony, 11:27) because the probe grid slides with the screen; adaptive probes; BRDF importance sampling.

## Kept hits (temporal reuse of visibility, not light)

- **Ours:** a probe keeps the hits of its last 8 strata. Each frame every kept hit is shaded again (`lighting_light`), so no light is carried over: a lamp that moves or switches shows in full next frame. A probe that slid on its surface takes strata from last frame's neighbours within 2.5 spacings on the same plane (parallax-corrected). Kept rays crossing the bounds of a moved object are re-traced; strata nobody traced (newly seen surfaces) are filled from the probe this frame.
- **Reference:** Lumen accumulates *radiance* over 10 frames with per-pixel depth rejection and a fast-update mode for pixels whose rays hit movers [W21 s.98-100]. ReSTIR GI [ReSTIR GI] reuses path samples (visibility and radiance) with reservoir resampling and validation rays. SVGF / ReBLUR [SVGF, ReBLUR] accumulate radiance with variance-guided rejection.
- **Differences:** we reuse visibility only, so light changes have no lag and no ghosting; the cost is fewer effective samples than a 10-frame radiance history while moving. Closest to ReSTIR GI's sample reuse with re-shading, without resampling weights.
- **Open:** importance resampling of kept hits (ReSTIR-style) could raise effective samples.

## Probe filter

- **Ours:** `lighting_filter.comp`, Lumen's recipe: 3 à-trous rounds (neighbours 2, 4, 8 probes apart) over 3×3, same direction, depth-only weight, neighbour ray used only if its hit seen from this probe is within 10° after clamping its distance to our own. Last round projects to SH (bands 0-2, cosine-convolved).
- **Reference:** Lumen: 3 passes of 3×3 neighbours, depth weight, 10° hit angle, hit-distance clamp [W21 s.57-63].
- **Differences:** à-trous spacing (2/4/8) instead of adjacent neighbours, to spread residual noise into blotches wider than the light varies (helps grid redraw and blotch).
- **Scores (P6):** blotch PASS (limit 0.023).

## Light cache (ray-hit irradiance, push mode)

- **Ours (P6 default, `GENOS_LIGHTING_WORLD=0`):** a hashed cache of world patches (1M slots, 16-slot search) keyed by position, axis normal and size (25 cm × 2^level beyond 8 m from the eye). Ray hits read it to get the bounce (multi-bounce by feedback). Each round relights a batch of 16384 patches with 64 rays each, young patches first; each relight replaces the value (no running mean). Patches unread for 1800 frames are freed.
- **Reference:** SHaRC [SHaRC] (hashed world cells, accumulate then resolve); NRC [NRC]; Lumen's surface cache radiosity at hits [W22]; DDGI probes [DDGI].
- **Differences:** replace-on-relight keeps it noisy per patch but unbiased and lag-free; ray hits average over many patches so the noise mostly cancels.
- **Scores:** lights-only return (lampret, limit 0.002): hall 0.0002, room 0.0001 PASS.

## Surface cache (default since S54K, `GENOS_LIGHTING_WORLD=3`)

- **Ours:** the light cache with fine texels that pixels read directly (bilinear over the 4 nearest texels on the pixel's axis plane): one size, 12.5 cm, at every distance (S57; before that 6.25 cm within 2.5 m of the eye, 12.5 cm to 8 m, then 25 cm × 2^level); ray hits read 25 cm patches, also one size at every distance (S57; before, doubling past 8 m). Each texel accumulates 64-ray sets as a running mean up to a cap (64). Texels start from a pooled Gaussian of their lit neighbours and from the lit texel twice the size. Relight budget: one batch of 16384 per round. Best flags 239882: slanted surfaces (|n| not axis-aligned) use the probes; texels read keep relighting at the cap with rotating ray sets (multi-bounce keeps arriving); hit patches replaced each relight (push-style, removes the hit-cache bias).
- **Reference:** Lumen's surface cache: mesh cards fixed by geometry, resident pages sized by distance (5-20 cm), lighting updated within a fixed per-frame budget by priority = LastUsed − LastUpdated (direct 1/32, radiosity 1/64 of the atlas per frame), radiosity accumulated over at most 4 frames, lighting resampled when a page is reallocated, unused pages kept 256 frames [W22, SRC]. In Lumen pixels never read the cache directly; it is only read at ray hits, behind the screen-probe temporal filter.
- **Differences and why:**
  - Pixels read texels directly: this is what removes the screen-probe swim (texels are world-anchored), so texel noise and drift are on screen; Lumen hides cache noise behind the final gather.
  - Running mean of up to 64 sets instead of Lumen's 4-frame accumulation, because the texel is shown directly.
  - Camera only orders updates (flag 524288, S31, testing): once a texel has a set its lit-from point is frozen (no restart when a nearer reader appears), and a texel claimed at a new size (the eye crossed a size ring) starts from the light and count (up to 16 sets) of the lit texel at the next size up or down, as Lumen resamples lighting on realloc.
  - Per-texel estimator (flag 1048576, S32, testing), after GIBS [GIBS21] and Stachowiak's multi-scale mean estimator [MSME]: besides the long-term mean each texel keeps a short-term mean (blend 0.08) and short-term luminance variance. The long-term blend is a plain mean for the first 16 sets, then grows with how many standard deviations the short-term mean sits from it (with an inconsistency term and a variance-based reduction) and falls to 1/256 once they agree, so a converged texel neither drifts nor freezes and still follows real change. Fireflies above the short-term mean + 8σ are clipped. Converged texels whose sets agree (inconsistency < 0.3) go dormant: the sweep relights them one round in 8 to notice change, freeing budget for the rest (GIBS: dormant surfels fire only change-detection rays). Our ray count per relight stays 64; GIBS varies rays per surfel instead.
  - **Decision (S57, Anthony 2026-10-12): no camera-distance texel sizes.** The bands (6.25 cm within 2.5 m, 12.5 cm to 8 m, 25 cm and up past it, and the ray-hit patches doubling past 8 m) are removed. Every band edge was a ring that followed the camera; crossing it re-keyed texels into new cells that started with few samples, which Anthony saw as coarse grain on walls and floors while running that cleared only after he stopped. Inheriting light across the edge (S56, below) softened it but could not remove it: the new cells still converged from the start, and the far band's 25 cm cells were the ray-hit patches, which hold at most 3 one-relight sets (flag 131072). Lumen's cards are placed by geometry and keep their resolution through camera moves (its distance-based page sizing is a residency budget with hysteresis and resampling), and none of the surveyed papers keys cell size to the eye. Detail and effort now come only from the light and the schedule (below); the camera only decides residency (texels unread for 1800 frames are freed) and update order (young texels first).
  - Band crossings (S56, superseded by S57 the same day; Anthony saw a ring where quality dropped as he came closer): the size bands (6.25 cm within 2.5 m, 12.5 cm to 8 m, 25 cm and up past it) stay, but a texel claimed at a new size now inherits the old size's converged light bilinearly at its centre (the 4 nearest texels of the size up, or the 2 x 2 finer texels it covers) with their full sample count (to the cap), not one texel's light and at most 4 sets. Its own sets then blend in by count, so a crossing never shows fewer samples than before, in either direction. Band edges have 10 % hysteresis: inside the margin a pixel keeps reading the size its surface is already lit at. Debug view `texels` colours the size each pixel reads (red 6.25, green 12.5, blue 25 cm, magenta coarser; dim under 4 sets). Approach run (quick): grain 0.0059 -> 0.0052 (worst frame 0.0087 -> 0.0071), recovery 71 -> 49 frames, soaked end against fresh 0.0175 -> 0.0142; trail 0.0372 -> 0.0421 (passes, limit 0.05).
- **What decides history and effort per texel (after the bands went), and where each comes from:**
  - Change-aware history and dormancy, after GIBS [GIBS21] and Stachowiak's multi-scale mean estimator [MSME] (flag 1048576): a texel's long-term mean speeds up only when its short-term mean moves away from it; a converged texel whose sets agree goes dormant and is relit one sweep in 8 to notice change. Not yet: GIBS's variable ray count (more rays for unconverged or high-variance cells); ours is a fixed 64 rays per relight.
  - Relight budget, after Lumen's surface cache update [W22]: a fixed batch of 16384 texels per round, young texels (new, or near a change) first, then the mature ones in turn. Not yet: Lumen's priority LastUsed − LastUpdated (relight what was read most recently and updated longest ago); ours is a round-robin sweep (item 2, the scheduler).
  - Read rule at ray hits, after SHaRC [SHaRC]: a near hit reads only a texel with at least 4 sets and size small against the hit distance; otherwise the 25 cm patch.
  - First light of a new texel, after Lumen's ResampleLighting [SRC] (flag 524288): bilinear from the lit 25 cm patches with their count, short (4 sets) near a change or under changing light.
- **Scores (S41, flags 1812746 = 239882 + 524288 + 1048576, commit after a2fbf3e8):** settled room-a 0.0405 (bias -0.001), corner 0.0421 (+0.009), contact 0.0372 (+0.007), lamp-wall 0.0344 (+0.007), hall 0.0371 (+0.012), close 0.0204 (+0.006). Room: world swim 0.0019 PASS (settled-per-pose 0.0012 = the reference's own), grid redraw 0.0008 PASS, 48 px swim 0.0010 (target 0.0006), blotch 0.0151 PASS, close-up 0.0093 PASS, structure 0.0102 (target 0.010), end 0.0026 FAIL. Still shimmer 0 and 600-frame still drift 0. Turn: arrival 0.0027 / walking 0.0050 PASS, onset 0.0051 PASS, held 0.0050, return 0.0067, end 0.0073 FAIL. Lights-only return: hall 0.0032, room 0.0023 (limit 0.002; push 0.0002). Spin 0.1096 / flicker 0.0519 FAIL, mix 0.0116 / 0.0117 FAIL.
- **Earlier scores (S27, flags 239882):** settled room-a 0.0385 (bias +0.009), corner 0.0424 (+0.013), contact 0.0374, lamp-wall 0.0328 (-0.003), hall 0.0371 (+0.013), close 0.0212. Room: blotch 0.0110 PASS, structure 0.0079 PASS, close-up 0.0082 PASS, world swim 0.0042 FAIL (settled-per-pose 0.0056; was 0.0006 before relighting at the cap), grid redraw 0.0050 FAIL, end 0.0036 FAIL. Still shimmer (60 still frames) 0.0000. Lights-only return: hall 0.0020, room 0.0022 (limit 0.002). Spin 0.1075 / flicker 0.0529 FAIL; mix 0.0116 FAIL.
- **Camera trace (S30, 239882 vs S31 with 524288), per readback while turning:** claimed 542 / 602, offer restarts 234 / 0.6; still: 0 claimed, 0 restarted, relit 16k (budget always full).
- **Open:** slow drift of texels at the cap (continual relighting moves the mean; swim settled-per-pose 0.0056); holding converged texels (flag 262144) fixed the drift (0.0006) but raised blotch to 0.0207 and close-up to 0.0165. Next: Lumen-style priority (relight what was read and is oldest, LastUsed − LastUpdated) with converged texels relit only when their inputs change. Moving and spinning boxes (Lumen recaptures cards of movers and relies on screen traces for them). Bounce darkness was fixed by push-style hit patches; Lumen's equivalent is radiosity reading last frame's indirect at hits.

- **Read rule at ray hits (S40):** after SHaRC [SHaRC]: a hit closer to its ray's origin than size × √3 reads the finest lit texel that satisfies t > size × √3 and holds at least 4 sets; otherwise the 25 cm patch, which falls back to its lit neighbours, never black. SHaRC instead continues the path past a too-close cell; we cannot afford another bounce there, so we pick a finer cell.

## Scheduling and change response (S62-S68, 2026-10-12 overnight)

- **Ours:** rate-based relighting (S64): each texel has a due interval from how much its light changes. It is due every round while it is unsettled, near a mover, or reached by a moved lamp. Otherwise the interval is 240 / (1 + 16 x inconsistency) frames, 30 at most while the light changes, and 600 when the texel is dormant. The batch is a ceiling, not a target: priority orders texels only when more are due than the batch holds. Two tiers (S65A) sit above that. Texels whose change is under LIGHTING_SC_TRICKLE_THR (0.173, found by bisection; table in SUMMARY.md) get one 16-ray entry from a small trickle list (4096). Texels above it get whole 64-ray sets, ranked by change. The change is the larger of the estimator's inconsistency and the moved-lamp delta at the texel (S65C, from the lamp's old and new position, range and power). Sun and sky light are stored as a share per texel (S63A) and rescaled by the global sun/sky level, so a day-to-night change is right on the first frame without relighting. The WRC (25 cm) cells keep accumulating under change with an EMA floor and are never reset (S62D). Anti-lag (S67A, after ReBLUR and A-SVGF history acceleration): when the short mean is more than 3 sd from the long one, the long mean moves to it. A neighbourhood variance clamp (2 sd, as TAA clamps history to the neighbourhood's colour box (Karis, UE4 TAA)) applies while the light changes. A texel read within 2 frames counts as read since its relight (S68A, Lumen LastUsed - LastUpdated [W22]). Without that slack, texels read every frame were relit every other round.
- **Reference:** Lumen updates the surface cache within a fixed budget by priority [W22]. GIBS fires change-detection rays at dormant surfels and varies rays per surfel [GIBS21]. SHaRC keeps a radiance cache with a stale-entry rule [SHaRC]. ReBLUR and A-SVGF speed history up where the signal changes.
- **Differences:** ours relights only what is due, so the GPU idles when little changes; Lumen always spends its budget. The trickle tier is our version of GIBS's change-detection rays.
- **Scores (640x360, S68A de906b50 vs S67A cb215ee2):** turn error area 0.0573 -> 0.0425, light move 0.0262 -> 0.0205, turn frames to under 2 % 120 -> 60, rays per round 444k -> 446k, fps 50.4 -> 49.5. Threshold 0.173 against no trickle tier: 56 % fewer rays per round, +15 % fps, flicker 0.0052 -> 0.0030.
- **Open:** the error after a turn is a dark bias (frame 1 -13.5 %, 16 -4 %, 60 -1.8 %). Cutting the seed's weight from 8 sets to 2 made it worse (S68B, reverted), so the early sets are dark as well as the seed. The likely cause is that bounce light builds up over rounds after a reveal, because hits read texels that are not lit yet. A temporary budget boost (S66A) fires for about 3 rounds after a turn and does not change the time to correct.

## Radiosity and multi-bounce

- **Ours:** ray hits read the light cache's irradiance at the hit (last round's value), so each relight adds one bounce: infinite bounce by feedback. In SC, hit patches are 25 cm and replaced on each relight.
- **Reference:** Lumen radiosity: one 4×4 hemisphere probe (16 rays) per 4×4 card texels, jittered over 4 frames, plane and hit-distance weighted interpolation, accumulated over 4 frames; hits read this frame's direct plus last frame's indirect [W22]. DDGI gets multi-bounce the same way through probe feedback [DDGI].
- **Bounce split (S35, `GENOS_GI_CHECK_BOUNCES=1` references against a one-bounce engine, flag 2097152):** share of the picture from bounce 2 and later, reference / push engine: room-a 0.584 / 0.579, corner 0.873 / 0.857, contact 0.832 / 0.821, lamp-wall 0.661 / 0.661, hall 0.342 / 0.336, close 0.427 / 0.422. The first bounce is slightly bright in the dark corner and contact views (corner +15 % in push, +6 % in the surface cache; it is the gather, P6 has it too) and later bounces 1-1.6 points short there. The earlier 3-4 % darkness is gone.
- **Differences:** per-patch 64-ray hemispheres instead of shared 16-ray card probes; no spatial reuse between texels except pooling.
- **Open:** card-space probes for the bounce (Lumen's radiosity) would share rays between texels.

## Dynamic objects

- **Ours:** each frame lists the bounds of objects that moved, came or went (up to 64; more drops every kept stratum). Kept rays crossing them are re-traced; light-cache patches near a change (1 m) are relit first and run short means (2 sets) in SC.
- **Reference:** Lumen: movers are in the SDF/BVH, their cards are recaptured, pixels whose rays hit movers switch to fast temporal update with stronger spatial filtering [W21 s.99, W22]. ReSTIR GI validates reused samples with visibility rays.
- **Differences:** we invalidate by ray path (exact) rather than by hit velocity.
- **SC movers (S42-S49):** occluders that moved in the last 240 frames are movers; pixels inside a mover's bounds and within 1 m of this frame's changes show the probes, texels near a change are relit first with 2-set means. While only the light changes (lamps move), texels stay shown and lag a little, as Lumen's surface cache does (S46/S47; swapping every pixel to the probes swapped methods and was most of the mix error).
- **Scores:** P6 passes trail, mix, spin. SC (S49): mix 0.0084 / 0.0087 (target 0.0055), spin 0.0871 / flicker 0.0470 (the spinning green box is not caught as a mover; Anthony judges spin by eye). S54K (headless gi-check, not comparable bit for bit with the windowed rows): the movers never reached the GPU before S52 (the change block upload stopped after the changes); fixed, object-local texels pooled and trusted by sets (S53), texels kept shown next to movers (S54K): spin 0.0716 / 0.0397 (S49 headless 0.0861 / 0.0460), mix 0.0092 / 0.0095 (S49 headless 0.0084 / 0.0087, worse). Quick tier not yet run on the S54K defaults.
- **Open:** object-local texels (GIBS: transform id + local position) are not built; texels on a moving box are world-keyed and re-claimed as it moves.

## Contact term

- **Ours:** none at full resolution; contact comes from probe rays (16 px) and the filter's hit-distance clamp.
- **Reference:** Lumen's full-res short-range AO / bent normal (screen traces of about 16 px), applied after the temporal filter [W21, W22].
- **Open:** contact rows pass but corner contact is the weakest settled view; a full-res contact term is planned (design rec. 3).

## Temporal and history

- **Ours:** no radiance history in the screen path (visibility only, see kept hits). The SC holds per-texel running means (that is a history, world-anchored, reset per texel on change or under changing light).
- **Reference:** Lumen's 10-frame full-res temporal filter, per-pixel rejection, no global reset [W21]; SVGF / ReBLUR.
- **Open:** SC lights-only return is at the limit (0.0020-0.0022): stale light lingers in texel means after lamps move and come back.

## Reference path tracer

- **Ours:** `crates/debug/src/reference.rs` (CPU) and the GPU reference (`GENOS_REFERENCE_GPU=1`): primary rays per pixel centre, shadowed direct light at each vertex, cosine bounce, Russian roulette after 3 bounces, traced to 0.05 noise or 2048 spp, cached by scene, pose and size. Furnace, closed room and inverse-square checks hold it honest.
- **Reference:** standard unidirectional path tracing (PBRT).
- **Differences:** the GPU refs read 0.002-0.004 higher error for the same engine than the old CPU refs (old corner leak), so the settled mean line is 0.043 with GPU refs.

## gi-check grades

- **Ours:** `genos-stress gi-check` (see [Debugging](../debugging.md)): settled views against references (mean, bias, blob, contact, open wall), moving views, repro, camera, trail, flick (moving flicker and still shimmer), turn (arrival, onset, held, return), room (world swim, 48 px swim, grid redraw, blotch, structure, close-up, end), mix, spin, lampret (lights move and return; end against the soaked baseline, limit 0.002). approach (S56: run 9 m at 5 m/s up to a lit wall in the hall across the texel bands; grain of each moving frame against its pose settled, metrics::blotch at 2 px, limit 0.0056; onset 0.0045; frames held until the grain is under 0.003, limit 120; soaked end against the end pose settled fresh, blob limit 0.016; limits provisional, from S56B with ba9adca0 failing grain and end). Window titles name the run, GI mode, flags and test, plus REF while a reference renders. `GENOS_LIGHTING_SC_TRACE=1` prints the surface cache's claims, restarts, frees and relights per readback.
- **Reference:** Lumen is checked visually against the path tracer; we gate numerically.
- **Fill run (S71D/S71G, time to fill; Anthony 2026-10-12: pop-in is fine when the screen fills near-immediately, smoothing would be lag, so S71E's display ramp was reverted):** `fill.rhai`, the turn pose settled, then a cut to the unseen side held 120 frames (reveal/) and settled (reveal_end/), then a sun move 12:00 -> 15:00 held 120 frames (light/) and settled (light_end/). Score: frames until mean |L - settled| (linear luminance, 320 x 180) stays under 0.005, target 30 (provisional, MISS not FAIL); the note gives the gap at frames 0/10/30/60 and the share of pixel-frames stepping more than 0.01 from their 9-frame average (pop-in, not graded). With turns (turn run) and night->day (dnout frames to 2 %), time to good is the main lighting score. Older builds: run `fill.rhai` with their binary and score with `gi-check --score-only --only fill`.
- **Open:** the still shimmer row (frame to frame, 60 frames) reads 0 for every build; the drift Anthony sees is slower than 60 frames at 8-bit, so a longer-window row (first vs last of ~600 still frames) is next.

## Fill speed: sparse fill, reuse, denoise (S72, 2026-10-12)

- **S72A sparse fill** (flag 16777216 off): only key texels (even cells on both in-plane axes) of fine texels get rays; the rest show the bilinear mean of their 1-4 keys and are promoted to own rays when a key is missing, off-plane or keys differ by more than `LIGHTING_SC_SPARSE_RATIO` (1.6). Smoke: due backlog 35-50 % lower than S71G at the same per-frame budget; fps unchanged (light pass is per-pixel bound).
- **S72B ReSTIR GI reuse** (reverted by S72H: neighbour reuse without visibility spread correlated low-frequency blotches): one-sample reservoir per fine texel (`LIGHTING_SC_RES_AT`, slot is 160 B). A relight resamples its rays (target luminance x cosine), its own history (dropped under local change / changing light) and two coplanar same-size lit neighbours; the resampled estimate is blended 50/50 into the set mean. No retrace validation (change flags only); no Jacobian (same plane).

- **S72C denoiser primary + earlier stop** (flag 67108864 off): the S62A variance-guided, luminance/plane edge-stopped pool now runs at every set count (was: below 24 sets) and records its variance gain; `lighting_sc_settled` stops a texel when its noise / sqrt(gain) is under the perceptual target, so texels stop at "the denoised result is good enough".

- **S72D coarse-to-fine fill** (fixes S72A never refining non-keys when still): texels have levels (4-apart 2, 2-apart 1, rest 0); every level is relit, young texels weigh 4x/2x/1x by level, so a backlog fills the coarse grid first. A texel short of 4 sets shows the next level up's interpolation (2-apart keys, else 4-apart), mixed with its own sets as they come; only a luminance step between keys (an edge) promotes it to its own light at once.

- **S72C** denoiser-primary + stop on pooled noise: reverted by S72G (baked low-frequency blotches, stopped texels while the error was low-frequency). **S72E** still-frame direct reuse: reverted by S72F (no light-pass gain measured).
- **S72I confidence-weighted spread**: the keys' interpolation carries their confidence (sets / 4, the 4-apart keys at half), weighed against the texel's own sets, so own rays overwrite unconverged spread; `LIGHTING_SC_SPARSE_RATIO` 1.6 -> 1.3 so more edges (penumbras, contacts) get their own rays.

- **S72J region-uniform refinement** (flag 128 off; Anthony: uniform low or uniform high beats a patchwork, refine whole regions together). A page is 8 x 8 texels of one size on one plane; a hashed table (`LIGHTING_SC_PAGE_AT`, 65536 pages x 3 rotating rounds in the cache keys buffer) holds each page's fewest sets among its read texels. A texel more than `LIGHTING_SC_PAGE_LEAD` (2) sets ahead of its page waits, so a page's texels converge at the same rate and the budget flows to the pages behind; near a change still goes first. Quality differs only by page (texel size follows distance, so near vs far), never texel by texel.

- **S72K still-scene pops** (video of S72J, everything still: 8 x 8 pages popped as squares, coarse base then sharp pages): (1) the page gate's bar is the page minima interpolated between page centres plus a per-texel dither of up to one set, so neighbouring pages advance together and a page's texels do not step at once; edge (promoted) texels are not held to pages. (2) The coarse-to-own handover is by sample count: own weight n / (n + 2 c f), c the keys' confidence (half for 4-apart keys, half again across an edge), f fading to 0 at 16 sets; the base is the keys' current values, so both converge. (3) A texel with no sets that shows its keys' interpolation now counts in the pixel's bilerp (it used to drop out and pop in with its first set); edges blend the same way instead of switching. (4) Flags 2^23 and up now reach the GPU (head 3.w); before, the CPU capped them, and flag 16777216 turned every lower flag on.

- **S72L wide anchoring + first fill** (flag 33554432 off; Anthony: squares compound as regions converge to their own noisy means). Every texel's shown value (the pool, now at all set counts) is the wide same-plane mean (16 taps 2-16 cells out, weighed by sets) plus its own difference shrunk by significance d^2 / (d^2 + (2 sigma)^2), sigma its mean's standard error; noise-level differences stay with the neighbourhood, real steps pass, detail grows in as sigma falls. In the pixel read, a corner with no texel and no lit parent takes the hit-size patch or its lit neighbours, and corners with nothing are left out of the blend rather than handed to the screen probes texel by texel (probes only where most of the pixel has nothing).

- **S72M first-reveal seam** (video: screen-aligned tearing on the side the camera turns to): the newly revealed strip had texels claimed but unlit, no lit parent, so its pixels showed the screen probes while the rest showed texels, a method seam along the screen for the rounds until they lit. Now such a texel (fine, nothing lit at or above it) starts from the pixel's probe irradiance as one set, so the picture is the surface cache from the first frame and refines by count.
- **S72N seeds apart, dark fill**: a texel's first light from elsewhere (resample of the next sizes, the coarser cells, the hit-size patch, the pixel's probes) is a seed kept beside its own mean (SPARSE_AT .y rgb9e5, .z confidence), never counted as its own samples; it joins the coarse-level interpolation as the prior of the count-based handover, and the texel is scheduled as unlit (its page waits for it). The coarser-cell fallback is bilinear (a nearest 50 cm / 1 m parent showed squares). Flag 134217728: the old seeding. Dark fill: growing WRC cells that rays read are ranked with the texels by staleness instead of the first-come background list (1024 a round in hash order, so the same cells won and the rest waited), and while under half their cap their samples weigh k^2 so bounce light that arrives late is not held at 1/n. Flag 67108864 off. Audit: the mover-bounds probes (flag 4194304) are inert while LIGHTING_SC_KEEP_NEAR is set.
- **S72O decorrelation and rectification** (13:43: tearing fixed, squares still build up, less): texels of a page no longer move in step. While refining (8 to 64 sets) each texel sits out one round in four on its own hashed phase; the page-bar dither spans the 2-set lead (was 1); the quarter of the hemisphere a relight starts from turns per texel and set; sets retaken after the second pass are new rays. The wide regularisation is edge-aware and connected: four directions a texel (axis or diagonal by hash), walked 2/4/8/16 cells, each stopping at a missing or off-plane cell or a step beyond 4 standard errors, taps weighted by sets and a Gaussian in standard errors. The stored mean (not only the shown value) is pulled toward that regularised value by 1/(sets+1) a round below 64 sets, so early per-region noise cannot lock in; real steps are many standard errors and stay. Flag 268435456 off.
- **S72P two layers** (Anthony 13:48, supersedes the S72O stored-value smoothing, which is now inactive): layer 1 is the traced data (texel means, WRC), never shown as is; layer 2, shown, is rebuilt from it per texel each round (texels shown in the last 30 frames) into WORLD_AT .y with its support in SPARSE_AT .w. Eight directions walked 1/2/4/8/16 cells, same plane only, stopping at a missing/off-plane cell (visibility) or a real step (over 3 standard errors and 12 % from the last tap, or 4 sigma and 24 % from the start), so the radius shrinks across shadow edges, contacts and creases and widens on flat light; taps weigh sets x r x Gaussian(8 cells), stale taps a quarter. The display hands over from a texel's seed to the field by the field's support. Ray density follows the light: past 16 sets, texels where layer 2 found flat light (no edge within 4 cells) are relit one round in 4 (level 0) or 2 (level 1); keys, edge texels, near-change and changing-light texels keep their rate. Flag 536870912 off.
- **S72Q smooth stops**: S72P's hard direction breaks popped (pop share 0.33 % / 0.72 % against S72O's 0.11 % / 0.19 %: a far tap's whole weight flipped as noise crossed the threshold). Now a step from the last tap scales the direction's transmittance by exp(-x^2) (x the step over max(3 sigma, 12 %)), the difference from the start scales the tap the same way (4 sigma, 24 %), and a direction ends below 1 % transmittance; Gaussian 6 cells. The flat-light ray thinning also runs while the sun moves (still not near a change or a moved lamp): S72P used 19-25 % more rays than S72O in the still run because the stress scene's sun moves and the thinning never ran.
- **S72R history suppression, stage 1 (MSME)**: the texel accumulator is the multi-scale mean estimator verbatim, one update per 64-ray set: firefly cap y <= 0.1 + short mean + 8 sigma; short mean rate 0.08; variance rate 0.04; rho = |long - short| / sigma; inconsistency rate 0.08; k = clamp(smoothstep(0, 1, rho max(0.02, iota - 0.2)), 1/256, 1) vbbr; vbbr -> clamp(0.5 short / sigma, 1/32, 1) at rate 0.1; never a hard reset. Newborns start with short mean = mean, inconsistency 1, vbbr 1. Free hint from the existing change records: a mover near the texel raises k to at least 0.5, a moved lamp to its delta at the texel, at most 0.9. This replaces the S59-S68 extras (plain-mean warm-up, young EMA floor, 3-sigma anti-lag, change alpha, neighbourhood clamp, restart after changing light); flag 1073741824 brings them back. Variance and rho are luminance scalars (Rec. 709). Not added: the A-SVGF same-seed re-trace (extra rays), unless big changes visibly lag. Sources: Barre-Brisebois et al., "Hybrid Rendering for Real-Time Ray Tracing", Ray Tracing Gems ch. 25 (2019), pp. 25-26; Halen, Brinck et al., "Global Illumination Based on Surfels", SIGGRAPH 2021 (EA SEED); WickedEngine ShaderInterop_SurfelGI.h and jure/webgiya (newborn seeding); Lumen, SIGGRAPH 2022 (fast update when traces hit moving objects, MaxFastUpdateModeAmount 0.9); Schied et al., "Gradient Estimation for Real-Time Adaptive Temporal Filtering", HPG 2018 (A-SVGF, the stage-2 option). Notes in /workspace/research/history-suppression.md.
- **S72S scaling down, MSME warm-up** (Anthony 14:03: the method must scale down to a GTX 1060-class GPU; with far fewer rays the picture stays even, with no popping, blotches or patchwork, and only builds up more slowly; nothing may depend on a minimum ray count). Every per-round budget (texel room and base, trickle, WRC refresh, scouts, reveal bursts) is divided by 2^GENOS_LIGHTING_SC_BUDGET_SHIFT (0-7, head 3.w bits 9-11); list layouts are unchanged. The design rules this keeps: layer 2 rebuilds from whatever texels have sets (unlit taps are skipped, not stopped at), seeds and the coarse levels cover the rest, page evenness and handovers count sets rather than rounds, and thinning and scheduling are by priority within whatever budget there is. S72R's verbatim MSME popped during fill (pop share 2.8 % at frames 20-80 against 0.15 %): with the variance starting at 0, sigma was about 0, so k was about 1 and a young texel's mean jumped to each new set. Now the variance starts at (0.15 luminance)^2 and k is at most 1/(n+1) below 16 sets (the change hint still overrides it).
- **S72V layer 2 as SVGF, irradiance-caching placement** (Anthony 14:15: moving lights and objects still left soft blocky mottling; use the papers' definitions and structure). After the cache pass, seven dispatches over the slots (lighting_atrous.comp): (0) variance per Schied et al. 2017 sec. 4.2, the texel's MSME moment variance, or with < 4 sets a 7x7 spatial moment estimate x 4/sets (Falcor SVGF reference); (1-5) five a-trous levels (Dammertz et al. 2010), B3 kernel (1/16, 1/4, 3/8, 1/4, 1/16), step 2^i cells, w_l = exp(-|dl| / (4 sqrt(g3x3 var) + eps)), w_n = max(0, n.n)^128, w_z with sigma_z 1, variance filtered with w^2; the WRC cells run the same levels on their own grid; (6) pull-push (Gortler et al. 1996): fine value weighted by its mask, WRC bilinear for the rest, written to the displayed field. Layer 1 is never written by layer 2 (no SVGF feedback). Only WRC cells and texels shown in the last 30 frames are filtered (the rest carry mask 0) [inferred]. S72V3 (Anthony 14:25, d23a9218 very splotchy, slower): each level's certainty is Gortler et al. 1996's pull weight min(1, sum h w) with the unnormalized kernel (4 x normalized), not the normalized-convolution average, so sparse IC sites saturate after a level and the 25 cm WRC only fills real holes (it had 3/4 of every texel: the splotches and a darker picture); a settled IC site (16+ sets, no local change) is relit one round in 4 [inferred rate] (records kept until the light changes, Ward 1988). S72V4 (Anthony 14:26, close-up: regular wavy stripes on floor and ceiling, smeared wall blotches, dark): the IC lattice itself was in the picture; only the lattice texels were traced and the rest had no data, and a-trous over a sparse lattice is not an interpolation (SVGF assumes a sample at every pixel). Now every texel is traced and the IC spacing g sets its rate, one round in min(g^2, 16), so the rays per area are those of the lattice [deviation: IC places records in space]. S72V5 (Anthony 14:26-27, "have we missed something?"; two wiring faults found with diagnostic builds: identity layer 2 was bright, the push alone already dark): (1) the a-trous taps addressed cells from the texel's stored position, which is where its claimer stood (a pixel's bilinear corner claims the neighbouring cell), so about half the texels had every tap shifted by a cell; the texel's cell now comes from its key (lighting_sc_true_cell); (2) the input weight t/4 [inferred] is gone: a texel with samples counts fully, as a pixel in SVGF (whose filter has no count weights). With t/4 the texels kept short under changing light lost to the settled ones around them and to the 25 cm WRC in the push, which darkened lit areas by about half and drew the WRC's blocks. S72V6: the real fault behind S72V-V5's dark, blocky, striped picture: the a-trous stage number was ORed into push-constant bits 1-3, and bit 3 is the gather's "changes this frame", set on most frames with moving lights or objects. Every stage then read 4-7: the variance pass and the first levels never ran, the push found no certainty, the field stayed empty and pixels showed the texels' seeds (25 cm parent bilinear, darker). A diagnostic build showing the push weight W found it (W was 0 everywhere). The stage bits are now cleared before the stage is set. S72V7: with the filter running, dawn (sun x1) showed camouflage plateaus: texels relit at different times hold their light in different sun scales (the pixels apply each texel's sun/sky gain), and the luminance stop kept those groups apart. Each texel's light (and variance, by the gain squared) is now carried to now before the levels; the field is light as of now and shown without a gain. S72V8: where a texel has no field yet it shows its own sets before its seed; a seed is a first-fill guess and must never outlast real samples (under S72V-V5's empty field, pixels kept their first-fill seeds: the picture filled once and never refined, and the seeds' 25 cm parent bilinear drew the ripples and streaks). S72V9: the IC rate and the settled 1-in-4 apply only while the light is quiet (IC records hold for static light; Ward 1988, Krivanek et al. 2006 recompute records when it changes). At dawn with the sun at x1, texels relit at different rounds held light of different moments and the luminance stop kept the groups apart (camouflage patches). Cost: under a moving sun the ray count is the unthinned one. S72V11: per record instead: the thinning holds while the global light since the texel's relight has changed by under 5 % (lighting_sc_global_change) [inferred threshold]; S72V9's frame-wide quiet test is never met with the sun running (still smoke: 744k rays a round settled against 87k). S72V10 (Anthony 15:05: texels entering the screen when walking backwards or turning must fill quickly): a field is shown only if the push wrote it this frame or the last (stamp in a-trous buffer B .w). A texel back on screen after more than 30 frames off it held the field from when it left; for its first frame it now shows the 25 cm WRC as of now (bilinear, lighting_sc_wrc_now), then its rebuilt field; the Lumen-priority scheduler already relights it first (read now, updated long ago). Not done: a velocity- and rotation-extrapolated frustum margin (guard band) to pre-claim texels off screen: the G-buffer is rastered at the screen size, and claiming cells by extending visible planes past the edge would create texels inside walls the planes run into [needs a guard-band G-buffer or a ray-validated claim]. Placement: irradiance caching (Ward et al. 1988; Ward & Heckbert 1992; Krivanek et al. 2006 sec. 5.1-5.2): R = min(R_geo, E/|grad E|), neighbour clamp R_i = min(R_i, R_j + |p_i - p_j|), clamped to [1, 64] cells, influence a R with a = 0.25; a texel is traced only on the power-of-two grid (1-8 cells) of spacing a R'; near a change every texel is traced. Replaces S72Q layer 2, the S72O stagger, pages and the sparse levels (all off under SVGF); MSME (S72S) kept. Flag 2147483648 (head 3.w bit 8) turns it off. Deviations, flagged for Anthony: w_z uses half a texel instead of the screen depth gradient; texels without data join as normalized convolution (Knutsson & Westin 1993) with mask min(1, sets/4) (WRC sets/16) [inferred]; no feedback of level 1 into history; no R_geo term (S72V2: a missing texel is unseen, not the surface end, and the first try held R at 4-8 cells so nothing thinned; Ward's harmonic mean ray distance needs per-ray hit distances kept); grad E is the central difference of the filtered field instead of Ward & Heckbert's hemisphere gradient; a, R_min, R_max chosen [inferred]; change-adjacent texels traced regardless [inferred].

- **S74 cost cuts (2026-10-12 16:05-16:25; S73A/B reverted to the checkpoint tree first, pre-fill kept on branch gi-v2-prefill)**.
  - *S74A last-camera fix*: the CPU copied last frame's eye and basis from bytes 48/528/544/560, an older scene-block layout (scene_data.glsl: eye 16, right 96, up 112, forward 128). Scouts saw a moving camera every frame (4096 scout rays a round when still) and the probes' reprojection (lighting_gather.comp) read garbage. Fixed.
  - *S74A A-SVGF change-adaptive rays* (Schied, Peters & Dachsbacher 2018; Schied et al. 2019). Every round, in each 3 x 3 block of fine texels one resolved texel (hash of block and round) re-traces one ray of its last relight with the same seed (direction, lamp pick). This runs every frame; there is no 'nothing changed' skip. lambda = min(1, |L_now - L_then| / max(L_now, L_then)) (A-SVGF eq. 6), written to the block's 9 texels; MSME blend alpha' = (1 - lambda) alpha + lambda (eq. 7). Resolved = lambda < 0.05 and the MSME standard error sqrt(var / n) < 0.03 x mean (ReBLUR-style); resolved texels take no rays besides checks. Lamp and mover hints only nominate checks (every resolved texel there). Age floor: not relit for 900 frames -> scheduled as before (Lumen recaptures oldest pages slowly). Deviations: lambda -> rays = a whole set with probability min(1, lambda / 0.25) a round [inferred; A-SVGF has no ray-count rule]; 1-ray checks spread as a box over the block, no a-trous on the gradients; checks are measurement only (A-SVGF reuses them as samples); thresholds [inferred]; ray luminance normalised by max(L_now, L_then, 0.1 x texel mean).
  - *S74A static-lamp shadow reuse for hits* (cached shadow pages with invalidation, as UE Virtual Shadow Maps): a light-cache or probe hit's shadow ray to a scene lamp is kept per 12.5 cm cell, facing axis and lamp, tagged with the lamp's epoch, which moves when the lamp changes, when a change bound of the frame lies within its range (suns: any change) or when the traced world is replaced. Deviations: visibility shared within a cell (hits only, never pixels); invalidation per lamp range, not per page.
  - Next (Anthony 16:22): Lumen surface-cache direct lighting (Wright et al. 2022: per card page, fixed budget, priority from LastUsed - LastUpdated). Flag: in Lumen the cached direct light is read by GI ray hits; the screen's own direct light is deferred-shaded per pixel (shadow maps / ray-traced shadows), not read from the surface cache. A screen read of 12.5 cm texels would blur shadow edges. S74B's per-pixel reuse was reverted for this.
  - Future option (Anthony 15:55, parked until spare performance): S-curve lag bound for off-screen pre-fill (gi-v2-prefill).
## GI v1 removed (2026-10-12)

GI v1 (radiance cascades, the world probe tier, `stage_tier`, the v1 `kick_light` paths, `light.comp`, `scene.frag`, `tier.glsl`, `ssaa.comp`, `lighting.rs`, `probes.rs`, `probe_tier.rs`, `field.rs`, `shadow.rs`, their tests and bench columns, and the `GENOS_GI` switch and stress `gi` knob) is gone. Removed with it: SSAA (it drew through the v1 fragment shader; panel code 2 now means FXAA), the live-readback switch, `TierStats` (replaced by `SettleStats { settled, still_frames }` from `Renderer::settle_stats()`), and the debug views that showed v1 data (cascade, probes, light field). Debug views now: full, direct, bounce, albedo, normal, depth, light, texels.

Historical v1 numbers (for comparison only; v1 cannot be run any more). Full tables in [GI v2 baseline](../gi-v2-baseline.md) (master `0874229d`, tag `gi-v2-baseline`, zanven-pc, RTX 4070, 2026-10-10):

| v1 check (320x180, CPU reference) | mean_rel | bias |
|---|---|---|
| hall | 0.043 | +0.001 |
| room-a | 0.198 | +0.107 |
| doorway | 0.040 | +0.016 |
| corner | 0.272 | +0.272 |
| noon / night | 0.055 / 0.058 | -0.026 / +0.049 |

v1 flicker gate (worst frozen max_delta, limit 0.004): 0.046 FAIL (hall). v1 light settle after a lamp colour change: 92 frames. The gi-check v1 comparison columns are dropped; their last values are in SUMMARY.md (work log) under the S5x/S6x rows.

## Fault ledger (short; the full ledger with first-seen points is in the work log SUMMARY.md)

Every failing grade of the surface cache, where it first appeared and which planned item should fix it. Re-checked before any push.

| Grade | Now / limit (S49, quick) | First seen | Expected fix |
|---|---|---|---|
| room end, soaked vs baseline | 0.0025 / 0.0020 | S26all, flags 239882, b1d7e956, 2026-10-11 | item 2 scheduler |
| turn held / return / end | 0.0052 / 0.0067 / 0.0073, limit 0.0020 | S26all 239882 (0.0045 / 0.0037 / 0.0043); longer settle (900) does not help | item 2 scheduler |
| lights-only return (lampret) | see SUMMARY (S47 0.0034 / 0.0025) / 0.0020 | S29lr239882, cf916c36 (0.0020 / 0.0022) | item 4 change detector |
| spin error / flicker / end | 0.0871 / 0.0470 / 0.0058, limits 0.027 / 0.020 / 0.002 | S26all 239882 (0.1075 / 0.0529) | item 4 object-local texels; judged by eye |
| mix still / turning / end | 0.0084 / 0.0087 / 0.0036, limit 0.0055 / 0.002 | S26all 239882 (0.0116) | item 4 |
| room light structure (target) | 0.0106 / 0.0100 | S34b estimator (239882 was 0.0079) | item 2 |
| approach grain while running (S56) | 0.0052 / 0.0056 (ba9adca0 0.0059); grain climbs 0.0027 -> 0.0071 over the run | ba9adca0, Anthony 2026-10-12 | under investigation (what grows with time moving) |
| room 48 px swim (target) | 0.0010 / 0.0006 | P6 itself 0.0025 | item 5 |

Fixed: camera regeneration (settled-per-pose 0.0056 -> 0.0012) and grid redraw (0.0050 -> 0.0008) by the estimator; offer restarts on camera moves (234 -> 0 per readback) by 524288.

### S78B: a-trous variance and push driven by change

Per-level counts (still scene, frames 300-600, of ~110k filtered texels): the
variance stage recomputed 75k a frame, because a spatial estimate (WRC cells,
young texels) was redone every frame, and the push (4 WRC lookups, 4 gradient
reads, 4 R clamps) ran for every filtered texel. Now a spatial estimate is
redone only when its own value moved or one of its 7 x 7 inputs did (stamp
slot 22.z, held for 2 frames), and the push only when its level 5 or a +-1
neighbour's moved (stamp 22.w), when it leans on the WRC (W < 1), or one frame
in 16 by hash. No new memory. Still 21.2 -> 19.6 ms (a-trous 3.46 -> 2.14),
moving 26.9 -> 25.7 ms (6.14 -> 4.83). [deviation: the 1-in-16 refresh is a
small fixed floor kept for the +-4 R clamp, which has no reverse links.]

### S78C: probe reprojection was off (float rounding of the head parity)

The change head's z holds `probe_runs & 1 + 2 * flags`; with the default flags
(6007050) it sits above 2^23, where floats step by 1, and every reader took
`uint(z + 0.5)`: an odd value + 0.5 rounds to the even one above. The view
parity read 0 every frame, so the gather's picks always wrote and read
different blocks, no probe stratum was ever reprojected, and 7 of 8 strata
were re-traced and re-lit as fills every frame (3.6M fill hits a frame in a
still scene; GPU counters). The same rounding toggled sc flag 1 every other
frame. All head reads now take the exact integer (`uint(z)`).
Still 19.6 -> 12.5 ms (trace 3.9 -> 0.6, light 8.5 -> 4.6; 48 -> 72 fps),
moving 25.7 -> 18.3 ms (38 -> 52 fps); pictures within 0.002 mean luminance.

### S78D: kept hits relit halfway only when they should be

A kept probe hit was relit when its stratum was traced and again halfway round
(LIGHTING_KEPT_RELIGHT), whatever happened: ~1.0M hits a frame in a still
scene. The halfway relight now needs a reason: a moved lamp reaching the hit
since its trace (lighting_wrc_lamp_changed), the sun / sky gain moving by more
than 1 %, or a mover nearby (as before). Still 12.5 -> 11.55 ms (light 4.57 ->
3.62), moving unchanged (18.3 -> 18.6, the lamps move everywhere there); pop
0.074 / 0.107 %. No new memory. [deviation: the bounce part of a kept hit now
refreshes with its re-trace, every LIGHTING_STRATA frames, unless lit above.]

### S78E: texel direct light past its age is checked, not recomputed

An entry of the texel direct-light cache past LIGHTING_TDL_STALE (600 frames)
with its signature unchanged was recomputed in full (all lamps, shadow rays)
from the budget. It now takes the S76C check (picked lamps at the texel
centre): within 10 % it is kept and its age restarts, else it is dropped and
recomputed. A confirming check also restarts the age of a signature-changed
entry. Timing unchanged at 600 frames (still 11.52, moving 18.7 ms); it removes
an age-only recompute wave every 600 frames. No new memory.

### S78F: stale WRC cells take a check, not a whole set

A WRC cell older than LIGHTING_WRC_REFRESH_FRAMES (180) was given a whole set
from the background list on age alone (607 cells a frame in the still scene).
It now takes the S77A 16-ray check when its deferred check is due (confirm
doubles the deferral, a miss escalates to a set). Growing cells (below their
cap, read) are unchanged here. Still 11.52 -> 11.20 ms, moving 18.7 ms; pop
0.073 / 0.110 %. No new memory.

### S78J: per-pixel direct light reused in quiet frames

Every pixel ran the lamp loop and its shadow rays every frame (2.05M pixels at
1080p, ~2 ms in a still scene). A pixel now keeps (signature, packed light) in
the work buffer (+8 B a pixel, 7.9 MiB at 1080p, fixed per resolution); the
signature hashes its surface point (1 mm), normal, sidedness and the epochs of
its lamps (lighting_lamps_sig, as the texel direct light). In a frame with no
change listed and no mover (lighting_pix_quiet) a matching pixel shows last
frame's light with no rays. Any mover anywhere relights every pixel: a moving
object's shadow can reach any pixel, and shadow reuse across movers tore edges
before (S77B). Still 11.2 -> 9.4 ms (light 3.49 -> 1.56, 80 -> 93 fps);
moving unchanged (18.8 ms); pop 0.088 / 0.115 %.

### S78K: held probe strata, confirmed by one check ray

A probe standing where it stood when its current stratum was last traced
(place pass: 1 mm, same normal) holds that stratum's hits: one ray a frame (by
hash) is re-traced as the check against the kept hit (miss/hit, 2 % of its
distance, normal), and rays whose path crosses a listed change are re-traced
as before. A failed check marks the probe for full traces over the next
LIGHTING_STRATA frames. The held hits are lit as the halfway relight (S78D:
only for a reason). No new memory (flags in the stratum and probe normal w).
Still 9.42 -> 8.33 ms (trace 0.61 -> 0.29, light 1.56 -> 0.90; 93 -> 103 fps);
moving unchanged (18.9 ms: there the moving lamps relight the kept hits);
pop 0.077 / 0.114 %. [deviation: a static held stratum keeps its rays' exact
directions instead of being re-traced with the same directions.]

### S78P: gather bins in shared memory
The gather's binning round runs one workgroup per probe (one thread per ray
index). Each thread writes its 8 strata's (cell, light x w, w x t) to shared
memory; thread c then sums cell c and writes it once. Before this, every ray did
read-modify-writes to the cell tables in VRAM. The sums and outputs are the same
(only the summation order differs). Cell table 1 no longer holds the distance
sums afterwards (the filter overwrites it). Gather time: 1.25 -> 0.91 ms still,
1.62 -> 0.95 ms moving. No added VRAM.

### S78X2: dense a-trous when the dirty fraction is high

S78X's work lists help a still scene (~5-18k dirty texels of ~108k filtered:
the million-thread dispatch returns after one uniform read) and regress a
moving one. Each mark does a contended `atomicAdd` on that level's single
counter, and the dispatch then walks slots in append order, so the 5x5 taps
lose the coalescing of the dense slot sweep. The 65536 cap falls back only
after that compaction, and only when the list overflows. S78A had already been
rejected for the same locality cost.

Levels 1-5 now keep the lists while last frame's fullest level was at or under
`GENOS_LIGHTING_ATROUS_DIRTY` (default 0.25 of the filtered texels) and run the
dense sweep above it. `0` is always dense, `1` is the S78X lists up to the
cap. The decision is last frame's fraction: the mark that crosses the line is
already inside the filter. `GENOS_LIGHTING_DEBUG` or `GENOS_LIGHTING_SC_TRACE`
prints `lightingatrous` (dirty fraction, mode `list`/`dense`, threshold,
filtered count). Owner's S78X moving regression, RTX 4070, 1920x1080: a-trous
6.6 -> 9.4 ms, 42 -> 37 fps. This fallback is not re-timed here.
