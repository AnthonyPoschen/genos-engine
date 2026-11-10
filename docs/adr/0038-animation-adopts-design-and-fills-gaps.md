# ADR 0038: Animation adopts the system design and fills its gaps

Status: Accepted (10 Nov 2026) for the cook, the CPU runtime, GPU skinning, and the foot IK hook. Anthony approved that build without networking. The replication bytes are not in this ADR. Another change owns them. Where this file and [animation.md](../systems/animation.md) §6 disagree about bytes on the wire, §6 stays until that change lands, and this ADR does not implement it.

## Context

- [docs/systems/animation.md](../systems/animation.md) (27 Oct 2026) already sets the animation direction. It covers glTF skins and clips, CPU pose sampling on the fixed sim tick, compute pre-skinning, blend spaces, a data state machine, root motion, two-bone and FABRIK IK, budgets, a reference oracle and agent control. No ADR recorded that direction.
- The roadmap lists animation as the next priority, and physics foot IK depends on it.
- The design predates several decisions it needs to sit on: the cook pipeline (ADR 0020, ADR 0035), the job system and scene ECS (ADRs 0031 and 0032, proposed in #131), and the 4 GB VRAM minimum spec (ADR 0023).
- The design leaves out retargeting, an ordering for its tests, and any place to evaluate learned motion synthesis.

## Decision

1. **Adopt animation.md** as the animation design. This ADR only adds what it lacks. Where the two disagree, this ADR wins until the design is updated. The replication section is the exception named in the status line.

2. **Retargeting.** Clips are authored against one skeleton and play on others through a retarget map. The map is data, made at cook time: it pairs bones by name (overridable) and records the rest-pose difference for each bone.
   - The runtime applies the map while sampling. Rotations transfer in each bone's local space relative to its rest pose. Translations transfer only on the root and the hips, scaled by the leg-length ratio, so feet don't slide.
   - Bones in the target that have no source bone keep their rest pose. Source bones that have no target bone are dropped, and the cook report lists both cases.
   - Rejected: retargeting at runtime from bone names alone. It costs a lookup every frame, and the result changes if a rig is renamed.

3. **Cook pipeline tie-in (ADR 0020, ADR 0035).**
   - **Import:** reads glTF skins, joints, inverse bind matrices, `JOINTS_0`/`WEIGHTS_0` and animation samplers. Each dropped feature is noted, as ADR 0020 already requires (for example more than 4 influences, morph targets until they are supported, and cubic-spline tangents if they are not kept).
   - **Cook:** writes skeletons, clips (raw f32 first, quantized later) and retarget maps into the `.gskin` record in Addition A, with deterministic output and identity by cook hash. They do not go inside a `GCK1` record.
   - **Runtime:** does no glTF parsing and no retarget-map building when that record verifies.
   - **ADR 0035 is Accepted.** Skin bytes follow its verify-then-fallback rule. Addition A is that rule for skins. A verified skin does not take a glTF path.

4. **Job system and ECS tie-in (proposed ADRs 0031 and 0032, #131).**
   - **Job system:** pose evaluation (sample → blend → state machine → IK → local-to-model) is one job per skeleton batch on the 0031 job system. Results must not depend on how the work is split or how many threads run it.
   - **ECS:** animation state, the skeleton reference and the pose buffer are shaped as one component struct the scene can store today. When ADR 0032 lands, that struct moves into a 0032 component. The animation system writes into the scene's transform hierarchy for attached bones, such as a weapon on a hand, after physics reads its inputs.
   - **GPU:** skinning stays in the compute pass that animation.md chose, reading the poses that the CPU jobs produced.
   - **If 0031 or 0032 changes:** this section follows them. Animation does not get its own scheduler or its own scene store. Until they land, the component struct is the store.

5. **IK hook for physics.** Foot IK takes ground contacts from physics queries (ray or shape casts along each leg). Physics owns the queries. Animation owns the solve and the pelvis offset. That offset is the visual foot plant: it runs after the graph pose is chosen and before the skin dispatch, and it does not move the body.

6. **Budgets.**
   - Low stays tuned for the GTX 1060 6 GB, with animation.md §8's numbers: 32 skinned characters × 60 bones, sampling ≤ 1.0 ms CPU, skinning ≤ 0.5 ms GPU, skinned vertex buffers ≤ 64 MB.
   - The engine must also fit the 4 GB VRAM minimum (ADR 0023). The 64 MB skinned-buffer cap is about 1.6% of 4 GB. Added to Low's ~200 MB lighting soft budget, it stays well inside the shared `VK_EXT_memory_budget` budget. The cap must therefore hold at Low on a 4 GB device, and no preset may exceed it on a device that reports 4 GB.
   - The 64 MB cap is the GPU vertex buffer plus the joint palettes. Clip bytes are CPU and are reported as `anim.clip_mb`. The split is Addition C.

7. **Reference-first, staged test order** (go-hard playbook rule 1: oracle first). Each stage merges its oracle before the code it grades:
   1. **CPU reference:** a deterministic f64 CPU reference for sampling, blending, skinning, retargeting and IK, checked against Khronos glTF-Sample-Assets (RiggedSimple, CesiumMan, Fox). Plain asserts, no GPU.
   2. **Comparison harness:** a harness that compares the real path with the reference (bone dumps and skinned vertices), plus the mutation tests in animation.md §9, which must fail when broken.
   3. **CPU runtime:** the real CPU runtime (cooked import, sampling, blend spaces, crossfade) graded against the reference.
   4. **GPU skinning:** compute skinning graded against the reference within animation.md's tolerance. The 4070 timing is an owner-PC command, not a merge gate.
   5. **State machine and root motion.**
   6. **IK and the physics foot hook.**
   7. **Retargeting:** each step graded against the reference.

   Each stage is one small PR. The order of steps 5 and 6 is an open question (below). Gate A below can land as the byte check before the f64 oracle.

8. **Research slot: learned motion synthesis (later evaluation only).** NVIDIA MotionBricks, and batched variants such as crowdbricks, are noted for evaluation after stages 1–5 land. They are not part of this decision. Before any adoption ADR, the evaluation must answer the licence of the weights, the cost on a GTX 1060 at Low, whether the server can run the runtime deterministically, and which authored clips are still needed. Any adoption would be a separate ADR, graded against the same CPU reference.

## Consequences

- Animation work starts at the import and the `.gskin` record. ADR 0035 is Accepted. Skin bytes are Addition A.
- Retarget maps make the cook output larger and add a cook step, but keep the runtime simple.
- The 4 GB rule caps how many characters a scene can skin at full quality. Far characters fall back to the reduced update rate in animation.md §7.
- Locomotion blend inputs are derived on the machine that plays the clip, from velocity and heading. A later network layer sends discrete events (a jump, an attack), not the pose and not the blend weights.

## Open questions for Anthony

1. **Graph format:** RON/JSON data plus Rhai conditions (recommended), or pure Rhai?
2. **Server evaluation:** gameplay joints only (recommended), or full skeletons?
3. **Order of IK and the state machine:** which comes first? Physics foot IK depends on IK.
4. **Retarget map defaults:** is name matching with per-rig overrides enough, or should it also support humanoid-avatar style bone roles?

## Addition (10 Nov 2026)

Reviewed against master `959ec860`. Where this addition and the 5 Nov text differ, this addition wins. It does not open a second animation design. Replication bytes are omitted on purpose.

A joint is one entry in a glTF skin. [animation.md](../systems/animation.md) calls the same thing a bone. A gameplay joint is the root, plus the hit joints the skin record names. Codimation (`crates/scene/src/codimation.rs`) stays the scripted move of one solid. It is not a clip.

### Addition A — glTF skin and clips through the cook

ADR 0035 is Accepted. `--cooked DIR` already loads `GCK1` geometry and `.gtex` blocks. A material record does not read skins, clips, or joint attributes, and this path does not wait on it.

What the pipeline did before this ADR's implementation:

- `import_bytes` kept each `skins[]` and `animations[]` entry as `LoadedUnsupported` and stored the raw JSON. The test was `skin_and_morph_target_are_kept_as_unsupported`.
- `cook` copied those note strings into the `GCK1` record.
- `GCK1` is magic `GCK1`, version 1, then a SHA-256 trailer. The decoder rejects any other version. Joints, inverse binds, and weights are not fields of that record.
- The manifest line is `<asset> cook=<sha256> inputs=<sha256>` plus ` gtex=<sha256>` when a texture record exists. There was no skin token.

`GCK1` version 1 stays the rigid mesh. A skin is a sibling record, the same split `.gtex` already uses.

**Record.** `DIR/cooked/<asset>.gskin`, magic `GSK1`, version 1, little-endian, SHA-256 trailer over the bytes before it. The asset id and the settings hash match the `GCK1` record for that source. Sections, in order:

1. **Joints.** Count, then each joint: parent index (`i32`, −1 for a root), name, inverse bind as sixteen `f32` in glTF column order, then the rest local translation (`f32×3`), rotation (`f32×4`, x y z w), and scale (`f32×3`), then the ancestor matrix (`f32×16`, column-major). The ancestor is the product of the non-joint nodes between the parent joint and this joint. It is identity when the parent node is the parent joint. It is not baked into the rest TRS, because a channel replaces that TRS. Order is parent before child. Siblings keep glTF joint order. A missing inverse-bind accessor is identity, and the cook note says so. An accessor longer than `joints` keeps the prefix and notes the extra matrices. After the joints, one `i32` per `GCK1` instance: the skin index, or −1 when that instance has no skin.
2. **Influences.** One block per primitive, in `GCK1` mesh and primitive order. `JOINTS_0` stored as `u16×4`. `WEIGHTS_0` stored as the source `f32` bits. A vertex with more than four influences keeps the four largest weights, divides those four by their sum, and writes a note. A zero sum writes weight 1 on joint 0 and a note. Morph targets stay `LoadedUnsupported` and are not a section.
3. **Clips.** Name, duration in seconds (`f32`), then channels whose target is a joint in this skin. A channel is a joint index, a path (`translation`, `rotation`, or `scale`), an interpolation (`linear` or `step`), and the key times and values as the source `f32` bits. `step` holds the left key until the next time. `linear` matches `Animation::sample` for translation, scale, and short-arc slerp. Cubic-spline samplers, morph-weight channels, and channels aimed at a node that is not in the skin are omitted and listed in the notes.
4. **Retarget map.** The pairs from decision 2. An empty map is a zero count. The runtime does not build the map.
5. **Notes.** The dropped-feature strings.

**Manifest.** When the file exists, the line gains ` skin=<sha256>` after `gtex=` if that token exists, otherwise after `inputs=`. A line without `skin=` is unchanged. `verify` hashes the `.gskin`. A missing file, a bad magic, or a trailer mismatch is `assets fallback <asset> <reason>` and that asset loads as the rigid mesh. A fallback does not fail the frame. The game path does not call the glTF decoder for a line that verified.

**ECS.** The record is an asset. It does not wait for the scene ECS. When that design lands, the skinned object is a scene handle and the joints are children (`docs/systems/ecs-scene.md` §7). This addition does not add a store of its own (decision 4). The in-memory animation state is one component-shaped struct so it can move into that store without a second layout.

**GI.** Drawn vertices may be skinned later by the compute pass animation.md already chose. Exact GI of a skinned mesh stays out of scope. Those meshes use proxies (`docs/systems/gi-design.md`).

**Gate A**, before any GPU skinning and before ADRs 0031 and 0032. `cargo test -p genos-load --test gltf_skin` and `cargo test -p genos-cook --test skin`. One fixture with a joint chain, four weights, one linear clip, and one step clip:

- The cook writes `.gskin` and `skin=` on the manifest line.
- `verify` accepts it. One flipped trailer byte fails `verify`.
- Decode with the source file absent returns the parents, the inverse binds, the linear midpoint bit for bit at half the key span, and the step value equal to the left key just before the next key.
- A fifth influence is reduced as above and the mesh still cooks. A morph target stays unsupported and is absent from the record.
- The test does not create a Vulkan device.

Decision 7 step 1 grades this decode against the f64 reference once that reference exists. Gate A is the byte-level gate and can land first.

### Addition C — Low preset budget

Decision 6 and animation.md §8 set the ceilings. They do not bind them to `LightingPreset::Low`, and they do not show the byte sum. Low is the GTX 1060 6 GB / RX 580 at 1080p, and the Steam Deck at 800p. The frame ceiling is 16.6 ms. The minimum is Vulkan 1.3 and 4 GB (`docs/adr/0023-graphics-settings-presets.md`, `docs/engine/quality-settings.md`). Lighting's soft budget at Low is about 200 MB. The animation sample is ≤ 1.0 ms wall inside a 6.0 ms Must scope, on the 4-core CPU paired with that card.

`AnimationSettings` implements `EngineSettings`. MCP `get_animation_settings` and `set_animation_settings`, and the debug menu, edit that value. A preset only fills the table. There is no per-GPU branch.

| Knob | Low | Medium | High |
|---|---|---|---|
| `animation.max_skinned` | 32 | 96 | 256 |
| `animation.update_rate_far_hz` | 15 | 60 | 60 |
| `animation.far_distance_m` | 25 | 25 | 25 |
| `animation.dqs` | off | off | off |
| `animation.ik` | off | on | on |

The vertical field of view is 60°. At 25 m a 1.8 m figure is about 67 px tall at 1080p, so inside that distance Low still samples the mesh every tick. Past it, the drawn mesh samples at 15 Hz. Gameplay joints stay on the `1/60` tick at every distance. Medium and High set the far rate to the sim rate, so the distance knob does not drop samples until someone lowers the rate. `dqs` stays off, so the 0.5 ms ceiling is linear blend skinning. `ik` is off at Low, which keeps the solver out of the Low frame.

**64 MB.** The cap is `64 × 1024 × 1024` bytes of GPU memory: the skinned vertex buffer plus the joint palettes. Clip bytes are CPU, reported as `anim.clip_mb`, and are not in the 64 MB. Gate A prints `anim.clip_mb` for the fixture. That print is not a pass threshold.

- An output vertex is position `f32×3`, normal `f32×3`, and tangent `f32×4`: 40 bytes.
- A joint palette entry is a `mat3x4`: 48 bytes. At 32 × 60 joints that is 92,160 bytes.
- The vertex bytes that remain are 67,016,704. Split evenly, that is 52,356 vertices per character. 32 × 52,356 × 40 + 92,160 = 67,107,840, which is 1,024 bytes under the cap. One more vertex on every character crosses it.
- The runtime skins nearest-first and stops when `max_skinned` or the byte cap binds. A mesh over 52,356 vertices is legal. It spends the shared cap, and the characters that no longer fit stay on the rest pose.
- `anim.skin_ms` ≤ 0.5 ms is the skin dispatch only. It sits beside the lighting cap, inside the 16.6 ms frame. It does not include a GI refit.
- The same 32 and the same 64 MB hold on the Steam Deck's Low preset. A miss moves `max_skinned` down, which is the ADR 0023 rule for a knob that misses its tier.

**Probes.** `anim.sample_ms` (stage wall time, not the sum of worker times), `anim.skin_ms`, `anim.skinned_count`, `anim.bones`, `anim.vram_mb`, `anim.clip_mb`. `anim.vram_mb` over 64, or `anim.skinned_count` over `max_skinned`, fails the cheap check. `assets fallback` is the probe for a bad `.gskin`.

**Gate C**, a CPU test with no device: `cargo test -p genos-scene --test animation_settings`. `AnimationSettings::preset(Low)` round-trips through `to_text` and `parse` as the table above. The byte function accepts 32 characters, 60 joints, and 52,356 vertices on each character, and rejects 52,357 vertices on each. The 0.5 ms measurement is an owner-PC command for the animation debug binary after GPU skinning exists (`genos-animation --measure-skin`). It is not a merge gate. Lavapipe timings are not a result.

### Rejected for this addition

- Putting joints inside `GCK1` version 1. The trailer covers the whole file, and another version is rejected, so every current record would fail decode.
- Sending joint matrices, root-motion deltas, IK targets, or notify lists. Locomotion is derived locally. A later layer sends discrete events.
- A second character budget for the Steam Deck. Low is one table.
- A cross-platform `f32` bit-identity gate.

### Consequences of the addition

- Decision 7 is unchanged in order. Gate A can land before its f64 oracle as a byte check. Gate C lands with the settings struct.
- The asset plan's row that skins are `loaded, unsupported` changes in the commit that writes `GSK1`.
- Questions 1 through 4 stay open. None of them changes the `.gskin` section list.
