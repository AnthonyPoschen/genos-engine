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

3. **Cook pipeline tie-in (ADR 0020, ADR 0035).**
   - **Import:** reads glTF skins, joints, inverse bind matrices, `JOINTS_0`/`WEIGHTS_0` and animation samplers. Each dropped feature is noted, as ADR 0020 already requires (for example more than 4 influences, morph targets until they are supported, and cubic-spline tangents if they are not kept).
   - **Cook:** writes skeletons, clips (raw f32 first, quantized later) and retarget maps into the `.gskin` record in Addition A, with deterministic output and identity by cook hash. They do not go inside a `GCK1` record.
   - **Runtime:** does no glTF parsing and no retarget-map building when that record verifies.
   - **ADR 0035 is Accepted.** Skin bytes follow its verify-then-fallback rule. Addition A is that rule for skins. A verified skin does not take a glTF path.

4. **Job system and ECS tie-in (proposed ADRs 0031 and 0032, #131).**
   - **Job system:** pose evaluation (sample → blend → state machine → IK → local-to-model) is one job per skeleton batch on the 0031 job system. Results must not depend on how the work is split or how many threads run it, which keeps animation.md's determinism promise.
   - **ECS:** animation state, the skeleton reference and the pose buffer are 0032 components. The animation system writes into the scene's transform hierarchy for attached bones, such as a weapon on a hand, after physics reads its inputs and before the frame snapshot is taken.
   - **GPU:** skinning stays in the compute pass that animation.md chose, reading the poses that the CPU jobs produced.
   - **If 0031 or 0032 changes:** this section follows them. Animation does not get its own scheduler or its own scene store.

5. **IK hook for physics.** Foot IK takes ground contacts from physics queries (ray or shape casts along each leg). Physics owns the queries. Animation owns the solve and the pelvis offset. That offset is the visual foot plant: it runs after the graph pose is chosen and before the skin dispatch, it does not move the body, and it is not in the snapshot (Addition B). The server still evaluates gameplay joints. It does not evaluate the visual plant into those joints.

6. **Budgets.**
   - Low stays tuned for the GTX 1060 6 GB, with animation.md §8's numbers: 32 skinned characters × 60 bones, sampling ≤ 1.0 ms CPU, skinning ≤ 0.5 ms GPU, skinned vertex buffers ≤ 64 MB.
   - The engine must also fit the 4 GB VRAM minimum (ADR 0023). The 64 MB skinned-buffer cap is about 1.6% of 4 GB. Added to Low's ~200 MB lighting soft budget, it stays well inside the shared `VK_EXT_memory_budget` budget. The cap must therefore hold at Low on a 4 GB device, and no preset may exceed it on a device that reports 4 GB.
   - The 64 MB cap is the GPU vertex buffer plus the joint palettes. Clip bytes are CPU and are reported as `anim.clip_mb`. The split is Addition C.

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

- Animation work can start at stage 1 now. Stages 3+ wait on ADRs 0031 and 0032 being accepted. ADR 0035 is Accepted. Skin bytes are Addition A and are not in the package yet.
- Retarget maps make the cook output larger and add a cook step, but keep the runtime simple and deterministic.
- The 4 GB rule caps how many characters a scene can skin at full quality. Far characters fall back to the reduced update rate in animation.md §7.

## Open questions for Anthony

1. **Graph format:** RON/JSON data plus Rhai conditions (recommended), or pure Rhai?
2. **Server evaluation:** gameplay bones only (recommended), or full skeletons?
3. **Order of IK and the state machine:** which comes first? Physics foot IK depends on IK, so putting IK (stage 6) before the state machine (stage 5) unblocks physics sooner.
4. **Retarget map defaults:** is name matching with per-rig overrides enough, or should it also support humanoid-avatar style bone roles?

## Addition (10 Nov 2026)

Reviewed against master `959ec860` and the open cook PRs. Where this addition and the 5 Nov text differ, this addition wins. It does not open a second animation design. No renderer code.

A joint is one entry in a glTF skin. [animation.md](../systems/animation.md) calls the same thing a bone. A gameplay joint is the root, plus the hit joints the skin record names. Codimation (`crates/scene/src/codimation.rs`) stays the scripted move of one solid. It is not a clip.

### What the 5 Nov text already covers

| Piece | Where it already lives | Code on master |
|---|---|---|
| Skinning method | animation.md §4, compute linear blend, once per frame. Decision 1 adopts it. | No skinning in `crates/render`. |
| Clip sampling for one text transform | animation.md §2 | `Animation::sample` at `crates/load/src/animation.rs:45`, short-arc slerp at `crates/math/src/quat.rs:82`. `load_animation` at `crates/load/src/lib.rs:162`. |
| Blend spaces, state machine, root motion, IK, events | animation.md §4–§5 and decisions 2 and 5. The graph format, the server joint set, and the IK-versus-state-machine order stay open questions. | None of these run. |
| GPU skinning | animation.md §4. Vulkan 1.3 compute, after the CPU pose. | `Primitive` (`crates/load/src/gltf.rs:74`) has positions, normals, texcoords, tangents, indices, and a material. No joint attribute is uploaded. |
| Cook, in outline | Decision 3. | The outline said `.gcook` and a pending ADR 0035. Both are corrected below. |
| Replication, in outline | animation.md §6: state id, clip ids, normalized time, 16-bit weights, no joint matrices. | No snapshot type. |
| Low ceilings, in outline | Decision 6 and animation.md §8: 32 characters, 60 joints, ≤ 1.0 ms sample, ≤ 0.5 ms skin, ≤ 64 MB. | No `AnimationSettings`. |

### Addition A — glTF skin and clips through the cook

ADR 0035 is Accepted. `--cooked DIR` already loads `GCK1` geometry and `.gtex` blocks. PR #178 cooks a `GMT1` material record and a `mat=` manifest token. It does not read skins, clips, or joint attributes, and this path does not wait on it.

What the pipeline does today:

- `import_bytes` keeps each `skins[]` and `animations[]` entry as `LoadedUnsupported` and stores the raw JSON (`crates/load/src/import.rs:117`). The test is `skin_and_morph_target_are_kept_as_unsupported` (`crates/load/tests/import.rs:43`).
- `cook` copies those note strings into the `GCK1` record (`crates/cook/src/lib.rs:159`, `crates/cook/src/format.rs:69`).
- `GCK1` is magic `GCK1`, version 1, then a SHA-256 trailer (`crates/cook/src/format.rs:12`). The decoder rejects any other version. Joints, inverse binds, and weights are not fields.
- The manifest line is `<asset> cook=<sha256> inputs=<sha256>` plus ` gtex=<sha256>` when a texture record exists (`crates/cook/src/package.rs:12`). There is no skin token.

`GCK1` version 1 stays the rigid mesh. A skin is a sibling record, the same split `.gtex` already uses.

**Record.** `DIR/cooked/<asset>.gskin`, magic `GSK1`, version 1, little-endian, SHA-256 trailer over the bytes before it. The asset id and the settings hash match the `GCK1` record for that source. Sections, in order:

1. **Joints.** Count, then each joint: parent index (`i32`, −1 for a root), name, inverse bind as sixteen `f32` in glTF column order. Order is parent before child. Siblings keep glTF joint order. A missing inverse-bind accessor is identity, and the cook note says so.
2. **Influences.** One block per primitive, in `GCK1` mesh and primitive order. `JOINTS_0` stored as `u16×4`. `WEIGHTS_0` stored as the source `f32` bits. A vertex with more than four influences keeps the four largest weights, divides those four by their sum, and writes a note. A zero sum writes weight 1 on joint 0 and a note. Morph targets stay `LoadedUnsupported` and are not a section.
3. **Clips.** Name, duration in seconds (`f32`), then channels whose target is a joint in this skin. A channel is a joint index, a path (`translation`, `rotation`, or `scale`), an interpolation (`linear` or `step`), and the key times and values as the source `f32` bits. `step` holds the left key until the next time. `linear` matches `Animation::sample`. Cubic-spline samplers, morph-weight channels, and channels aimed at a node that is not in the skin are omitted and listed in the notes. The text clip format stays for the one-node file `load_animation` already reads. Codimation is not converted.
4. **Retarget map.** The pairs from decision 2. An empty map is a zero count. The runtime does not build the map.
5. **Notes.** The dropped-feature strings.

**Manifest.** When the file exists, the line gains ` skin=<sha256>` after `gtex=` if that token exists, otherwise after `inputs=`. A line without `skin=` is unchanged. `verify` hashes the `.gskin`. A missing file, a bad magic, or a trailer mismatch is `assets fallback <asset> <reason>` and that asset loads as the rigid mesh. A fallback does not fail the frame. The game path does not call the glTF decoder for a line that verified.

**ECS.** The record is an asset. It does not wait for the scene ECS. When that design lands, the skinned object is a scene handle and the joints are children (`docs/systems/ecs-scene.md` §7). This addition does not add a store of its own (decision 4).

**GI.** Drawn vertices may be skinned later by the compute pass animation.md already chose. Exact GI of a skinned mesh stays out of scope. Those meshes use proxies (`docs/systems/gi-design.md`).

**Gate A**, before any GPU skinning and before ADRs 0031 and 0032. `cargo test -p genos-load --test gltf_skin` and `cargo test -p genos-cook --test skin`. One fixture with a joint chain, four weights, one linear clip, and one step clip:

- The cook writes `.gskin` and `skin=` on the manifest line.
- `verify` accepts it. One flipped trailer byte fails `verify`.
- Decode with the source file absent returns the parents, the inverse binds, the linear midpoint bit for bit at half the key span, and the step value equal to the left key just before the next key.
- A fifth influence is reduced as above and the mesh still cooks. A morph target stays unsupported and is absent from the record.
- The test does not create a Vulkan device.

Decision 7 step 1 grades this decode against the f64 reference once that reference exists. Gate A is the byte-level gate and can land first.

### Addition B — replicated animation state

animation.md §6 already refuses to send joint matrices, and it names the server, the owning client, and the tick. It does not record the bytes, the snapshot rate, or how that sits next to the physics plan. Gameplay networking still waits. There is no `docs/systems/core-networking/plan.md` in the tree. The physics plan is the co-design this record matches: the server is the authority, the tick is `1/60` s (`crates/scene/src/camera.rs:413`), snapshots go out at 20 Hz, the owning client predicts, everyone else takes the snapshot, same-process determinism is required, and cross-platform bit identity of `f32` is not a gate (`docs/systems/physics/plan.md`, Networking).

The session codec stays in that future plan. Animation defines the record, the way physics defines a replicated body and does not open a socket.

**Record.** One dirty character, little-endian, 36 bytes. Order on the wire is `NetId` order.

| Field | Width | Meaning |
|---|---|---|
| `net_id` | `u32` | Server-assigned. For a shared scene file it is that file's scene handle (`docs/systems/ecs-scene.md`). |
| `skin_hash` | `u64` | First eight bytes of the `.gskin` SHA-256, little-endian. A mismatch drops the snapshot for that id. |
| `state_id` | `u16` | State-machine state. `0` until the graph exists. |
| `flags` | `u16` | Zero. Pads the header to 16 bytes. A non-zero value fails Gate B. |
| clip 0, clip 1 | `u16` id, `u16` phase, `u16` weight, each | Phase `0` is the start and `65535` is just before the next loop. A live pair of weights sums to `65535`. An unused slot has weight `0`. |
| `params` | `4 × u16` | Blend-space inputs, mapped from the range stored in `.ganim`. |

An unchanged record sends nothing. The first time a peer sees the id, it sends the full record.

**What stays local.** Joint matrices, the sampled pose, the skinned vertex buffer, the retarget scratch, foot IK, and notify lists. Root motion writes the body. The body then replicates as the physics plan already quantizes it (millimetre position, smallest-three quaternion). The animation record does not carry a second position.

**Foot IK and events.** The physics plan already leaves foot plant off the checksum and runs it on the pose that will be drawn. Decision 5 does not put IK on the wire. A notify is a named tick on the `.ganim` graph. The server runs a gameplay notify when the phase crosses it. The client may run a cosmetic notify from the same phase. The record has no event list.

**Authority.** The server. Clients do not accept one another's animation records.

**Prediction and rollback.** The owning client steps the same graph on the `1/60` tick and keeps the record beside the predicted body. On a snapshot for tick T, a mismatch of the quantized body or of this record restores both for that `NetId` and replays later inputs. Other characters are not resimulated. Remote phase snaps to the latest snapshot. A clip of at least 50 ms may interpolate phase for the drawn mesh only, taking the short way around the `u16` wrap. A shorter clip snaps. Gameplay joints on a remote character are not a client hit test. Question 2 stays open. The record is the same if the server evaluates every joint, because the matrices are still local.

**Determinism.** Pose time is tick × `1/60`, never wall time. Two worlds in one process, given the same `.gskin` bytes and the same inputs, produce the same gameplay-joint positions for 10 000 ticks. Job splits are allowed only when the reduce is in joint-index order (decision 4). A Windows server and a Linux client compare this record and the quantized body, not raw `f32` bits.

**Headless server.** No window and no Vulkan device. It evaluates gameplay joints and applies root motion to the body. It does not build a skinned vertex buffer. `LoadProfile` is not a type in the tree.

**Gate B.** In-process, no socket, beside decision 7 step 3. `cargo test -p genos-load --test anim_snapshot`. The test moves with the component when the scene crate owns it.

- 10 000 ticks, two worlds, bit-identical gameplay-joint positions.
- The encoded record is 36 bytes and contains no `f32` and no matrix.
- An owned-client mismatch at tick T restores the record and replays. A remote id is not stepped.
- Running a foot-IK pose does not change the checksum.
- A notify on the graph fires once on the crossing tick and adds no bytes to the record.

`state_id` stays `0` until decision 7 step 5. The IK checksum assertion lands with step 6.

### Addition C — Low preset budget

Decision 6 and animation.md §8 set the ceilings. They do not bind them to `LightingPreset::Low`, and they do not show the byte sum. Low is the GTX 1060 6 GB / RX 580 at 1080p, and the Steam Deck at 800p. The frame ceiling is 16.6 ms. The minimum is Vulkan 1.3 and 4 GB (`docs/adr/0023-graphics-settings-presets.md`, `docs/engine/quality-settings.md`). Lighting's soft budget at Low is about 200 MB. The job system puts the lighting GPU cap at 4 ms and the animation sample at ≤ 1.0 ms wall inside a 6.0 ms Must scope, on the 4-core CPU paired with that card (`docs/systems/job-system.md`).

`AnimationSettings` implements `EngineSettings`. MCP `get_animation_settings` and `set_animation_settings`, and the debug menu, edit that value. A preset only fills the table. There is no per-GPU branch.

| Knob | Low | Medium | High |
|---|---|---|---|
| `animation.max_skinned` | 32 | 96 | 256 |
| `animation.update_rate_far_hz` | 15 | 60 | 60 |
| `animation.far_distance_m` | 25 | 25 | 25 |
| `animation.dqs` | off | off | off |
| `animation.ik` | off | on | on |

The vertical field of view is 60° (`crates/scene/src/camera.rs:430`). At 25 m a 1.8 m figure is about 67 px tall at 1080p, so inside that distance Low still samples the mesh every tick. Past it, the drawn mesh samples at 15 Hz. Gameplay joints stay on the `1/60` tick at every distance. Medium and High set the far rate to the sim rate, so the distance knob does not drop samples until someone lowers the rate. `dqs` stays off, so the 0.5 ms ceiling is linear blend skinning. `ik` is off at Low, which matches the physics plan's visual foot plant and keeps the solver out of the Low frame.

**64 MB.** The cap is `64 × 1024 × 1024` bytes of GPU memory: the skinned vertex buffer plus the joint palettes. Decision 6's line that clip memory shares this cap is narrowed. Clip bytes are CPU, reported as `anim.clip_mb`, and are not in the 64 MB. Gate A prints `anim.clip_mb` for the fixture. That print is not a pass threshold.

- An output vertex is position `f32×3`, normal `f32×3`, and tangent `f32×4`: 40 bytes.
- A joint palette entry is a `mat3x4`: 48 bytes. At 32 × 60 joints that is 92,160 bytes.
- The vertex bytes that remain are 67,016,704. Split evenly, that is 52,356 vertices per character. 32 × 52,356 × 40 + 92,160 = 67,107,840, which is 1,024 bytes under the cap. One more vertex on every character crosses it.
- The runtime skins nearest-first and stops when `max_skinned` or the byte cap binds. A mesh over 52,356 vertices is legal. It spends the shared cap, and the characters that no longer fit stay on the rest pose.
- `anim.skin_ms` ≤ 0.5 ms is the skin dispatch only. It sits beside the 4 ms lighting cap, inside the 16.6 ms frame. It does not include a GI refit.
- The same 32 and the same 64 MB hold on the Steam Deck's Low preset. A miss moves `max_skinned` down, which is the ADR 0023 rule for a knob that misses its tier.

**Probes.** `anim.sample_ms` (stage wall time, not the sum of worker times), `anim.skin_ms`, `anim.skinned_count`, `anim.bones`, `anim.vram_mb`, `anim.clip_mb`. `anim.vram_mb` over 64, or `anim.skinned_count` over `max_skinned`, fails the cheap check. `assets fallback` is the probe for a bad `.gskin`. The replicated checksum is the probe for a snapshot that started carrying joint matrices or an IK output.

**Gate C**, a CPU test with no device: `cargo test -p genos-scene --test animation_settings`. `AnimationSettings::preset(Low)` round-trips through `to_text` and `parse` as the table above. The byte function accepts 32 characters, 60 joints, and 52,356 vertices on each character, and rejects 52,357 vertices on each. The 0.5 ms measurement is an owner-PC command for the animation debug binary after decision 7 step 4. It is not a merge gate. Lavapipe timings are not a result.

### Rejected for this addition

- Putting joints inside `GCK1` version 1. The trailer covers the whole file, and another version is rejected, so every current record would fail decode.
- Waiting on PR #178. That draft is the material record.
- Sending joint matrices, root-motion deltas, IK targets, or notify lists. The body snapshot and the phase already carry the gameplay result.
- A second character budget for the Steam Deck. Low is one table.
- A cross-platform `f32` bit-identity gate. The physics plan already refuses that for the same `genos-math`.

### Consequences of the addition

- Decision 7 is unchanged in order. Gate A can land before its f64 oracle as a byte check. Gate C lands with the settings struct. Gate B lands with the CPU evaluator.
- The asset plan's row that skins are `loaded, unsupported` changes in the commit that writes `GSK1`, not in this one.
- Question 1, question 2, question 3, and question 4 stay open. None of them changes the 36-byte record or the `.gskin` section list.
