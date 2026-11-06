# 0028: Bloom conserves energy in the dual-filter chain

Status: **Proposed** (29 Oct 2026). Anthony Poschen reviews this before any renderer change. The CPU reference is `genos-math::bloom`. `genos-render` does not call it. The shipped pass stays the additive pyramid in `crates/render/src/hdr_output.rs` ([ADR 0015](0015-hdr-output.md)).

## Context

ADR 0015 picked the Call of Duty dual filter (Jorge Jimenez, SIGGRAPH 2014) and the Karis weight on the first downsample (Brian Karis, same course). The shipped combine is `(scene + bloom_intensity × pyramid) × exposure`. The pyramid sum grows with the level count, and the intensity adds light. A flat field does not stay a flat field.

ADR 0023 fixed the presets as Low, Medium, High, and Ultra. ADR 0027 maps HDR onto those presets and gives Ultra a 7th bloom level. The post ceilings in the HDR design are for bloom plus the display pass (1.50 ms on the 4070 at 1440p). They are not a bloom-only budget, and they are not measured.

"Physically based" here means the filter redistributes light. It does not mean Spencer's eye point-spread function, and it does not mean a measured lens. ADR 0015 already rejected both.

## Decision

1. The kernel stays the Jimenez chain: 13-tap downsample (weights sum to 1), 3×3 tent upsample (weights sum to 1), thresholdless, on the scene-referred image, before exposure. A convolution bloom and an eye point-spread function stay rejected.
2. The combine conserves the sum of the image. The chained upsample is divided by the level count. Unless Karis is on, one scalar then makes the blur's sum equal the scene's sum. The output is `scene × (1 − scatter) + blur × scatter`. `scatter` is clamped to 0..=1. A flat field is unchanged at any scatter. `scatter` 0 returns the scene.
3. Samples outside the image repeat the edge texel. A zero border darkens the edge and leaks the sum. The shipped `bloom_downsample` still returns 0; this reference does not replace it.
4. Karis stays available (`Params::with_karis`) and is off on every preset below. The Karis path does not restore the sum: the weight exists to remove firefly energy. That is a departure from decision 2, and it is not the default.
5. Preset knobs are a starting point, not a graded picture. Scatter is not `bloom_intensity`.

| Preset | Levels | Scatter | Karis |
| --- | --- | --- | --- |
| Low | 4 | 0.04 | off |
| Medium | 5 | 0.05 | off |
| High | 6 | 0.05 | off |
| Ultra | 7 | 0.06 | off |

6. Cost is a ceiling. The numbers below are a bandwidth model, not a 4070 measurement. RGBA16F is 8 bytes. Each downsample reads its source once and writes the half image. Each upsample reads the coarse image and the finer level and writes the finer level. The last step reads the half-resolution blur and the scene and writes the output. The 13-tap and the tent are assumed to hit cache, so they are not counted 13 times. Lavapipe time is not this score.

| Preset | Where | Pyramid | Traffic at 1440p | Paper | Ceiling |
| --- | --- | --- | --- | --- | --- |
| Low | GTX 1060 6 GB, 192 GB/s, 1440p | 9.338 MiB | 137,088,000 bytes | 0.714 ms | 1.00 ms |
| Medium | RTX 4070, 504 GB/s, 1440p | 9.366 MiB | 137,491,200 bytes | 0.273 ms | 0.30 ms |
| High | RTX 4070, 1440p | 9.373 MiB | 137,591,680 bytes | 0.273 ms | 0.30 ms |
| Ultra | RTX 4070, 1440p | 9.374 MiB | 137,616,320 bytes | 0.273 ms | 0.30 ms |

   The 4070 budget is under 0.30 ms. The paper floor at High is 0.273 ms. Extra levels are cheap: Low to Ultra adds about 0.5 MB of traffic. The full-resolution mix dominates. A cache miss fails the 0.30 ms ceiling, and adding a level does not buy that miss back.

7. The reference is `genos-math::bloom`. `bloom` is the mixed image. `blur` is the image mixed in. `filter_gain` is the restore scalar before it is applied (`1` on a flat field). `compare` diffs an RGB or RGBA `f32` readback against that image. `to_dump` / `from_dump` round-trip the same pixels as text (`genos-bloom 1`). `fixture_input`, `fixture_params`, and `fixture_output` are the fixed buffer a later GPU pass diffs. `pyramid_traffic_bytes` and `traffic_ms` are the budget probe.

## Departures from Jimenez 2014

