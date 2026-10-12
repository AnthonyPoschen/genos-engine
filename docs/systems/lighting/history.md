# Lighting history: Anthony's feedback per build

Chronological extract of the "From the Engine chat" sections of the experiment files (Engine chat, 10-12 Oct 2026). Tags like t102u are Anthony's message numbers in the Engine transcript. See README "Missing history" for gaps.

## [E01: Splotch diagnosis: which cause makes v2 blotchy (cache resolution, per-texel albedo, probe rays, still-scene averaging)](experiments/E01-splotch-diagnosis.md)
Anthony asked (t9u/t10u) to read exactly how Lumen solves it ('it is a very very good lighting engine'): Lumen's published methods became the default answer. He also asked (t12u/t13u) for executable checks instead of image interpretation, hand-validated first, then ~1% spot checks: this became gi-check.

## [E02: Probes keep 512 hits (8 rotating strata of 64) and re-light them every frame; no reset on lamp/sun/sky change](experiments/E02-stored-probe-hits.md)
Anthony (t15u-t19u): looks good when still; small artefacts behind moving objects; the big one is splotches swimming while the camera moves, 'the moment the camera is stable it instantly goes good'; uncovered areas behind moving objects are splotchy; treat what is behind a dynamic object as important (keep it warm).

## [E03: Project each kept ray along its own direction; moving/uncovered probes borrow strata from last frame's 5x5 neighbours on the same surface](experiments/E03-per-ray-projection-neighbour-borrow.md)
Anthony (t22u-t26u): the test must capture the real problem and fail on today's build first; 'only measure at the end'.

## [E05: When a moving box crosses a kept ray, re-trace it in the same frame instead of dropping it](experiments/E05-kept-ray-retrace-on-movers.md)
Anthony (t46u-t52u): starts beautiful, goes splotchy and darker the moment anything moves, worst in the hallway by the dark room; this was later found to be a test artefact for the hallway (bounce-only view), see E09.

## [E08: Turn swim fix: trace missing strata from the probe in the same frame; pattern cell looked up half a cell in from the surface](experiments/E08-turn-fill-and-cell-offset.md)
Anthony (t74u-t88u) test rules: soak and record a settled frame before any motion; end soaked; motion returns to the exact start so start and end can be compared; short tests for one situation with one realistic movement; cut tests that never caught anything. Stuck splotches after stopping were not reproduced with camera-only motion.

