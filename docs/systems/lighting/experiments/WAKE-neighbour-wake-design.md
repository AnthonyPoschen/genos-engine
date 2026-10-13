# WAKE: Anthony's wake design: no wakes from shared field; real change wakes neighbours by coarse-to-fine ring tests with group-test splitting; always-on stratified spread-out checks

- **Date:** 2026-10-12
- **Result:** implemented behind `GENOS_LIGHTING_WAKE=rings` (default remains the legacy check ray)
- **Commit / patch:** this commit

## Idea
Anthony's wake design: no wakes from shared field; real change wakes neighbours by coarse-to-fine ring tests with group-test splitting; always-on stratified spread-out checks

## Motivation
not recorded

## Paper basis
Ward/Krivanek IC error bound; A-SVGF sparse checks; Lumen background budget; progressive radiosity (Cohen 1988); hierarchical radiosity (Hanrahan 1991); group testing (Dorfman 1943); galloping search. Combination is original (documented deviation).

## What was implemented
`wake=rings` (default `legacy`), also `GENOS_LIGHTING_WAKE=rings`. The draw copies the settings and the variable wins when it is set. A lamp move or a geometry change queues up to eight events for eight frames. The place pass tests one probe per sector of a ring at r, 2r, 4r, … and splits 4 → 8 → 16 sectors when the group changed by more than the tolerance. A failed ring fully traces a quarter of the probes inside it; probes inside the first radius are traced in full. A negative radius (sun, lamp added or removed, colour change above 0.75 relative, or a change list that overflowed) traces every placed probe once. Verification retraces `wake_budget` probes a frame (default 16) and the CPU tightens the tolerance on a miss, down to a quarter of the setting. `GENOS_LIGHTING_WAKE_TOL` (0.08), `GENOS_LIGHTING_WAKE_BUDGET`, `GENOS_LIGHTING_WAKE_RADIUS` (8 m). Counters: rings tested, groups split, probes woken, verification hits and misses, probes traced, rays traced (`lightingwake` under `--lighting-debug`, or `GENOS_LIGHTING_DEBUG` when that argument was omitted). A settled rings frame uses the legacy idle (pixel direct and compose) plus the verification stride. Legacy is unchanged, including the settled idle.

## Numbers
Legacy baseline, before this scheduler: [WAKE0](WAKE0-legacy-trace-baseline.md). Lavapipe, 320×180 shipped hall: a held still frame traced 1862 probes and 1862 rays (one check ray each); a moving camera traced 2089 probes and 133696 rays (64 per probe).

Rings, same window and hall, budget 16. Frames 0–7 trace the stratum because the strata are not held yet. From frame 8 until the scene settles, a still frame traces one check ray per placed probe, the same as legacy, and wakes 0. The verification stride is only the settled idle. Shrinking to that stride at frame 8 froze a probe field the picture kept.

A 1080p grid at the 16 px tile is 120×68 = 8160 slots. The 320×180 hall placed 1862/3600 of them. The same fraction is about 4200 placed probes. Until settle, rings traces about one check ray each, matching legacy. After settle, rings traces about the budget (16). That 1080p figure is an expectation, not a measurement.

## Comparison