The paper is Jorge Jimenez, "Next Generation Post Processing in Call of Duty: Advanced Warfare", SIGGRAPH 2014, the dual-filter section (13-tap downsample, 9-tap tent upsample). Karis, "High Quality Bloom", is the same course.

- The paper's reconstruction sums the levels and an artist intensity scales that sum. The sum adds light, and it grows with the level count. We divide by the level count, restore the sum, and replace a fraction of the scene. A flat field has gain 1.
- Edge samples repeat the edge texel. The course does not pin the out-of-bounds rule. The shipped engine returns 0.
- Karis is off in the presets. The course puts `1 / (1 + luma)` on the first downsample. The linear chain is what the energy tests grade. Karis is a named option and skips the restore.
- There is no per-frame highlight check in the reference. It is a pure function. ADR 0015's reuse rule stays a renderer concern for the integration change.
- No artist threshold and no soft knee. Same as ADR 0015. The slides argue for thresholdless; some ports add a knee. We do not.

## Departures from ADR 0015

ADR 0015 decision 5 is the additive pyramid with Karis on the first downsample. This ADR keeps the kernel and the order (scene-referred, before exposure) and changes the combine. It does not replace the shipped pass. The two CPU chains both exist until this ADR is accepted:

- `hdr_output::bloom_pyramid`: additive, Karis on, zero outside the image. The GPU pass follows it.
- `genos_math::bloom`: scatter mix, Karis off, clamp-to-edge, sum restored.

## Networking

Bloom is a present-client post process. The reference is a pure function.

- Replicated state: none.
- Authority: the client that presents. A server does not bloom.
- Prediction / rollback: none.
- Determinism: the value is fixed by the image and the params.
- Headless server: the same function. No swapchain and no picture.

## Probes

The reference has no overlay yet. These are the probes that exist now. The lighting binary's Output page and `get_hdr` gain a bloom-only time when the pass is integrated, separate from `hdr_ms` (bloom plus display).

| Failure | Probe |
| --- | --- |
| The filter creates or destroys light | `total`, `compare.energy_rel`. Test `a_varying_field_keeps_its_sum` |
| A flat field changes with scatter | `filter_gain` (must be 1) and the flat-field test |
| A firefly survives the Karis option | `blur` with `with_karis`, test `karis_removes_firefly_energy` |
| The 4070 or 1060 ceiling is missed on paper | `pyramid_traffic_bytes`, `traffic_ms`, test `preset_budgets_hold_at_1440p` |
| A later GPU buffer disagrees | `compare` (RGB or RGBA `f32`). The fixed image is `fixture_output`. `to_dump` writes it |

## Rejected

- Integrating this chain into `hdr_bloom.comp` in the same change. Anthony reviews the ADR first.
- A new crate. ADR 0027 rejected `genos-hdr` for pure functions. `genos-math` already holds the headless curves.
- FFT convolution and Kawase. ADR 0015 rejected both. The 1060 does not pay for an FFT, and Kawase has a weaker energy contract.
- Spencer, Shirley, Zimmerman, and Greenberg, SIGGRAPH 1995, as the kernel.
- Using `bloom_intensity` as the name of `scatter`. The shipped knob adds light. Scatter replaces a fraction. The glossary entry is **Bloom scatter**.

## Consequences

The renderer is unchanged. Post stays outside the lighting caps (ADR 0013). The 0.30 ms ceiling is bloom only. The display pass stays inside the existing `post_ms_device` ceiling of 1.50 ms. A wrong weight or a combine that adds light fails `cargo test -p genos-math` before any GPU picture.

## Open questions

1. Accept the scatter mix, or keep ADR 0015's additive intensity? Scatter dims a highlight by the scattered fraction. The additive pass brightens the image.
2. Karis on the first downsample (ADR 0015) or off, so the energy gate can pass? Karis removes firefly energy on purpose. The linear chain does not.
3. Is the restore scalar part of the GPU pass, or only a CPU correction until a 1440p readback shows the raw filter is already within the gate?
4. Low's bloom gate is the 1060 at 1440p. ADR 0023 tunes Low for the 1060 at 1080p, plus FSR. Which resolution is the gate? A pass that meets 1440p meets 1080p; the 1440p number is the heavier one.
5. Scatter 0.04 / 0.05 / 0.05 / 0.06 is ungraded. The 4070 pictures decide it.
6. Clamp-to-edge versus the shipped zero border. The energy contract needs clamp. The shader should be read before a port copies either rule.
