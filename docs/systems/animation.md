# Animation system (design)

Status: proposed, 27 Oct 2026. Updated 10 Nov 2026: the cook record and the Low table are [ADR 0038](../adr/0038-animation-adopts-design-and-fills-gaps.md). The replication bytes are not part of that update. Owner: Engine. Binds to [ADR 0012](../adr/0012-gi.md) (a moving mesh keeps an object-space proxy), [physics plan](physics/plan.md) (foot IK is the visual foot plant), [asset plan](assets/plan.md), [agent rubric](../agent-rubric.md).

## 1. Goal
Play glTF skinned characters with blended clips, a data-driven state machine, root motion and later IK, on the GPU, deterministic enough that a server and clients agree on poses that matter to gameplay.

## 2. Where it sits today
| Piece | State | Code |
|---|---|---|
| Text TRS clip, slerp sampling | done (one node) | `Animation::sample` in `crates/load/src/animation.rs` |
| Scripted motion | done | `crates/scene/src/codimation.rs`, `.rhai`. This is not a clip. |
| glTF skins, joints, clips | loaded | `crates/load/src/gltf.rs` reads skins, `JOINTS_0`/`WEIGHTS_0`, and linear/step clips. Morph targets stay unsupported. |
| glTF skins in the cook | missing | ADR 0038 Addition A (`.gskin`). |
| Skinning, blending, state machine, GPU skin | missing | No compute skin pass yet. |

## 3. Prior art
- glTF 2.0 spec (Khronos): skins, `inverseBindMatrices`, `JOINTS_0/WEIGHTS_0`, linear/step/cubic-spline samplers, morph targets.
- Skinning: linear blend skinning; Kavan et al., "Geometric Skinning with Approximate Dual Quaternion Blending", TOG 2008.
- Runtime: ozz-animation (Guillaume Blanc) SoA sampling, local-to-model jobs; ACL (Nicholas Frechette) clip compression; Unreal AnimGraph/state machines; Unity Mecanim.
- Motion: Clavet, "Motion Matching", GDC 2016 (later option). Root motion: Unreal/Unity root-motion extraction.
- IK: FABRIK (Aristidou & Lasenby 2011), analytic two-bone IK (Unreal TwoBone).
- Networking: Overwatch GDC 2017 (Ford) server-side poses for hit-reg; Valve Source lag compensation.
- GPU skinning: compute pre-skin into a vertex buffer (Unreal GPU skin cache) vs vertex-shader skinning.

## 4. Options
| Area | Option A | Option B | Pick |
|---|---|---|---|
| Skinning | vertex-shader LBS each pass | **compute pre-skin into a buffer once/frame** | B: the G-buffer and the shadow pass read one skinned buffer |
| Blending | 1D/2D blend spaces + layered masks | full node graph | A (graph is data, small node set) |
| State machine | hard-coded | **data file (`.ganim` RON/JSON) + Rhai conditions** | data |
| Clip storage | raw f32 keys | ACL-style quantized (later) | raw first, quantize when memory shows |
| Dual quaternion | yes | LBS only | LBS default, DQS per-skin flag later |

## 5. Recommendation
Stage 1: glTF skins and clips through the cook (ADR 0038 Addition A), CPU sampling (SoA, fixed 60 Hz sim tick), 1D/2D blend spaces, crossfade, compute linear-blend pre-skin. GI keeps the proxy for a skinned mesh. A BLAS refit is not part of this design. Stage 2: data state machine with Rhai conditions, root motion, anim events/notifies. Stage 3: two-bone IK + FABRIK for foot placement (physics hook), look-at. Later: morph targets, motion matching, compression.

**Deviation flags.** None from the papers. Sampling on the CPU (not GPU) is deliberate so the server, which has no device, runs the same code.

## 6. Determinism for networking
- Sim tick fixed; pose time = tick × dt, never wall time. Fixed-point-free but order-stable f32 math, no SIMD reassociation in the gameplay pose path.
- Replicate state-machine state id, clip ids, normalized time and blend weights (quantized 16-bit), not bone transforms.
- Server evaluates only gameplay bones (root, hit-boxes) on `LoadProfile::Sim`; clients evaluate the full skeleton. Root motion is server-authoritative, predicted on the owning client, reconciled with rollback of state-machine time.
- Rubric lines: replicated = anim state; authority = server; prediction = owner client; determinism = tick-based; headless = gameplay bones only.

