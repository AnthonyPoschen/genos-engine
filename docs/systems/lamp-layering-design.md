# Lamp layers and fast-blend settling

Status: **Proposed**. ADR: [0017](../adr/0017-lamp-layering.md). Terms: [`docs/lighting/GLOSSARY.md`](../lighting/GLOSSARY.md). This document does not change the renderer. The budgeted refresh it relies on is [ADR 0013](../adr/0013-lighting-invalidation-is-local.md) and the in-flight branch `cursor/lighting-p3-capped-refresh`. This design does not edit that branch.

Anthony, 8 Oct 2026 (`docs/systems/lighting/history.md`): probes never throw a value away; they blend and settle in 100–200 ms; a point lamp can be marked fixed; a fixed lamp gets a baked layer re-baked locally. The hardware floor is unchanged: RTX 4070 at 1440p for the high end, Low tuned for a GTX 1060 6 GB and an RX 580, 4 GB VRAM minimum, lighting held to about 200 MB.

## 1. Goals

- A static point lamp can carry an optional flag. The flag gives that lamp a **lamp layer**: its finished indirect irradiance at unit white. Switching it off or on, dimming it, or running a glow is a **layer weight** write. The bounce does not retrace.
- Every other light writes the **live layer**: moving lamps, static lamps without the flag, the sun's indirect, and emissive. One field, not one field per light.
- **Fast-blend** lands first. A live-layer change reaches a settled picture in 100–200 ms on Medium and High (400 ms on Low) and does not pop. A probe or a pixel reads a finished value only.
- Memory per layered lamp is capped per preset. Past the cap, the lamp stays on the live layer.
- A lamp layer uses the cache entry's existing detail level. It does not invent a second set of sizes.

## 2. Non-goals

- A private layer for a moving lamp. `docs/systems/gi-design.md` proposed that. A moving lamp's indirect field changes when the lamp moves, so the layer would be rebuilt on the move. That spends the memory the flag is meant to save.
- Baking direct light into the layer. Direct stays per pixel (ADR 0012, S75). The pixel shader multiplies the same weight. Shadows stay in the direct pass and the shadow cell cache.
- A display ramp. S71E was reverted: smoothing the shown picture is lag. Fast-blend is the estimator's published mean, not a second fade.
- Edit-time lightmaps, a CPU radiosity solve, Enlighten's UV atlas, DDGI probe volumes, and radiance cascades.
- Potato GI. Potato stays direct plus flat ambient.
- Switchable emissive as its own layer. Emissive stays on the live layer until a later design.
- Any change to `lighting_schedule.rs` or the phase-3 branch.

## 3. Where it sits

Indirect light already lives in the surface light cache: hashed world-space entries, 25 cm near the eye and doubling with distance, MSME on each entry, screen probes that re-read light and reuse visibility (ADR 0012). ADR 0013 adds a level on the key (L0 25 cm through L4 4 m), a work-unit cap per pass, and a priority scheduler. There is no full-frame fallback. Under load, values get older and stay finished.

| Piece | What this design does with it |
|---|---|
| Live irradiance on a cache entry | Stays the live layer. Fast-blend changes only how fast the published long mean may move after the short mean has confirmed a change. |
| ADR 0013 ray, relight, and denoise caps | A bake and a live-layer refresh spend those caps. They do not raise them. Work past the cap waits. |
| Coarse layer (L3–L4, fixed MB cap) | Unchanged. Lamp-layer samples stored on a coarse entry are charged to the lamp pool below, not to that cap. Evicting the entry drops the samples with it. |
| Sun and sky share (E20) | Stays its own rescale. The sun is not a lamp and does not get a lamp layer. |
| Screen probes | Read the published sum. Kept hits are not dropped on a lamp change (E02). |
| Presets | Potato, Low, Medium, High, as in ADR 0013. No Ultra column. |

A weight change is not a cache refresh. It does not enter the ray budget. A bake does.

## 4. Alternatives

Three current approaches cover the same job. The pick is at the end.

### 4.1 Enlighten: one indirect term per light, scaled at runtime

Sam Martin and Per Einarsson, "A Real-Time Radiosity Architecture for Video Games", SIGGRAPH 2010 course *Advances in Real-Time Rendering in 3D Graphics and Games* (Geomerics Enlighten). Diffuse transport is linear, so the bounced field of a set of lights is the sum of each light's field. Intensity and colour are a multiply of a precomputed term. Shipped games use this so a lamp can dim without a new radiosity solve.

