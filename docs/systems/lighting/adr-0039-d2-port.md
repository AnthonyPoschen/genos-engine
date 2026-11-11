# ADR 0039 D2: port the existing C1–C5 direct ReSTIR

Status: **Proposed**, for Anthony's review. Not accepted. Merging this note does not authorize implementation, does not amend ADR 0039, and does not revive the world-cell light-list table from PR #164.

The tree this note is written against is `d7f7bd2`. The lighting system record stays as it is until an implementing commit.

## Intent

On the Reuse presets (`low`, `medium`, `high`), screen-pixel point lamps shade from the existing ReSTIR DI path in PR #120 (ADR 0022 stages C1–C5), at every point-lamp count. The product exact-sum shortcut is deleted. `LampShadeGate::Exact` remains, and it is the labelled reference. The port uses that path's reservoir. It adds no new reservoir and no cell table. The sun, the synthetic fire, and the hit shade stay on the current code. A camera move keeps a pick when depth and normal still match.

`LampPickMode::Off` (potato, day-cycle) still traces every lamp. "Every lamp count" means the Reuse product path, including counts at or under 8.

## D1, corrected record

ADR 0039 still says the GI-on direct pass is 13.4–14.3 ms against 4.3 ms GI-off because pixels trace about 4.8 lamp shadow rays (17.66M a frame), and that the extra ~10 ms is unexplained (context, and the D1 row's −8 ms estimate toward a ≤ 5 ms gate). That sentence is the pre-split reading. This note leaves the ADR text in place. It records the later measurements. It contains no new 4070 timing, and it does not show the ≤ 5 ms gate as met.

| Record | What it measured |
|---|---|
| 30 Oct 18:00, commit `ecd8efce`, `docs/systems/lighting/direct-bucket-breakdown.md` (absent on `d7f7bd2`) | 4070, 2560×1440 stress, `GENOS_LIGHTING_LAMP_PICK_DIAG=1`. `lamp_px` is 17.66M (~4.8 per pixel). Pixels are ~4.4 ms with GI on and with GI off (GI-off direct 4.27 ms). The GI-on extra ~9.8 ms is the kept/fill half of the same `pixels + 2×kept` dispatch: with direct off, those threads alone are `light_hit` 9.84 ms. Probe picks (`LIGHTING_PROBE_PICKS`) save ~1.5 ms. Kept-hit shadow rays are ~0.7M. |
| PR #123, merged `fceaf77f` (30 Oct) | `shadows=` added every `B_DIRECT` and `B_HIT` time into `light=` a second time. The 30 Oct C5 table's "shadows 15.7 + initial …" counted that work twice. After the fix, `light_px` is pixels plus kept hits, and `light_hit` is cache-ray hits. |
| PR #120 comment, 30 Oct 02:41, build `2469d274`; written up in PR #122 | Flag off `light=` 13.349 ms. Flag on 25.815 ms (initial 7.015 + temporal 0.466 + spatial 4.449). Lighting total 32.620 vs 45.276 ms. That run is before the shade replacement and before the 18:00 split. The same-day comment on `6866b41` reads the leftover ~13.9 ms as probe and cache hits plus suns. |
| PR #167, merged `eca4827f` (5 Nov), ancestor of `d7f7bd2` | Hits off the screen stop walking every lamp. Lavapipe, 640×360, high: lamp entries 3.25M → 1.73M, hit lamp entries 1.88M → 0.37M. The PR states that pixel lamp rays were already the same with GI on and GI off. The PR body asked for a 4070 timing before merge. That timing is not in the PR. |

17.66M is the measured pixel shadow-ray count. It is the pixel half, which matches GI-off. It is not an unexplained GI-on ray blow-up.

## D2 design

Take the checker and the live pixel shade from PR #120 (`cursor/restir-di-live-a877` at `67dfc320`). Leave the rest of that branch. It is 337 commits and 260 files onto `lighting-v3`, including later merges that are not this change.

Screen point lamps use that one reservoir: `restir_words`, about 64 bytes per pixel plus the prefix, about 236 MB at 1440p, allocated only while the flag is on. That figure is already in the C5 plan and as an estimate in ADR 0039. The C5 shade replaces the product pairwise loop for those pixels. PR #122's first finding was a build that ran both, and the direct pass was the sum of the two.

### Files the port takes

Workspace members to add:

- `crates/restir-ref/` — CPU reference (`restir.rs`, `tree.rs`, `shade.rs`, `reservoir.rs`) and tests `live_estimator.rs`, `turn_reference.rs`, `turn_reprojection.rs`
- `crates/restir-gpu/` — C1–C4 checker: `restir_initial.comp`, `restir_temporal.comp`, `restir_spatial.comp`, `restir_tree.glsl`, `restir_sample.glsl`, `restir_merge.glsl`, `restir_spatial_ops.glsl`, plus `tests/initial.rs`, `temporal.rs`, `spatial.rs`, `tree.rs`

Live wiring from the same head:

- `crates/render/shaders/restir_live.comp`
- `crates/render/src/restir_live.rs`
- `crates/render/tests/restir_di.rs`

Stage text stays in `docs/systems/lighting/restir-di-plan.md` on that branch. This note does not copy it.

### Files a later port edits on this tree

- `crates/render/shaders/lighting_direct.comp` — a screen pixel's point lamps come from the reservoir sample (`p(y) · V(y) · W`, one shadow ray). The sun trace stays. Hits, fills, and cache-ray hits stay.
- `crates/render/shaders/lighting_lamp_pick.glsl` — delete the product exact shortcut (`LAMP_PICK_EXACT_LIT`, the quiet-held exact record). The history test stays the constants in choice 1 unless that choice is accepted otherwise.
- `crates/render/src/lighting_dispatch.rs` — `direct_shade_plain` stops selecting the plain module for `LampPickMode::Reuse`.
- `crates/render/src/lighting_gpu.rs` and `crates/render/build.rs` — drop the `LIGHTING_DIRECT_PLAIN` bind that Reuse uses at ≤ 8 lamps.
- `examples/stress/src/noise_gate.rs` — the exact baseline stays a named gate mode.

The synthetic fire stays on the existing shade. It is not a light-tree leaf. That is the C5 departure, kept.

### Product exact branch, deleted

On the Reuse product path, delete all three:

- `shade_exact` when `quiet_held` or `point_lit <= LAMP_PICK_EXACT_LIT` (8) in `lighting_direct.comp`
- `direct_shade_plain` returning true for `LampPickMode::Reuse` at ≤ 8 point lamps
- the `LIGHTING_DIRECT_PLAIN` SPIR-V used for that Reuse case

`LampShadeGate::Exact` (bit 19 of `pc.lamp.y`) stays. It sums every point lamp inside the shade. It is the reference, not a preset. `LampShadeGate::Random` and `Broken` stay gate tools. `direct_shade_plain` already returns false for every gate other than `Product`, so the reference does not use the plain module.

### Retained

- Sun trace (`sun_lamp` in `lighting_direct.comp`).
- Synthetic fire, on the current shade.
- Hits, fills, and cache-ray hits, including the probe-hit cut already in PR #167.
- A pick kept across a camera move when the depth and normal test passes.

## Review choice 1 — history match

**Proposed. Not accepted.**

Keep the current product test in `lamp_pick_reproject`: one reprojected pixel, relative depth `LAMP_PICK_DEPTH_REL` 0.02, absolute `LAMP_PICK_DEPTH_ABS` 0.05 m, normal dot `LAMP_PICK_NORMAL_MIN` 0.9.

The C5 head widens that test. At `67dfc320`, `restir_live.comp` sets `RESTIR_DEPTH_REL` to 0.1, keeps absolute 0.05 and normal 0.9, and searches the pixel plus four neighbours (`RESTIR_TEMPORAL_TRIES` 5). The shaded-mean reprojection uses the same 0.1 (`RESTIR_MEAN_DEPTH_REL`). The shader comment cites Bitterli 2020 §6 and says 2% rejected oblique walls. That comment is not an accepted change of the product test. This note keeps 0.02 / 0.05 / 0.9.

## Review choice 2 — shaded-colour mean

**Unresolved.**

At `67dfc320` the live shade stores a running mean of the shaded sample, cap `RESTIR_MEAN_CAP` 32, tag `LAMP_GEO_RESTIR_MEAN`, and that mean is the colour on screen (commit `af94ef8`; #120 comment, 30 Oct 23:25). The product mean on this tree (`LAMP_PICK_MEAN_CAP`, 4096) is the noise-gate bit only. The lighting README says production does not set that bit, and that the mean is the gate only.

Porting the 32-frame mean must not make it the graded picture. Recommendation, pending Anthony: store the raw reservoir shade and the mean as two outputs. Near, turn, and `grid_worst` stay on the raw picture. An averaged diagnostic is not a pass of those gates.

## Gates, unchanged

| Gate | Value | Where it is set |
|---|---|---|
| `grid_worst` | ≤ 0.15 | C5 plan; PR #120 |
| Turn near error | ≤ 0.403, the ReSTIR-off baseline | ADR 0039 D2 |
| Direct pass at 1440p | ≤ 3 ms | C5 plan |
| GI-on light pass | ≤ 5 ms | ADR 0039 D1. Not shown met by the records above |

### Five-lamp scene and the many-lamp scene

`docs/systems/lighting/oracle-harness.md`: the main scene has 5 point lamps, so `spin` and `switch` shade the exact sum and their `grid_worst` is 0. A one-sample ReSTIR cannot score 0 on those two rows. The #120 comment of 30 Oct 20:05 (lavapipe, `c2881ba`) recorded that and left the decision open. The `lamps` profile (`lamps-spin`, `lamps-switch`, 32 lamps, PR #143) is the scene that grades a path which is not the exact sum. On that profile, flag-off is not an exact sum either.

Both scenes stay in the harness. This note does not drop the five-lamp rows, does not make `lamps` the only acceptance scene, and does not change 0.15. Which row is the pass, once the product exact branch is gone, waits on review.

## Known C5 failures

These are the recorded runs. This note does not re-run them.

4070, 30 Oct, PR #122 and the #120 comment on `2469d274`, oracle `quick`, flag on:

- `ref_err_near` worse than master on sun-x100, sun-x10, turn, and spin. Turn near 0.5925 against flag-off 0.4753. The ADR bar is 0.403. Flag-off that day was already above 0.403. Neither number becomes a new gate here.
- `grid_worst` from 0.27 to 0.89, against ≤ 0.15. sun-x100 and sun-x10 already miss 0.15 on master (about 0.56 and 0.72). That miss sits outside C5. The gate stays 0.15.
- Direct pass 25.8 ms, against ≤ 3 ms.

Lavapipe `lamps` profile after the 32-frame mean (`af94ef8`). Lavapipe times are not a result:

- `lamps-switch` `grid_worst` 0.231 on the frame after the switch. Longest run 1.
- `lamps-spin` `grid_worst` 0.610, beside flag-off 0.655. Longest run 1, down from 239.

Commit `5cf48e2c` on the C5 branch reports the initial pass from 5.96 ms to 3.09 ms with 4 candidates instead of 8. That is one stage. It is not a pass of the 3 ms direct-pass gate.

## Staged port

Each stage is a later change, after this review. This document is not that change.

1. **Checker only.** Add `restir-ref` and `restir-gpu`. Reservoir diffs match the CPU reference, as the C5 plan states for C1–C4. The live flag is absent.
2. **Live flag, default off.** Screen pixels only. Sun, fire, hits, fills, and cache rays stay. History rejection is the test in choice 1 until Anthony decides it. Lavapipe `restir_di` is graded against `LampShadeGate::Exact` on the gate scenes. The grade reads the raw shade.
3. **Delete the product exact branch** on Reuse, at every lamp count. A test shows `direct_shade_plain` is false for Reuse at 8 and at 9, and that `LampShadeGate::Exact` still sums every point lamp. A product frame does not take `shade_exact`.
4. **Raw and mean, if the mean code is ported.** Two stored colours. The oracle and the noise gate read the raw shade until choice 2 is decided.
5. **4070 after the lavapipe rows pass.** The session is the owner's. Report `ref_err_near`, `grid_worst`, turn, and `light=` beside the gate table. A miss stays a miss.

## Held for this note

- No owner-PC run belongs to the change that adds this file.
- A merge of the note is the review of the design. The port is a separate review.
- The gate numbers above stay. The five-lamp / many-lamp question is not a silent swap of scenes.
- The lighting README and the glossary change in the implementing commit.
- Networking for that commit, copied from the C5 plan: replicated state is none (reservoirs stay on the rendering client); authority is the client that shades; prediction and rollback are none; determinism is one PCG stream per pixel, not the checker's single stream; a headless server does not run the live pass.

## Sources

- ADR 0039, the GI-on direct sentence, the D1 and D2 rows, and the −8 ms estimate. Accepted text, left as written.
- `ecd8efce`, the 30 Oct 18:00 direct-bucket note.
- PR #123 `fceaf77f`, PR #120 `67dfc320`, PR #122, PR #167 `eca4827f`, PR #143 (the `lamps` profile). PR #164 is the cell-table proposal this design leaves closed.
- `docs/systems/lighting/oracle-harness.md` and `docs/systems/lighting/adr-0039-remaining.md` (D2 is the existing ReSTIR path, not a second reservoir).
