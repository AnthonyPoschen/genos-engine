# Animation system (design)

Status: proposed, 27 Oct 2026. Owner: Engine. Binds to [ADR 0012](../adr/0012-gi.md) (GI sees moving meshes), [physics plan](physics/plan.md) (foot IK hooks), [asset plan](assets/plan.md), [agent rubric](../agent-rubric.md).

## 1. Goal
Play glTF skinned characters with blended clips, a data-driven state machine, root motion and later IK, on the GPU, deterministic enough that a server and clients agree on poses that matter to gameplay.

## 2. Where it sits today
| Piece | State | Code |
|---|---|---|
| Text TRS clip, slerp sampling | done (one node) | `crates/load/src/animation.rs` |
| Scripted motion | done | `crates/scene/src/codimation.rs`, `.rhai` |
| glTF skins, joints, clips | missing (glTF loader reads meshes/materials only) | `crates/load/src/gltf.rs` |
| Skinning, blending, state machine | missing | – |

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
| Skinning | vertex-shader LBS each pass | **compute pre-skin into a buffer once/frame** | B: reused by G-buffer, shadows/trace BLAS refit, motion vectors |
| Blending | 1D/2D blend spaces + layered masks | full node graph | A (graph is data, small node set) |
| State machine | hard-coded | **data file (`.ganim` RON/JSON) + Rhai conditions** | data |
| Clip storage | raw f32 keys | ACL-style quantized (later) | raw first, quantize when memory shows |
| Dual quaternion | yes | LBS only | LBS default, DQS per-skin flag later |

## 5. Recommendation
Stage 1: glTF skins+clips import, CPU sampling (SoA, fixed 60 Hz sim tick), 1D/2D blend spaces, crossfade, compute LBS pre-skin, BLAS refit for GI. Stage 2: data state machine with Rhai conditions, root motion, anim events/notifies. Stage 3: two-bone IK + FABRIK for foot placement (physics hook), look-at. Later: morph targets, motion matching, compression.

**Deviation flags.** None from the papers. Sampling on the CPU (not GPU) is deliberate so the server, which has no device, runs the same code.

## 6. Determinism for networking
- Sim tick fixed; pose time = tick × dt, never wall time. Fixed-point-free but order-stable f32 math, no SIMD reassociation in the gameplay pose path.
- Replicate state-machine state id, clip ids, normalized time and blend weights (quantized 16-bit), not bone transforms.
- Server evaluates only gameplay bones (root, hit-boxes) on `LoadProfile::Sim`; clients evaluate the full skeleton. Root motion is server-authoritative, predicted on the owning client, reconciled with rollback of state-machine time.
- Rubric lines: replicated = anim state; authority = server; prediction = owner client; determinism = tick-based; headless = gameplay bones only.

## 7. Data and settings
`settings.animation`: `max_skinned` (Low 32 / Med 96 / High 256), `update_rate_far_hz` (Low 15, distance-based throttling), `dqs` (off), `ik` (on High/Med). Assets: `.ganim` graph (states, transitions, blend spaces, masks), clips from glTF.

## 8. Per-frame stats and budget
`anim.sample_ms` (CPU), `anim.skin_ms` (GPU), `anim.blas_refit_ms`, `anim.skinned_count`, `anim.bones`, `anim.vram_mb`. Low preset, GTX 1060 6GB, 1080p: 32 characters × 60 bones, sampling ≤ 1.0 ms CPU, skinning ≤ 0.5 ms GPU, ≤ 64 MB skinned vertex buffers. Vulkan 1.3 compute only.

## 9. Verification harness and mutation tests
`crates/load/tests/gltf_skin.rs`, `crates/render/tests/skinning.rs`, `scripts/anim-walk.rhai`.
- **Reference oracle**: Khronos glTF-Sample-Assets (RiggedSimple, CesiumMan, Fox) — CPU reference skinning in f64 vs GPU buffer, max vertex error ≤ 1e-4 m.
- **Picture oracle**: rendered pose vs reference render, ≤ 2/255 mean.
- **Blend continuity**: crossfade has no joint jump > threshold between frames.
- **Determinism**: two runs, and server-vs-client gameplay bones, bit-identical over 10 000 ticks.
- **Root motion**: displacement matches clip root track integral.
Mutations (must fail): ignore `inverseBindMatrices`, drop the 4th weight, swap slerp for nlerp without sign fix, wall-clock time, skip BLAS refit (GI test shows stale shadow), wrong joint parent order.

## 10. Agent control and verification
CLI `--anim-play <entity> <clip>`, `--anim-speed`, `--anim-freeze-tick N`. MCP `anim.set_param`, `anim.state` (returns state, clip times, weights for the selected entity), `anim.stats`, `view skin-weights|bones`. An agent changes a `.ganim` file, hot-reloads it, plays a scripted sequence and compares bone dumps plus screenshots.

## 11. Decisions for Anthony
1. Graph format: RON/JSON data + Rhai conditions (recommended) vs pure Rhai?
2. Server evaluates gameplay bones only (recommended) vs full skeletons?
3. Priority of IK vs state machine (physics foot IK depends on IK).