**Take:** the multiply. A lamp layer stores the indirect field of that lamp alone, at unit white, and the weight is RGB intensity.

**Leave:** lightmap UVs, the edit-time CPU solver, and a whole-scene bake. The samples sit on cache entries we already have. The tracer is the one ADR 0012 already runs. The bake is hidden and capped by ADR 0013. That is a departure from the course: Enlighten's term is an atlas texel baked before the game runs; ours is a cache sample baked under a frame budget.

### 4.2 Lumen: one surface cache, updated by priority

Wright, Narkowicz, and Kelly, "Lumen: Real-time Global Illumination in Unreal Engine 5", SIGGRAPH 2022. One card cache holds the sum. A light change marks pages, and a budget updates them over later frames. Direct light is per pixel. There is no per-light indirect layer.

**Take, for the live layer:** one summed field, priority, a budget that is a ceiling. That is already our cache plus ADR 0013.

**Leave, for a flagged lamp:** a toggle would wait on that budget. The flag exists so the common off, on, and dim do not.

### 4.3 DDGI hysteresis, and MSME catch-up

Majercik, Crassin, Shirley, and McGuire, "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields", JCGT 8(2), 2019. Probes blend new irradiance into old with a hysteresis weight that keeps most of the previous value. The picture is stable. A lamp that turns off lingers for much longer than 200 ms.

Barré-Brisebois, Christensen, et al., Ray Tracing Gems, chapter 25, `MultiscaleMeanEstimator.hlsl` (the listing GIBS also uses). A short mean moves quickly (blend 0.08). The long-mean weight grows only once the short mean has left the long mean, in standard deviations:

```text
catchUp = clamp(smoothstep(rho * max(0.02, iota - 0.2)), 1/256, 1) * vbbr
```

**Take:** this estimator, which the cache already runs, as the live-layer update. The short mean is the confirmation. The long mean is what readers see.