## [E09: Room swim / spinning boxes: filter rounds 2/4/8 probes apart, 64 rays per light-cache patch; strata traced from inside a change are filled, patches near a change relit first](experiments/E09-room-swim-spin-fixes.md)
Anthony (t102u): massively improved but final quality off; the reference is grainy. (t103u-t109u): a 'wiggle' run looked fantastic; it was GI v1, which swims ~8x less on large blobs; v1 is the stability floor, not the goal ('it shouldn't be a shining star'); changing quality while moving is bad; blur should scale with noise and screen size (adaptive blend, later put on hold t119u).

## [E10: GPU path-traced reference on hardware ray queries (independent of the engine)](experiments/E10-gpu-reference.md)
Anthony (t116u): smoothed reference shows a clear bounce band off the walls while v2 shows blotches that do not follow light; became the light-structure grade.

## [E11: Stand-in test: pixels read a converged world light cache (world=1) to decide whether a surface cache is worth building](experiments/E11-surface-cache-stand-in.md)
Anthony (t120u/t121u): 'it will negate needing the adaptive blending'; (t122u) next is Lumen's few-rays system for performance.

## [E14: Survey item 1: GIBS change-aware cells (dormant cells fire few change-detection rays)](experiments/E14-item1-change-aware-cells.md)
Anthony (t134u): constant faint pixel swimming while still on the darkness fix; (t135u) lights moving and returning should give an identical image (lampret test); (t138u-t147u): use Lumen and 5 years of papers from all venues when stuck; keep a lighting doc comparing each component with papers (this doc). (t149u-t154u): build each item, mark done when its quick tests pass, log each failure with where it first appeared (fault ledger).

## [E15: Item 4: object-local texels for moving/spinning boxes](experiments/E15-item4-movers-gpu.md)
Anthony (t171u): S49 at ~20 fps, 'looks pretty good visually, a massive improvement'; (t187u) 'everything was looking good'; (t188u) make it the default config so plain make stress works; (t193u) 'just git push' (ba9adca0, pushed untested).

## [S57: One texel size (12.5 cm pixels, 25 cm hit patches) at every distance](experiments/S57-no-camera-distance-sizes.md)
Anthony (t194u-t212u): saw a coarse circle looking down and walls/floors dropping to coarse grain when walking closer; with everything frozen saw the ring; first 'don't delete them, make sure they don't remove what existed' (-> S56), then 'it's not in Lumen or the papers, it's causing more issues than it helps' -> removed.

## [S56: Texels at a new camera-distance size inherit converged light with full count; 10% hysteresis](experiments/S56-band-crossing-inherit.md)
Walk-up test: grain 0.0059 -> 0.0052, worst 0.0087 -> 0.0071, final settled 0.0175 -> 0.0142; trail got worse (0.0372 -> 0.0421). Anthony (t211u): 'better but still noticeable'.

## [E16: Lumen update order + GIBS extra rays + few rays per relight built together (S58A, S59C), then settle fixes (S59D, S59E)](experiments/E16-few-rays-gibs-lumen-bundle.md)
Anthony (t220u-t230u): do all three, then test; don't loop on small misses. (t231u/t232u) 'it's settling too soon... splotchy'; (t233u) no performance gains seen; (t235u) bump quality before it settles (S59E). (t237u): quality eases in like an easing curve; (t238u) seed a big new area from a few heavily traced points and borrow -> world space radiance cache seeding (name chosen t241u).

## [E17: World space radiance cache first draft: running average of 32 rounds, scout rays around the camera pre-fill cells, new fine cells seed from it](experiments/E17-wrc-seeding-prefill.md)
Anthony (t242u): seed slightly before it's needed, not just revealed areas. (t245u): corner reveal works with static lights, but seeded cells go stale when lights change. (t246u): refresh must be very cheap. (t249u, e00238dc): 'everything stays super low detail in a scene full of moving objects'; (t250u) converges once sun and boxes stop. (t253u) WRC refresh back to 64 rays; (t254u) never reset on light change (S62D). (t255u) day->night floor fades as rays build (-> S63A sun/sky share).

## [E18: Variance-guided surface-space denoise plus perceptual stop rule (S62A)](experiments/E18-denoise-perceptual-stop.md)
Anthony (t244u): still scene goes 20 -> 180 fps when idle; find a shortcut instead of brute-forcing the sqrt(n) tail. Approved queueing after the WRC fix (t247u).

## [E19: Rate-based updates: each surface's rate follows how much its light changes; budget is a ceiling, not a target](experiments/E19-rate-based-scheduling-design.md)
Anthony (t257u-t266u): moving lights cause swimming from non-uniform updates; neighbour group updates must stay inside the budget; rays proportional to how much the light changed; fewer total updates in the moving scene; 'without picking winners and losers', 'spotting waste in chasing low priority changes'; the denoiser keeps it smooth.

## [E20: Cache keeps sun/sky share separate from lamp light; sun/sky change rescales it at once (S63A); night->day black squares fix (S69A/B)](experiments/E20-sun-sky-share.md)
Anthony (t271u): still partial bad rays but fixes very fast. (t281u-t286u): insane splotchiness and black squares going night->day, fps 27 vs 9 while building; 'something is wasting performance'. (t293u) after bfdb8900 squares only where the camera looks.

## [E21: Code audit: is CPU<->GPU transfer the bottleneck?](experiments/E21-perf-transfer-audit.md)
Anthony (t294u-t297u): suspected transfers; design rule: everything frequently needed lives in GPU memory at deterministic locations, passes read/write each other there, CPU sends only small changes. (t297u answer) remove all v1 lighting and rename v2 (-> S71A-C); (t298u) no env variable needed.

## [E24: Fill-speed direction for S72: sparse fill, reuse, denoise, then regions refine together](experiments/E24-s72-fill-speed-direction.md)
Anthony (t299u-t301u): many small pops while filling; popping is fine if the screen fills near-immediately, smoothing would be lag. (t302u-t305u): measure how filled things need to be; papers on not filling every texel; 'we don't need tests we need action'. (t307u) fills faster but pops are bigger, not spread wide enough; still scene ~40 fps. (t308u/t309u) spread radius should scale with camera distance (not built beyond texel size). (t313u) quality before performance. (t315u-t317u) whole-image consistency: low everywhere or high everywhere beats a patchwork. (t321u-t325u) page squares pop; first-fill tearing when rounding corners. (t327u) two layers (traced data never shown; shown smooth field). (t328u/t329u) suppress history smoothly by how different new data is; follow the papers' maths. (t331u/t332u) GTX 1060 rule. (t333u/t334u) indirect light is smooth; fewer, smarter probes. (t335u/t336u) follow papers' algorithms and structure; our design choices are where bugs come from. (t337u) ec3d5e74 better but pages blocky with moving lights -> paper-exact SVGF (S72V). (t339u) d23a9218 'very very splotchy, big massive patches; don't abandon it'; (t342u/t343u) ripples, 'have we missed something?' (-> V5/V6 wiring bugs). Checkpoint 6b4fe308 tagged after Anthony's approval.

## [S71E: Display ramp to hide convergence pop](experiments/S71E-display-ramp.md)
Anthony (t299u-t301u, 12:27-12:37): pops are fine if the screen fills near-immediately; smoothing becomes the issue.

## [S72A: Only key texels get rays; others bilinear from keys](experiments/S72A-sparse-fill.md)
Anthony (t307u): frame rate noticeably up, texels fill faster, but pop-ins are bigger; not spread wide enough; still scene ~40 fps. A video showed static coarse blocks that never refined (-> S72D).

## [S72J: 8x8-texel pages refine together](experiments/S72J-region-uniform-pages.md)
6d055836 still-scene run: every texel >= 4 sets by frame 60; 38 fps; light pass ~16 ms; page gate held 21-30k texels a round. Anthony (t321u/t322u): better on average but the detail pass still pops, even with all movement stopped.

## [S72K: Smooth cross-page bar, count-based handover](experiments/S72K-smooth-page-bar.md)
Pop score could not separate it from 6d055836 (1.64% vs 1.61%); frame differences showed square page pops replaced by per-texel speckle. Anthony (t325u): initial fill fast and good, squares still appear but less; tearing when rounding corners quickly on first fill.

## [S72M: Seed first-reveal texels from probe light](experiments/S72M-first-reveal-seed.md)
Bundle 3db494ca: fps 27-34, still ~37, light pass 16.8 ms, total 26 ms; pop 0.40% vs 0.76% early, 0.22% vs 0.29% later. Anthony (t326u): tearing fixed, squares building up less noticeable.

## [S72S: Budgets scale by GENOS_LIGHTING_SC_BUDGET_SHIFT; MSME warm-up](experiments/S72S-budget-shift-scaling.md)
Pop / rays per frame (early -> settled): 3db494ca 0.40/0.22; S72N 0.16/0.26; S72O 0.11/0.19, 676k -> 331k; S72P 0.33/0.72, 847k -> 393k; S72Q 0.15/0.11, 153k -> 54k; S72R 2.83/0.11; S72S 0.095/0.111, 579k -> 49k (-84% settled vs S72O). fps 29-36, light 15.7 ms, total 24.6 ms. Failed the weak-GPU rule: at 1/8 and 1/16 budget still blotchy at frames 60 and 600. Anthony (t337u): better but pages blocky with moving lights.

## [S72V: Layer 2 as SVGF a-trous + Gortler pull-push; irradiance-caching placement (V-V11 fixes)](experiments/S72V-svgf-layer2-ic-placement.md)
Anthony (t338u-t343u): d23a9218 'very very splotchy, big massive patches, don't abandon it'; frame rate down (~4 ms new pass); close-up ripples/streaks; 'have we missed something?'. After V5-V11 he approved; checkpoint gi-v2-checkpoint-2026-10-12 tagged at 6b4fe308.

## [E23: Profiling at the checkpoint: how much traced work changes the picture](experiments/E23-cost-waste-analysis.md)
Anthony (t366u-t372u): measure the dynamic scene for waste; a static scene should reach 1000+ fps; a cheap continuous research-based check still has to run. Decisions (t373u): never 'skip' lighting, always verify with an extraordinarily cheap check; adaptive rays yes; smoothing only changed pages doubtful (try separately later, t374u); direct-light cache only as shadow reuse for static lamps. (t376u-t378u) cache direct light Lumen's way; (Q t378) 'Follow Lumen exactly: cache feeds bounce-light rays only, screen stays per-pixel'.

## [S73A: Guard-band scout rays claim fine texels just outside the frustum](experiments/S73A-offscreen-prefill-guard-band.md)
Anthony (15:51-15:57, t357u-t363u): do off-screen pre-fill for turns and walking backwards next; pre-fill directions not faced and occluded spaces at lower priority (S-curve lag idea); asked for paper basis, then: try the Lumen approach, expecting it to look bad until there is spare performance.

## [S73B: All scout hits claim fine texels and stamp LastUsed](experiments/S73B-lumen-feedback-prefill.md)
Turn strip ~/.cache/gi2diag/perf/turn-S73B/strip.png was not looked at; logs ship-S73A.log, ship-S73B.log.

## [S74A-cam: Fix: last-frame camera read from stale scene-block offsets](experiments/S74A-cam-last-camera-offsets.md)
Reported with 50db0f70: 32-38 fps vs 27-30 for the checkpoint, same brightness; pop 0.105% vs 0.084% settled (later shown to be within the checkpoint's own run-to-run range).

## [S74B: Pixel direct light reused when point unchanged and no change bound touches its shadow segments](experiments/S74B-pixel-direct-reuse-segments.md)
Anthony (t376u): cache the direct light only if we can redo exactly the parts movement affects. Then chose (t378 card) 'Follow Lumen exactly: cache feeds bounce-light rays only, screen stays per-pixel', so S74B was reverted for S75.

## [S75: Lumen surface-cache direct lighting for GI hits (TDL); A-SVGF SE 0.05](experiments/S75-lumen-direct-light-cache.md)
1920x1080, back-to-back on zanven-pc. 6b4fe308 / S74A / S75: still total 31.3-37.0 / 25.6 / 24.3-24.0 ms; still light pass 16.0 / 10.1 / 8.7 ms; still fps 27-31 / 37 / 40; moving total 35.0 / 29.1 / 29.8 ms; moving light 17.9 / 12.0 / 12.7 ms; moving fps 28 / 33 / 33; pop 0.081-0.090/0.111-0.124 / 0.072-0.094/0.102-0.110 / 0.087/0.114 %; brightness night/turn30/turn180/day30/day180 .351/.497/.505/.534/.531 -> .351/.495/.504/.536/.528. Moving scene is perf/moving.rhai (dusk, lamps and boxes moving, 50% dynamic, 600 frames). Resolved SE threshold 3% -> 5% (no paper value; 10% tried and rejected: early pop 0.45-0.53%, turn/day ~2.5% darker). The 0.084% 'pop regression' was one lucky checkpoint run. Anthony (t380u): 'barely notice the performance gains... it now looks great'.

## [S76: Variance-aware lambda, deferred checks, lambda-proportional rays, hints only nominate](experiments/S76-check-is-the-gate.md)
Moving scene d591abfa -> S76: cache rays per frame 621k -> 561k (f20-120), 242k -> 154k (f120-300), 232k -> 96k (f300-610); texels relit 3,630 -> 1,486; still 24.5/24.0 ms (39-40 fps); moving 29.8 -> 29.4 ms (33 fps), light 12.7 -> 12.5, trace 5.1 -> 4.8; pop 0.087/0.114 -> 0.079/0.107 %; brightness .350/.498/.504/.537/.529. Confirmations defer the next check 8 -> 128 frames; escalation rays scale 16..64 with lambda. Why fps barely moved: the cached direct light hit its 32,768-texel limit every frame under moving lamps. Anthony (t386u): during the sun sweep some spots between regions were skipped and artefacts lingered until a check landed on them.

## [S76B-D: TDL and WRC confirm-then-escalate; persistent ring nominations, fair service, global check boost](experiments/S76B-D-confirm-escalate-ring-nominations.md)
78637784 -> dbb4777a: moving 29.2 -> 28.6 ms (33 -> 34 fps), almost all from the TDL check; cache rays 96k -> 125k per frame (re-scheduled WRC cells ~27k). TDL check: picked lamps shadow-traced; within 10% the value is kept and nudged a quarter toward the check. Sun-sweep gaps: neighbours of a big change were never queued, not dropped.

## [E25: S76B-S77 one-at-a-time perf steps and Anthony's verdicts](experiments/E25-s77-perf-steps-feedback.md)
Anthony (t380u-t385u): barely notices gains; something keeps the budget busy; A-SVGF should grade change, a cheap ray that comes back close keeps the area quiet. (t386u) after S76 sun sweep left skipped spots between regions (-> ring nominations, S76B-D). (t387 answer): build cheap WRC check, per-pixel shadow caching, dirty-page smoothing and gather/compact-only-changed, one at a time; (t394u-t397u) put them straight on gi-v2, quick passes. (t399u/t400u) moving shadows jittery torn edges -> S77B reverted. (t401u) 'very disappointed these changes did not drastically increase frame rate'. (t402u-t405u) cap rays by need, stop refining within tolerance, want 1000s fps and spend free compute on drastic changes. (t415u/t416u) keep lighting VRAM roughly fixed but adding memory is fine if it cuts compute. (t417u) suspects rays shot at stale data because budget allows. (t419u/t420u) his own fps on c660c0db mostly 20-30 with moments at 52; keep tests consistent. (t422u) low-priority needed work stays queued at a small fixed rate. (t423u) moving scene 0 improvement, still 80 fps, no visual issues.

## [S77B: Per-pixel shadow reuse with check and escalate](experiments/S77B-pixel-shadow-reuse.md)
First tried and reverted for no gain (light pass stayed 8.6-9 ms); Anthony asked for all four steps on gi-v2 (t394u-t397u) so it was re-applied unchanged. Anthony (t399u/t400u): moving shadows had jittery torn edge artefacts; the rest of the shadow was fine. Reverted in 9e242987.

## [S78C: Float +0.5 rounding killed probe reprojection](experiments/S78C-exact-head-reads.md)
Anthony (t418u/t419u): his own fps on c660c0db mostly 20-30 with moments at 52; told to keep tests consistent rather than chase it (t420u).

## [S78F: Stale WRC cells take a due check](experiments/S78F-stale-wrc-check.md)
Anthony (t423u): moving scene showed 0 improvement for him; still scene improved to 80 fps; no visual issues.

## [S78Q: Weighted lamp picking per pixel (ReSTIR DI): Q fused spatial, Q2 temporal-only, Q3 +RIS, Q4 RIS 8 cand + temporal + 1 spatial](experiments/S78Q-restir-di-pixels.md)
Anthony (t430u/t431u): asked whether it throws lamps away (no: unbiased weighted picking); add the 100-lights small room as a benchmark. Decision (Q t436): 'Shelve Q4 for now'.

## [S78R: Gather skip for held probes](experiments/S78R-held-probe-gather-skip.md)
Anthony (t440u-t442u): need enhanced debug readouts; suspected a cycle of probes waking each other; this would have affected every earlier 'go quiet' test.

## [S78S: Debug counters: what keeps still probes awake](experiments/S78S-counters-still-churn.md)
Anthony (t445u/t446u): dump GPU data (lighting maps as textures etc.) and make each component separable and isolatable. (t448u-t451u): papers vs our refire logic; always-on spread checks; no wakes from the shared field; ring-test group-split wakes (approved).
