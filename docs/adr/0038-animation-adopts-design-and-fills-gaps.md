# ADR 0038: Animation adopts the system design and fills its gaps

Status: Proposed (5 Nov 2026). No animation code until Anthony accepts it.

## Context

- [docs/systems/animation.md](../systems/animation.md) (PR #79, 27 Oct 2026) already sets the animation direction. It covers glTF skins and clips, CPU pose sampling on the fixed sim tick, compute pre-skinning, blend spaces, a data state machine, root motion, two-bone and FABRIK IK, network snapshots, budgets, a reference oracle and agent control. No ADR records that direction yet.
- The roadmap lists animation as the next priority, and physics foot IK depends on it.
- The design predates several decisions it needs to sit on: the cook pipeline (ADR 0020, with runtime load still pending in ADR 0035), the job system and scene ECS (ADRs 0031 and 0032, proposed in #131), and the 4 GB VRAM minimum spec (ADR 0023).
- The design leaves out retargeting, an ordering for its tests, and any place to evaluate learned motion synthesis.

## Decision

1. **Adopt animation.md** as the animation design. This ADR only adds what it lacks. Where the two disagree, this ADR wins until the design is updated.

2. **Retargeting.** Clips are authored against one skeleton and play on others through a retarget map. The map is data, made at cook time: it pairs bones by name (overridable) and records the rest-pose difference for each bone.
   - The runtime applies the map while sampling. Rotations transfer in each bone's local space relative to its rest pose. Translations transfer only on the root and the hips, scaled by the leg-length ratio, so feet don't slide.
   - Bones in the target that have no source bone keep their rest pose. Source bones that have no target bone are dropped, and the cook report lists both cases.
   - Retargeting runs on the server too, so gameplay bones agree (see animation.md §6).
   - Rejected: retargeting at runtime from bone names alone. It costs a lookup every frame, and the result changes if a rig is renamed.

3. **Cook pipeline tie-in (ADR 0020).**
   - **Import:** reads glTF skins, joints, inverse bind matrices, `JOINTS_0`/`WEIGHTS_0` and animation samplers. Each dropped feature is noted, as ADR 0020 already requires (for example more than 4 influences, morph targets until they are supported, and cubic-spline tangents if they are not kept).
   - **Cook:** writes skeletons, clips (raw f32 first, quantized later) and retarget maps into `.gcook`, with deterministic output and identity by cook hash.
   - **Runtime:** does no glTF parsing and no retarget-map building.
   - **Pending ADR 0035:** how cooked packages load at runtime is still proposed in #140. Animation follows whatever 0035 settles. Until then, tests may load cooked animation data directly, but the game must not take a glTF path for it.

4. **Job system and ECS tie-in (proposed ADRs 0031 and 0032, #131).**
   - **Job system:** pose evaluation (sample → blend → state machine → IK → local-to-model) is one job per skeleton batch on the 0031 job system. Results must not depend on how the work is split or how many threads run it, which keeps animation.md's determinism promise.
   - **ECS:** animation state, the skeleton reference and the pose buffer are 0032 components. The animation system writes into the scene's transform hierarchy for attached bones, such as a weapon on a hand, after physics reads its inputs and before the frame snapshot is taken.
   - **GPU:** skinning stays in the compute pass that animation.md chose, reading the poses that the CPU jobs produced.
   - **If 0031 or 0032 changes:** this section follows them. Animation does not get its own scheduler or entity store.

5. **IK hook for physics.** Foot IK takes ground contacts from physics queries (ray or shape casts along each leg) and runs after the state machine, before local-to-model. Physics owns the queries. Animation owns the solve and the pelvis offset. Only the server's gameplay bones are affected on the server.

6. **Budgets.**
   - Low stays tuned for the GTX 1060 6 GB, with animation.md §8's numbers: 32 skinned characters × 60 bones, sampling ≤ 1.0 ms CPU, skinning ≤ 0.5 ms GPU, skinned vertex buffers ≤ 64 MB.
   - The engine must also fit the 4 GB VRAM minimum (ADR 0023). The 64 MB skinned-buffer cap is about 1.6% of 4 GB. Added to Low's ~200 MB lighting soft budget, it stays well inside the shared `VK_EXT_memory_budget` budget. The cap must therefore hold at Low on a 4 GB device, and no preset may exceed it on a device that reports 4 GB.
   - Clip and skeleton memory count against the same budget and appear in `anim.vram_mb`.

7. **Reference-first, staged test order** (go-hard playbook rule 1: oracle first). Each stage merges its oracle before the code it grades:
   1. **CPU reference:** a deterministic f64 CPU reference for sampling, blending, skinning, retargeting and IK, checked against Khronos glTF-Sample-Assets (RiggedSimple, CesiumMan, Fox). Plain asserts, no GPU.
   2. **Comparison harness:** a harness that compares the real path with the reference (bone dumps and skinned vertices), plus the mutation tests in animation.md §9, which must fail when broken.
   3. **CPU runtime:** the real CPU runtime (cooked import, sampling, blend spaces, crossfade) graded against the reference, including bit-identical poses across job splits and thread counts.
   4. **GPU skinning:** compute skinning graded against the reference within animation.md's tolerance, plus one 4070 smoke run inside the same PR (playbook rule 5).
   5. **State machine and root motion.**
   6. **IK and the physics foot hook.**
   7. **Retargeting:** each step graded against the reference.

   Each stage is one small PR. The order of steps 5 and 6 is an open question (below).

8. **Research slot: learned motion synthesis (later evaluation only).** NVIDIA MotionBricks, and batched variants such as crowdbricks, are noted for evaluation after stages 1–5 land. They are not part of this decision. Before any adoption ADR, the evaluation must answer:
   - **Licence:** the code is Apache 2.0, but the weights are under the NVIDIA Open Model Agreement. Can they be redistributed in a shipped game?
   - **Cost on a GTX 1060 at Low:** what does it cost inside the animation budget above, and how much VRAM does it add on a 4 GB device?
   - **Runtime:** ONNX or in-engine inference, and whether the server can run it deterministically.
   - **Authored clips:** which authored clips are still needed (attacks, emotes).

   Any adoption would be a separate ADR, and it would be graded against the same CPU reference.

## Consequences

- Animation work can start at stage 1 now. Stages 3+ wait on ADRs 0031 and 0032 being accepted, and runtime loading of shipped data waits on ADR 0035.
- Retarget maps make the cook output larger and add a cook step, but keep the runtime simple and deterministic.
- The 4 GB rule caps how many characters a scene can skin at full quality. Far characters fall back to the reduced update rate in animation.md §7.

## Open questions for Anthony

1. **Graph format:** RON/JSON data plus Rhai conditions (recommended), or pure Rhai?
2. **Server evaluation:** gameplay bones only (recommended), or full skeletons?
3. **Order of IK and the state machine:** which comes first? Physics foot IK depends on IK, so putting IK (stage 6) before the state machine (stage 5) unblocks physics sooner.
4. **Retarget map defaults:** is name matching with per-rig overrides enough, or should it also support humanoid-avatar style bone roles?