**Leave:** DDGI's probe volume as the store (ADR 0012 already rejected a camera-independent probe grid as the cache). DDGI's slow hysteresis as the settle rule. Dropping history on a change (the hold path's restart, and A-SVGF history rejection). Anthony's rule is that probes do not throw light away.

### Pick

Fast-blend first, on the live layer: the MSME listing, plus a time floor after the listing's catch-up input is already positive (§5). Then lamp layers for flagged static lamps only: Enlighten's multiply, stored in this cache (§6). Lumen's single field remains the live layer for every other light.

The 0.9 lamp-delta cap in today's shader (`k = max(k, min(hint, 0.9))`) is not the pick. It can move the published mean by most of one noisy set. ADR 0012 already records it as a departure. Fast-blend deletes it on this path.

## 5. Fast-blend

This is the first phase. No lamp layer exists yet. The live layer is the whole indirect cache.

A change record (ADR 0013) names the entries a lamp or a mover can affect. Those entries keep their published long mean. New sets update the short mean exactly as the listing says, including the firefly clamp. Readers do not see the raw set.

The time floor applies only when both are true:

1. A change record still covers the entry.
2. The listing's catch-up input is positive: `rho * max(0.02, iota - 0.2) > 0`. The short mean has moved off the long mean by enough that the paper's own `smoothstep` argument has left zero.

Then

```text
k = max(k_listing, 1 - exp(-dt / τ))
```

and the published value is `mix(long_mean, clamped_sample, k)`, as the listing already writes. τ is 50 ms on Medium and High, and 100 ms on Low. Potato does not run GI, so it has no τ.

At 60 fps and τ = 50 ms, `1 - exp(-dt / τ) ≈ 0.283`. The leftover share of an indirect step is `exp(-t / τ)`. It crosses 2% at `t = -τ ln(0.02) ≈ 196 ms`, which is 12 frames at 1/60 s. That 12-frame line is the indirect-lag budget (§10). It is a threshold, not the picture the grader wants, and it is not a claim that every scene has already been timed.

Direct light uses the requested lamp weight in the same frame. Fast-blend does not fade it. A lamp switched off is dark in that frame of the reference. Indirect light may still be closing; the budget is how long that close may take. A scripted fade is the script's own weights. `exp(−t / τ)` is not a target image.

Before the short mean confirms, `k` stays the listing's `k`, whose floor is 1/256. One sample cannot jump the published mean. That is the pop rule.

Screen probes re-read the published mean every frame. They do not reset kept hits. A probe cannot show an in-progress set, because the set is not the published value.

The ray count is the ADR 0013 cap (Low 64 k, Medium 128 k, High 256 k cache rays a frame). Fast-blend does not add rays. If the capped refresh cannot confirm the short mean in time, the settle gate fails and the phase is dropped. The scheduler, once it is on master, only reorders work inside the same cap.

Departures from the listing, on top of the ones ADR 0012 already records (scalar variance, Rec. 601 weights, newborn initialisation, the first-16 cap of `1 / (n + 1)`):

- The time floor above. The listing's `k` is not a function of the clock.
- The 0.9 lamp cap is not applied on this path.

No other term in the listing changes.

## 6. Lamp layers

### 6.1 Flag

A point lamp has an optional flag, default off. The sun cannot set it. A lamp with a non-zero direction (not a point) cannot set it. A moving lamp cannot set it: if a flagged lamp's position or reach changes, the layer is retired and the lamp returns to the live layer until a new bake publishes.

The flag is a scene field on that lamp, a debug checkbox, and an MCP field, in the same shape as other lighting settings (`docs/engine/settings-principles.md`). The public name is `lamp_layer` (`true` or `false`). It is not in this change. No environment variable is required for the grader.

### 6.2 What is stored

The bake traces **that lamp alone**, at unit white, with other lamps and the sun off, and with the static-geometry mask. Movers are invisible to the bake, as ADR 0012 already requires for static indirect. The sample is indirect irradiance only.

Linear diffuse transport is why the sum works: the bounced field of several lamps is the sum of each lamp's field, including further bounces, when each field was solved with the other lamps off. A bake that ran while another lamp was on would include a cross term and would be wrong. The bake does not do that.

### 6.3 Weight

The weight is the lamp's current linear RGB times its intensity, and zero when the lamp is off. Layer combine runs when a published layer's weight changes:

```text
published = live + Σ (weight_i × sample_i)
```

over published lamp layers whose samples sit on that entry. A quiet frame does not rerun combine. Probes and pixels read `published`. When no lamp layer is published, `published` is the live mean and the extra buffer is not allocated.

Direct shading multiplies the same weight in the same frame. A lamp that is off casts no direct light and contributes no indirect.

A scripted glow writes a new weight each frame. Combine tracks that curve. The curve is the script's curve. The engine does not add a second fade.

### 6.4 Publish without a double count

While a layer is baking, the lamp still lives entirely in the live layer, and the bake buffer is not read. On the frame the bake becomes finished, one combine publishes `live_without_this_lamp + weight × baked`. Both sides of that sum are finished before the switch. The previous published value stays up until that frame. The lamp is never in both terms at once, and the picture never shows a half-baked sample.

Turning the flag on at runtime is this hidden transition, not a same-frame operation. Off, on, dim, and glow are same-frame only after the layer has published.

A flagged lamp that loses the flag, or that starts to move, takes the reverse transition: the layer stays published until the live layer has a finished field that includes the lamp, then the layer is dropped in one combine.

### 6.5 Caps

Past the count cap, further flagged lamps are refused and stay on the live layer. Past a lamp's byte cap, that lamp is refused the same way. Refusal is a counter on the debug panel, not a silent quality pop: the lamp behaves as an ordinary live-layer lamp (fast-blend).

| | Potato | Low | Medium | High |
|---|---|---|---|---|
| Flagged lamps that receive a layer | 0 | 4 | 8 | 16 |
| Sample bytes, one lamp, ceiling | 0 | 256 KB | 512 KB | 1 MB |
| Published-sum bytes, ceiling | 0 | 0.5 MB | 1 MB | 2 MB |
| Pool ceiling | 0 | 1.5 MB | 5 MB | 18 MB |

The pool ceiling is the sum of the per-lamp ceilings and the published-sum ceiling. It is a ceiling, not a target. An indoor lamp at 8 m reach is far under it (§9).

## 7. Data and passes

```text
LampLayerDesc (CPU, one per lamp that holds a published or baking layer)
  lamp index, weight (rgb, intensity, on), reach
  state: baking | published | refused
  bytes charged

LampLayerPool (GPU)
  page: lamp slot, cache page id, level, sample count
  samples: fp16×4 irradiance per covered entry (rgb, occupancy in w)
  The pool is sparse. It is not a dense table of every cache slot times every lamp.

Live mean
  Today's long mean on the cache entry. Relights write this, not the published sum.

Published sum
  fp16×4 per occupied entry, allocated only while some layer is published.
  Layer combine is the only writer. Apply, probes, and compose read it.
```

Passes, in order, on a frame where something changed:

1. **Cache update** (existing, capped). Writes live means. Fast-blend's `k` applies here. A baking layer's samples update in the hidden pool, same cap, same finished-value rule.
2. **Layer combine**, only if a weight changed or a layer published or retired this frame. Touches the pages those lamps cover. Capped in milliseconds (§9). Writes the published sum.
3. **Screen resolve** (existing). Reads the published sum. No extra blend.

A frame with no weight change and no bake spends nothing on combine.

Direct shading is unchanged except that it reads the weight it already uses as colour and intensity. The flag does not add a shadow pass.

## 8. Detail levels

ADR 0013 levels stay the only sizes: L0 25 cm, L1 50 cm, L2 1 m, L3 2 m, L4 4 m. A lamp layer does not allocate a level the live cache does not have. The sample is stored on the entry, at that entry's level.

- **Coarsen.** Before a fine entry is retired, its lamp-layer samples are merged into the parent with the same area-weighted merge as the live mean (ADR 0013 §6.4). The parent's sample is a finished value. The bytes move to the parent's charge. The coarse-layer megabyte cap is not increased to hold them; if the parent is evicted, the samples go with it.
- **Return.** A new fine entry starts from the parent's lamp-layer sample, as a finished value, the same way ADR 0013 seeds the live mean. Variance is raised and the entry is young. The L-blend in cache apply already fades levels across the hysteresis band. Combine runs before apply, so the blend sees the summed irradiance, not a per-layer cross-fade.
- **A→B→A.** Walking away and back must not rebake a layer whose samples were merged and seeded. A rebake happens when static geometry or albedo inside the reach changes, or when the lamp's position or reach changes. Camera motion is not a rebake. ADR 0012: stored light does not follow the camera.
- **Hysteresis.** A level that the cache keeps alive keeps its samples. The 3× band and `coarsen_age` are ADR 0013's. This design adds no second band.

Movers still do not dirty a baked layer. Near the camera, screen probes trace all instances and cover the mover's occlusion and bounce. Far from the camera, the baked layer omits the mover. That error is an open question (§15) and a gate (§11): if the path-traced grade fails, the phase does not ship.

## 9. Cost

Figures in this section are estimates from the budgets, not timings. The 4070 and the 1060 have not run this design. Bandwidths used for the combine estimate: GTX 1060 class ~192 GB/s, RTX 4070 class ~504 GB/s. A copy of a lamp's samples plus a read and a write of the published sum is about three times the sample bytes.

**One indoor lamp, order of magnitude.** Reach 8 m, L0 cell 25 cm. Floor entries ≈ π·8² / 0.25² ≈ 3200. Walls at 3 m ≈ 2π·8·3 / 0.25² ≈ 2400. About 5600 entries, 8 bytes each, about 44 KB at L0. Coarser levels are a quarter, a sixteenth, and so on; storing every level would add about a third, still under 64 KB. The per-lamp ceiling (256 KB on Low, 1 MB on High) is an order of magnitude above that room. It exists so a bad reach cannot eat the card.

**VRAM, layer pool at the ceiling, added to ADR 0013's totals.** ADR 0013 High is about 140 MB and Low about 75 MB, before this pool. Medium sits between those two; ADR 0013 does not publish a single Medium total.

| | Potato | Low | Medium | High |
|---|---|---|---|---|
| Layer pool ceiling | 0 | 1.5 MB | 5 MB | 18 MB |
| Lighting total if the pool is full | unchanged | ~77 MB | under 130 MB | ~158 MB |
| 200 MB lighting budget | inside | inside | inside | inside |
| 4 GB card | the pool is 0 | the pool is under 0.1% of the card | same | High's full pool is under 0.5% of the card |

The lighting budget stays ~200 MB with the pool at its ceiling. Typical scenes sit on the "one lamp ≈ 64 KB" side, not the ceiling.

**Milliseconds.** These are ceilings inside the ADR 0013 lighting cap (Potato 1.5, Low 4, Medium 6, High 8 ms on a 4070 at 1080p; older GPUs keep the same cap and the controller shrinks the work). Combine is not added on top of that cap. A frame that spends the combine ceiling has that much less for other lighting work.

| | Potato | Low | Medium | High |
|---|---|---|---|---|
| Fast-blend extra rays | 0 | 0 (uses the 64 k ray cap) | 0 (128 k) | 0 (256 k) |
| Layer combine, whole pool, ceiling | 0 | 0.20 ms | 0.30 ms | 0.40 ms |
| One lamp, weight change, estimate | 0 | < 0.02 ms | < 0.02 ms | < 0.02 ms |
| Direct toggle | inside the 1.5 ms cap | inside 4 ms | inside 6 ms | inside 8 ms |
| Hidden bake | none | inside the relight cap | inside the relight cap | inside the relight cap |

A still scene with published layers and no weight change spends 0 ms on combine. The steady frame stays at the resolve cost plus the ADR 0013 starvation slice.

**Indirect lag, a budget, not a picture target.** Direct light matches the requested weight in the same frame. The table is how long the indirect close may take.

| Preset | τ | Time to 2% of a step | Gate at 60 fps |
|---|---|---|---|
| Potato | — | GI off | metric not applicable |
| Low | 100 ms | ~391 ms | ≤ 24 frames (400 ms) |
| Medium | 50 ms | ~196 ms | ≤ 12 frames (200 ms) |
| High | 50 ms | ~196 ms | ≤ 12 frames (200 ms) |

The grader in this change uses the High line (§10). It does not switch presets.

## 10. Success metrics

Each metric has one threshold. The grader prints one line: `name value threshold PASS`, `FAIL`, or `NOT-IMPLEMENTED`. A missing capture or a missing counter is `NOT-IMPLEMENTED`. It is not a zero, and it is not a pass.

The picture metrics use the lighting-ref settle line and `blur_relative` (`docs/systems/lighting/reference-check.md`): relative luminance `|Yc − Yr| / max(Yr, 0.25 · mean(Yr), 0.02)`, then the box blur. Linear light, before the tone curve. The line is 0.05. No per-scene nudge.

Settled means the blurred error stays ≤ 0.05 for 4 consecutive frames. The reported frame is the first of those four. The clock is the fixed step, 1/60 s, the same step as `light_settle.rhai`.

| Metric | Value | Threshold (High) | What a failure means |
|---|---|---|---|
| `toggle_settle_frames` | first capture frame where `\|frame − expected\|` stays ≤ 0.05 for 4 frames. `expected` is the path-traced or analytic image at that frame's requested lamp weight. | ≤ 12 | The picture is still off the requested image at 200 ms, or it never stays under the line. 12 frames is the indirect-lag budget. Low's design line is 24; this grader scores High. |
| `pop_magnitude` | max over consecutive frames of (peak per-pixel relative step − mean relative step). The step is `\|Y−Yprev\| / max(Yprev, 0.02)`, unblurred, so one speck counts. | ≤ 0.05 | A pixel jumped while the picture did not. A uniform step scores about 0. |
| `fade_deviation` | max over frames of `\|mean(Y) − mean(Yexpected)\| / max(mean(Yexpected), 0.02)`. Same relative-luma unit as the 0.05 line. | ≤ 0.05 | The mean missed the requested-weight image. A 50 ms fade after a step request lands near 0.72. A picture that matches the request is near 0. |
| `hold_flicker` | per-pixel population standard deviation of linear luma on the steady hold. The hold starts at the later of the settle index and the first frame whose requested image matches the last frame's request. If the series never settles, the hold is the last four frames. The value is the nearest-rank 99th percentile. | ≤ 0.02 | The hold shimmers. |
| `dim_error` | blurred error of the last frame against the expected image for that frame's requested weight | ≤ 0.05 | The end picture does not match the request. |
| `layer_same_frame` | blurred error of the first frame after a weight write, against the path-traced reference at that weight | ≤ 0.05 | A published layer did not track the weight in that frame. |
| `layer_bytes` | sample bytes charged to one lamp | ≤ 1,048,576 (the High per-lamp ceiling) | The lamp broke its cap. |
| `finished_read` | count of probe or pixel reads whose value differs from the published sum by more than 0.001 | ≤ 0 | A reader saw a bake buffer or a raw set. |

Potato: the GI picture metrics are not applicable (no GI). The grader does not invent Potato numbers.

`pop_magnitude` stays the speck check. It is blind to a jump that every pixel shares. `fade_deviation` is the miss against the requested-weight image, in the same relative-luma units as its 0.05 threshold. Dividing by the capture's net change is not the score: a picture that overshoots makes that span smaller than the step and the ratio leaves the luma scale. The 50 ms blend is only the 12-frame budget on `toggle_settle_frames`. It is not the curve `fade_deviation` wants.

An instant toggle requests a 1-to-0 step. The expected image is the off image on the request frame. A picture that instead fades with τ = 50 ms fails `fade_deviation` (near 0.72) and can still pass the 12-frame budget. A scripted fade requests that script's weights. Matching the 50 ms blend when the request was a step is a miss.

`layering-gate` (crate `genos-lighting-ref`) implements these comparisons. `--case good` is a step request whose picture matches on the same frame; every row passes. `--case broken` fails every row. `--case instant` is a step request whose picture fades with τ = 50 ms: `pop_magnitude` passes and `fade_deviation` fails. `--case noisy` alternates one pixel on a held picture: `hold_flicker` fails. `--case baseline` with no `--capture` is today's engine with nothing to measure, so every row is `NOT-IMPLEMENTED`. `--capture DIR` reads `frame-NNNN.png` (stored RGB8, the debug PNG). `weights.txt` is one requested weight per frame. `expected-on.png` and `expected-off.png` (or `expected-on-reference.png` and `expected-off-reference.png`) are the path-traced or analytic endpoints. The expected image is `off + weight · (on − off)` in linear light. Without both the weights and the endpoints, settle, fade, and dim stay `NOT-IMPLEMENTED`. Layer rows stay `NOT-IMPLEMENTED` until a lamp layer exists.

## 11. Test plan

No tolerance widened to hide a miss. No unfilled tint, no display ramp, no history drop used as a way to pass the settle line. The reference is the CPU path trace in `crates/debug/src/reference.rs`, the same one `lighting-ref` already grades. Fixtures in this change stand in for that picture so the comparison can fail on purpose; they are not a substitute for the trace when a phase ships.

Commands this change adds:

```sh
cargo run -p genos-lighting-ref --bin layering-gate -- --case good
cargo run -p genos-lighting-ref --bin layering-gate -- --case broken
cargo run -p genos-lighting-ref --bin layering-gate -- --case instant
cargo run -p genos-lighting-ref --bin layering-gate -- --case noisy
cargo run -p genos-lighting-ref --bin layering-gate -- --case baseline
```

`--out` sets the directory. The default is `./genos-debug/gi/layering/`. The tool writes `{case}.json` there. `--capture` writes `baseline.json`. It does not write under `$HOME`. There is no environment mirror. Exit 0 when no row is `FAIL`. Exit 1 when any row is `FAIL`. `NOT-IMPLEMENTED` does not fail the process. Exit 2 is a bad argument or an I/O error.

`cargo test -p genos-lighting-ref layering` locks the same rows: the good fixture passes each comparison, the broken fixture fails each comparison, a 50 ms picture after a step request fails `fade_deviation` while `pop_magnitude` passes and the settle stays inside the 12-frame budget, a scripted weight curve is the target (matching it passes, a 50 ms picture against it fails), the noisy hold fails `hold_flicker`, and the baseline without a capture stays `NOT-IMPLEMENTED` with a null value. `fade_deviation` in those tests stays under 2, the same relative-luma scale as the 0.05 threshold.

`examples/stress/scripts/lamp_toggle_frames.rhai` freezes the stress scene, writes `frame-0000.png` with a lamp on, turns that lamp off, and writes `frame-0001.png` through `frame-0036.png`. It then path-traces the off lamp and the restored on lamp at 320×180 (`expected-off-reference.png`, `expected-on-reference.png`). Paths are relative to `--report`. The script cannot write `weights.txt`. The shell writes it: frame 0 is weight 1, and the later frames are weight 0. That file is the request. The traces are the endpoints.

```sh
genos-stress --proof --no-panel --size 320x180 --preset high \
  --script examples/stress/scripts/lamp_toggle_frames.rhai \
  --report genos-debug/gi/layering/capture
{ echo 1; for i in $(seq 1 36); do echo 0; done; } \
  > genos-debug/gi/layering/capture/weights.txt
cargo run -p genos-lighting-ref --bin layering-gate -- \
  --capture genos-debug/gi/layering/capture \
  --out genos-debug/gi/layering
```

On a machine with no GPU that command runs under lavapipe and a headless Weston. The warmup is 30 frames, so the capture is the toggle, not a soaked cache. Timing on lavapipe is not a result. The expected image is the path trace at the requested weight, not the held end of the engine frames and not `exp(−t / 50 ms)`.

One lavapipe run (320×180, preset high, lamp 0 turned off, 37 frames) scored exit 1. The endpoints are path traces at 16 spp and 3 bounces. They stopped at `max_spp`, not the 40 s cap. Off noise was 0.255, on noise was 0.231.

```
toggle_settle_frames 37 12 FAIL
pop_magnitude 0.7340 0.0500 FAIL
fade_deviation 0.2813 0.0500 FAIL
hold_flicker 0.0028 0.0200 PASS
dim_error 0.6190 0.0500 FAIL
layer_same_frame NOT-IMPLEMENTED 0.0500 NOT-IMPLEMENTED
layer_bytes NOT-IMPLEMENTED 1048576 NOT-IMPLEMENTED
finished_read NOT-IMPLEMENTED 0 NOT-IMPLEMENTED
```

`fade_deviation` is 0.2813, on the same scale as 0.05. Frame 0 still requests the lamp on. Its mean linear luma is 0.158 against the on trace at 0.220, and that ratio is the 0.2813. The 30-frame warmup is not a soak. Frame 1 requests the lamp off. Its mean miss against the off trace is 0.020. The mean then climbs: frame 36 is 0.161 against the off trace at 0.141. Blurred error never holds for four frames, so settle is the frame count, 37. The last frame's blurred error is 0.6190. The trace noise is part of that blurred error. The last four frames are calm. The layer rows have no lamp layer.

Later phases add rhai gates under `debug/lighting/scripts/gates/`. The host cannot toggle a layer that does not exist, so those scripts wait. Each script soaks, records a settled frame, performs one change, and returns to the start, which is the existing test rule.

| Script | When | Pass |
|---|---|---|
| `live-blend.rhai` | Fast-blend. Unflagged lamp, colour or intensity change, camera still, sun frozen. | `toggle_settle_frames`, `pop_magnitude`, `fade_deviation`, `hold_flicker`. `dim_error` against the path trace. `finished_read` is 0. |
| `layer-weight.rhai` | A published layer. Weight 1 → 0.5. | `layer_same_frame` and `dim_error` the same frame. Cache-ray counter does not rise. |
| `layer-toggle.rhai` | Weight 1 → 0 → 1. | Same-frame match to a reference with the lamp off, then on. `temporal_flicker.rhai` while the weight is held. |
| `layer-fade.rhai` | Scripted weight curve over 30 frames. | Each frame's blurred error against a reference at that weight ≤ 0.05. No cache rays. |
| `layer-lod.rhai` | Walk A→B→A across a level boundary. | No rebake counter spike. `pop_magnitude` ≤ 0.05. `layer_bytes` stable. |
| `layer-mover.rhai` | A box crosses a layered lamp's beam inside 10 m. | `dim_error` ≤ 0.05 against the path trace that includes the box. A miss drops the phase. |

The owner's 4070 runs the picture grades that need a path trace. Lavapipe runs the grader's fixtures, the lamp-toggle capture, and checks that the JSON appears. Timing on lavapipe is not a result.

Debug probes the implementation has to grow, named now so a phase cannot land without them:

| Failure | Probe |
|---|---|
| Settle slower than the gate | `toggle_settle_frames` in `layering-gate`; `LIGHTING_MS` already on the debug binary |
| Speck or page pop | `pop_magnitude`; overlay `blend-k` (the `k` that was applied) |
| Mean misses the requested weight | `fade_deviation` |
| Hold shimmer | `hold_flicker` |
| Wrong brightness after a dim | `dim_error` against the path trace |
| Weight ignored | `layer_same_frame`; overlay `layer-weight` |
| Pool overrun or a refused lamp | `layer_bytes`; counter `lamp_layer_refused`; MCP `get_lamp_layers` |
| Reader saw a bake or a raw set | `finished_read` |
| Double count at publish | `dim_error` on the publish frame, plus `finished_read` |
| LOD pop on the walk back | `layer-lod.rhai`, `pop_magnitude` |

`get_lamp_layers` is the later MCP command: per lamp, state, bytes, weight, and whether the last read matched the published sum. It is not in this change.

## 12. Phases

Each phase is its own change, with an A/B against the path trace. A miss drops the phase. None of them edit the phase-3 branch; they wait until those files are free, or they touch only the cache blend.

| Phase | What lands | Gate |
|---|---|---|
| P0 (this change) | This design, the glossary entries, `layering-gate`, `lamp_toggle_frames.rhai` | Fixtures: good passes, broken fails, a 50 ms picture after a step request fails `fade_deviation`, noisy flicker fails. Baseline without a capture is `NOT-IMPLEMENTED`. `--capture` with weights and endpoints scores the picture rows against the requested weight. |
| P1 | Fast-blend `k` floor on the live layer. Removes the 0.9 lamp cap on that path. | `live-blend.rhai` on High and Low. Pop and settle. No new rays above the ADR 0013 cap. |
| P2 | Flag, hidden bake, layer combine, weight. | `layer-weight.rhai`, `layer-toggle.rhai`, `layer-fade.rhai`. `layer_bytes` under the cap. `finished_read` is 0. |
| P3 | Merge and seed of lamp-layer samples with ADR 0013 coarsening. | `layer-lod.rhai`, `layer-mover.rhai`. |

## 13. Networking

The implementation copies these lines into the lighting system record. They are the contract; gameplay networking still waits.

- **Replicated state:** the flag and the layer weight (colour, intensity, on or off). Irradiance samples, the published sum, and probe reads stay on the client that shades.
- **Authority:** the machine that owns the lamp. A server-owned lamp sends the weight. Clients do not each bake a different layer and then fight.
- **Prediction / rollback:** a predicted weight is a local combine. A rollback restores the weight and combines again. It does not replay rays.
- **Determinism:** a published bake matches for the same scene, lamp, level set, and sample budget. Fast-blend uses the frame's fixed `dt`, not the wall clock.
- **Headless server:** does not bake, combine, or shade. It replicates the weight only.

## 14. Open questions

1. τ = 50 ms (High, Medium) and 100 ms (Low) is the indirect-lag budget in §5. It is not the image a requested weight should show. The 4070 and the 1060 have not confirmed that the capped refresh can feed the short mean inside the first couple of frames. If it cannot, P1 fails its gate and τ does not get quieter to hide it.
2. A mover in front of a layered lamp is missing from the bake beyond the screen-probe range. `layer-mover.rhai` is the measurement. No error budget is asserted for the far field until that grade exists.
3. Sample format is fp16×4 in §7. rgb9e5 would halve the sample bytes and can clip. P2 picks the one that passes `dim_error`, and records the other as rejected.
4. A script that sets the flag at runtime pays the hidden transition in §6.4. Whether a game may set the flag, or only the scene may, is not decided. The default is off either way.
5. Specular is out of scope. The multiply is exact for linear diffuse. A later specular pass has to say whether it reuses the lamp layer or traces its own reflection.

## 15. Glossary

One meaning each. The glossary entries are the authority; this list matches them.

| Term | Meaning |
|---|---|
| Lamp layer | Finished indirect irradiance of one static point lamp, at unit white, on the cache entries that lamp reaches. |
| Layer weight | RGB intensity multiplied onto a lamp layer. Off, on, dim, and glow change the weight and do not rebake. |
| Live layer | The one indirect cache for every light that has no lamp layer. |
| Fast-blend | Published irradiance closing on a confirmed new mean with time constant τ. Not a display fade. |
| Layer combine | The pass that writes the published sum. Readers see that sum only. |

Coarse layer remains the ADR 0013 name for L3 and L4. The traced field and the shown field (experiment S72P) are not lamp layers.