## 7. Data and settings

`AnimationSettings` implements `EngineSettings`. MCP `get_animation_settings` and `set_animation_settings`, and the debug menu, edit the same value. The table is [ADR 0038](../adr/0038-animation-adopts-design-and-fills-gaps.md) Addition C.

| Knob | Low | Medium | High |
|---|---|---|---|
| `animation.max_skinned` | 32 | 96 | 256 |
| `animation.update_rate_far_hz` | 15 | 60 | 60 |
| `animation.far_distance_m` | 25 | 25 | 25 |
| `animation.dqs` | off | off | off |
| `animation.ik` | off | on | on |

`.gskin` holds the skeleton, the clips, and the retarget map. `.ganim` holds states, transitions, blend spaces, masks, and notifies. Clips in `.gskin` come from glTF. Locomotion inputs (speed, heading) are filled on the machine that draws the character. A later network layer sends discrete events only.

## 8. Per-frame stats and budget

`anim.sample_ms` (stage wall time), `anim.skin_ms` (GPU skin dispatch), `anim.skinned_count`, `anim.bones`, `anim.vram_mb`, `anim.clip_mb`. Low is the GTX 1060 6 GB at 1080p, inside the 16.6 ms frame: 32 characters × 60 joints, sampling ≤ 1.0 ms, skinning ≤ 0.5 ms, GPU skin buffers ≤ 64 MB. The byte split, the 25 m far distance, and the probes are ADR 0038 Addition C. `anim.clip_mb` is CPU and is not part of the 64 MB. Vulkan 1.3 compute only. Skinned meshes keep the GI proxy. The 0.5 ms measurement is an owner-PC command. Lavapipe timings are not a result.

## 9. Verification harness and mutation tests
`crates/load/tests/gltf_skin.rs`, `crates/cook/tests/skin.rs`, `crates/scene/tests/animation_settings.rs`, `crates/render/tests/skinning.rs`, `scripts/anim-walk.rhai`. Gate A is the cook bytes. Gate C is the Low table and the 64 MB sum (ADR 0038). The render test waits until the compute skin pass exists. The replicated record is not a gate in this revision.
- **Reference oracle**: Khronos glTF-Sample-Assets (RiggedSimple, CesiumMan, Fox) — CPU reference skinning in f64 vs GPU buffer, max vertex error ≤ 1e-4 m.
- **Picture oracle**: rendered pose vs reference render, ≤ 2/255 mean.
- **Blend continuity**: crossfade has no joint jump > threshold between frames.
- **Determinism**: pose time is the sim tick. Two runs with the same inputs agree.
- **Root motion**: displacement matches clip root track integral.
Mutations (must fail): ignore `inverseBindMatrices`, drop the 4th weight, swap slerp for nlerp without sign fix, wall-clock time, count clip bytes inside the 64 MB, wrong joint parent order.

## 10. Agent control and verification
CLI `--anim-play <handle> <clip>`, `--anim-speed`, `--anim-freeze-tick N`. MCP `anim.set_param`, `anim.state` (returns state, clip times, weights for the selected scene handle), `anim.stats`, `view skin-weights|bones`. An agent changes a `.ganim` file, hot-reloads it, plays a scripted sequence and compares bone dumps plus screenshots. The 0.5 ms skin measurement is `genos-animation --measure-skin` on the owner's PC. Until the runtime exists, `genos-animation <file.gltf>` prints the import dump: joints, parents, clips, influence counts, and the skin notes (missing inverse bind, a reduced fifth weight, an omitted cubic-spline). That dump is the probe for the import.

## 11. Decisions for Anthony
1. Graph format: RON/JSON data + Rhai conditions (recommended) vs pure Rhai?
2. Server evaluates gameplay bones only (recommended) vs full skeletons?
3. Priority of IK vs state machine (physics foot IK depends on IK).