[DDGI] (Majercik, Guertin, Nowrouzezahrai, McGuire, "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields", JCGT 8(2), 2019, pp. 1–30, http://jcgt.org/published/0008/02/01/) updates every probe every frame. Section 4.2 calls that the conservative choice: all probes stay active. Section 4.4 blends new irradiance into the probe with hysteresis α in 0.85–0.98 (equation 1), so a change fades in over several frames. Multi-bounce light is the same blend, amortized. The paper discusses turning probes off and does not test rings around a change.

[DDGI-P] (Majercik, Marrs, Spjut, McGuire, "Scaling Probe-Based Real-Time Dynamic Global Illumination for Production", JCGT 10(2), 2021, https://jcgt.org/published/0010/02/01/) adds a probe state machine that sleeps probes which do not affect the image (about 30–50% of the update) and places cascades in camera-locked volumes. A per-frame ray budget is mentioned as future work; the shipped control is rays per probe and how many probes exist. Sleeping probes spend nothing. The probes that stay awake still spend their whole ray budget. There is no change-driven ring.

[Lumen] (Wright, Narkowicz, Kelly, "Lumen: Real-time Global Illumination in Unreal Engine 5", SIGGRAPH 2022 Advances in Real-Time Rendering in Games, https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf) traces a screen probe on every 4×4 tile every frame, jittered. Surface-cache cards are the budgeted part: priority is LastUsed − LastUpdated. World and screen radiance caches are budgeted the same way. Screen probes are not selected by a group test.

Deviations, on purpose:

- The scheduler is doubling rings, Dorfman group testing (split only a group that changed), and a galloping / hierarchical-radiosity refinement, plus a verification budget that tightens the tolerance. That combination is not DDGI's grid update and not Lumen's per-frame screen probes.
- DDGI blends a new sample into the probe texel (hysteresis). Here a failed ring fully traces a sparse quarter of the probes inside it, and the existing probe filter (`gdirty`) spreads that to neighbours. Probes inside the first radius are traced in full, which is denser than the outer quarter.
- A verification miss tightens the tolerance. It does not start a ring. The check index still changes every frame. The vote compares that ray's direct outgoing luminance (reflectance times irradiance before bounce, plus glow) to a plain float stored for the same kept hit. The rgb9e5 kept-light word is what gather reads, and it includes bounce. Scoring that word counted the surface cache settling as a change: on lavapipe, with the cache left on, a still frame missed 6 of 7 checks. A baseline of 0 has never been shaded and casts no vote. The float is luminance + 1, so a black shade is still a shade. DDGI blends a new sample into the same probe texel (section 4.4, hysteresis α 0.85–0.98). Lumen retraces its screen probes every frame and does not keep a per-direction shade for this vote. Default 0.08 is a change in that direction's direct light. Bounce lag is not a scene change.
- A big change (the sun, a lamp added or removed, a colour change above 0.75 relative, a change list that overflowed) traces every placed probe once. A 2× colour is a ring, not that full wake.
- Ring progress is stored per flight slot, so the frontier advances every other frame. A settled rings frame does not hold the picture with no work: pixels are lit and the verification stride is retraced. It does not keep place, gather, or the filter, so the stride does not move the frozen picture. Legacy drops to the same pixel direct and compose, without the stride. Until that settle, a still rings frame traces the same check ray legacy does.
- While a lamp event is in the queue, a kept hit is not reshade only because that lamp moved. Woken probes and their check rays still shade, and the probe filter spreads them. DDGI 2019 section 4.2 updates every probe every frame. This sparse relight does not. Owner RTX 4070, head `6cd3cef` vs gi-v2 `917b9f47`: moving mean 0.77 / p99 17.7 against run noise 0.43 / 2.3. The x8 difference sits on the red and white box faces (bounce lag). Still was 1.19 / 3.0 against 0.23 / 1.3 and the corner 0.32 / 1.3 against 0.10 / 1.0, from the stride replacing check rays before settle. Those two paths trace the check rays again. The moving lag is the departure, and it is not within that noise.
- DDGI's sleeping probes and Lumen's screen probes both spend a fixed budget on whatever is awake. Legacy here can go fully idle. After settle, rings keeps only the verification stride.

## Lavapipe cases

`crates/render/tests/lighting_frame.rs`, 320×180, tile 16, world cache off, one bounce (bounce 0 records no probe pass), one point lamp in a corner. Every slot placed: 240 probes. Frames 0–7 trace the stratum. From frame 8 a held probe traces one check ray, the same as legacy, until the scene settles.

- `rings_still_room_wakes_about_zero`, budget 8, tolerance 0.08. Frame 13: traced 240, rays 240, rings 0, woken 0, hits 8, misses 0. That is one check ray per placed probe, the same count legacy traces. Frames 8–12 were 7 or 8 hits and 0 misses. Frames 0–7 trace the stratum and cast no vote: that direction has no stored shade yet.
- `rings_corner_lamp_wakes_nearby_probes`, budget 4, ring cap 2 m. The lamp steps from (0.45, 1.2, 0.45) to (1.05, 1.2, 0.45). Each of the next 6 frames: rings 2, woken 10, traced 14, rays 644 (ten full strata of 64, plus the 4 verification rays). Splits were 0, 0, 4, 7, 4, 8: the fail bits are read a frame later, and the slot is double-buffered, so the split count moves on frame 2. The 10 probes are unique. Each is within 0.60 m of the nearer lamp centre (the closest was 0.33 m). The far wall, which is on screen past 2 m, was not in the list.
- `rings_verification_catches_a_missed_change`, ring cap 0, budget 48. A tolerance of 1 cannot miss a non-negative luminance, so the warm-up leaves the scale alone. The next frame drops to 0.75. Still: 42 hits and 0 misses. The same frame with a 6× colour (relative 5/7 ≈ 0.71, under the 0.75 full-wake cut): 0 hits and 42 misses. Rings 0 and woken 0 either way. The ray luminance scales by about (6−1)/6 ≈ 0.83, which is past 0.75, so every vote that had a stored shade missed.
- `rings_still_room_with_cache_votes_hits` is the same corner with the surface cache left on and bounces left unlimited (denoise off, budget 8, tolerance 0.08). That is the lighting the stress A/B runs. Scoring the kept-light word, frame 23 was 1 hit and 6 misses, and frames 8–23 stayed in that range. Scoring direct light, frame 23 was 7 hits and 0 misses, woken 0, rings 0, traced 240 (one check ray per probe, not the stride). Frames 16–23 were mostly hits.
- `still_rings_picture_matches_legacy`, same corner, cache on, denoise off, frame 12. Lavapipe max-channel delta of rings against legacy: mean 0.023, p99 1.0 (0–255). The owner still gap at `6cd3cef` was mean 1.19 / p99 3.0.

## Real-GPU cost (RTX 4070, head e8fe3a39)

Owner measurement, not a lavapipe number, at head e8fe3a39. Probes traced fell about 70× (8093 to about 115 at 1080p, 3600 to 58 on the corner). Lighting time did not: moving 28.1 vs 28.2 ms, still 20.8 to 21.6 ms, corner 12.25 to 13.3 ms (125 to 101 fps). Atrous was about 11 ms in every config. The picture difference was inside run-to-run noise. That corner eye was room A's dark corner; the command below looks at room B's lit corner instead.

Why the trace counter did not move the time:

| Pass | Legacy and rings, before this change | Why the counter did not save it |
| --- | --- | --- |
| Trace | One launch of `probes × rays + 2 × kept` threads on every unsettled frame. Settled rings kept that launch; settled legacy did not. | The 70× drop was an early-out inside the launch. The group count did not change. The `2 × kept` tail re-traces geometry crossings and fills. A lamp is not in that list, so those threads returned immediately and still occupied the GPU. |
| Direct | One launch of `pixels + 2 × kept`. A kept hit whose stratum is current, or the halfway stratum, is shaded when a lamp that reaches it has moved (`lighting_wrc_lamp_changed`). | Both modes did that shade. Moving `--dynamic 50` keeps lamps in range, so the direct pass stayed the same size after the trace counter fell. |
| Atrous | Seven dispatches over the surface-cache slots, on a cache round. Not on the settled idle. | The slot count does not depend on how many probes traced. About 11 ms in every config means those runs were still taking cache rounds. |

What changed:

- A settled rings frame is pixel direct and compose, plus one trace and one direct of the verification stride (`wake_budget` probes, one check ray each). Place, gather, filter, cache, and atrous stay off, matching legacy. The stride does not run gather, so it does not move the frozen picture.
- An unsettled rings frame with no live ring keeps the probe-ray launch, so the check rays match legacy until settle. The kept-hit and fill tail is omitted when no geometry changed. Shrinking that launch to the stride at frame 8 was the still and corner picture gap at `6cd3cef`.
- A live ring still launches the probe rays, so a woken probe is traced. The kept-hit and fill tail is omitted when no geometry changed. A kept hit is not shaded only because a lamp moved. A woken probe and its check ray still shade. The probe filter spreads that. That is the moving picture gap (mean 0.77 / p99 17.7).
- Atrous is unchanged. It remains the cache-round cost, shared by both modes.

`settled_rings_frame_matches_legacy_idle_plus_the_budget` and `unsettled_quiet_rings_frame_keeps_the_probe_rays` lock the group lists. `still_rings_picture_matches_legacy` compares the still corner. No new 4070 millisecond is claimed here.

## Owner A/B

`genos-stress --wake legacy|rings` writes `wake` and removes `GENOS_LIGHTING_WAKE` for the process, so the flag wins. The variable is the mirror and applies only when the flag is omitted. `--lighting-debug` prints `lightingwake` (traced, rays, rings, splits, woken, hits, misses). `--lighting-debug=0` forces that off. `GENOS_LIGHTING_DEBUG` applies only when the argument was omitted, and any set value turns the counters on. `--gpu-times 30` prints `GPU_MS` every 30 frames. `GENOS_GPU_TIMES` applies only when `--gpu-times` was omitted. Both lines go to stderr.

`--wake-log <run>` writes stdout and stderr itself to `./genos-debug/wake/<run>/run.txt`. The file starts with the argv. A value that contains a slash, or `.` or `..`, is the directory. `GENOS_WAKE_LOG` is the mirror and applies only when `--wake-log` was omitted. With neither set, the process keeps the terminal. The log is opened once, after the arguments parse and before the renderer, the same class as `--gpu-times`: a later settings write does not move the descriptors. `--out` stays the unfilled-event directory (`./genos-debug/unfilled` by default).

`--frames 5 --wake-log --shot` does not wait for the picture to settle. It draws five frames, writes the PNG of the last one, and exits. On lavapipe at 320×180 that returned (the log contains `frames=5` and the PNG is written). `--shot` with no `--frames` used to spin until the window closed, and a proof window does not close, so the process never wrote and never returned; it now draws 60 frames. The readback fence wait is two minutes, not forever. A timeout ends the process without `vkDeviceWaitIdle`, which would wait on the same stuck fence. The terminal stays empty for the whole run because the log has stdout, so a hang and a slow first frame look the same until `run.txt` grows.

Moving stress scene, default hall view:

```
cargo run --release -p genos-stress -- --proof --no-panel --size 1920x1080 --frames 180 --dynamic 50 --freeze-sun --still-boxes --wake legacy --gpu-times 30 --lighting-debug --wake-log moving-legacy
cargo run --release -p genos-stress -- --proof --no-panel --size 1920x1080 --frames 180 --dynamic 50 --freeze-sun --still-boxes --wake rings --gpu-times 30 --lighting-debug --wake-log moving-rings
```

Lit corner. The gi-check corner is position `[3.5, 1.6, 3.5]`, look-at `[0.2, 0.4, 0.2]` (`examples/stress/scripts/gi_check/views.rhai`, yaw −π/4, pitch −0.252). That is room A's northwest corner. Room A has no window and no roof hole, so noon leaves it near black. On lavapipe at 320×180 that picture's mean luminance was 1.6 (0–255) at frame 3, and every pixel was under 30. The pose below stands under room B's skylight and looks into the southeast corner, where the south and east windows meet. Eye `[21.5, 1.6, 21.2]`, look-at `[24.6, 0.45, 24.6]` (yaw 2.402316, pitch −0.244923). The same lavapipe setup at frame 30 had mean luminance 65 (0–255), and no pixel was under 15. `--view bounce` is room A looking out of its opening, not this corner.

```
cargo run --release -p genos-stress -- --proof --no-panel --size 640x360 --frames 180 --dynamic 0 --still-boxes --freeze-sun --time 0.25 --eye 21.5 1.6 21.2 2.402316 -0.244923 --wake legacy --gpu-times 30 --lighting-debug --wake-log corner-legacy
cargo run --release -p genos-stress -- --proof --no-panel --size 640x360 --frames 180 --dynamic 0 --still-boxes --freeze-sun --time 0.25 --eye 21.5 1.6 21.2 2.402316 -0.244923 --wake rings --gpu-times 30 --lighting-debug --wake-log corner-rings
```

Grep `lightingwake` in `./genos-debug/wake/moving-legacy/run.txt`, `moving-rings`, `corner-legacy`, and `corner-rings`.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Notes from chat
Anthony (2026-10-12 22:50, t453u) asked whether the constant probe searching plus fan-out technique would be worth publishing if it works and performs well. Answer not in the available transcript.

## Lessons / when to revisit
After it lands, retest S77C, S78G, S78H, S78L, S78N, S78R.
