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
| Replication, in outline | The 5 Nov design: state id, clip ids, normalized time, 16-bit weights, no joint matrices. The 10 Nov draft turned that into 36 bytes at 20 Hz. Addition B below replaces both. | No snapshot type. |
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

### Addition B — derived locomotion, cues on the change tick

The 10 Nov record was 36 bytes per dirty character at 20 Hz. For 1000 visible characters that is `36 × 20 × 1000 = 720000` bytes/s on each client, about 720 KB/s. Phase, blend weights, and clip ids were in every snapshot. Those three are functions of data the session already sends. This section replaces that record. Joint matrices stay local, root motion writes the body, and foot IK stays off the checksum.

The session this follows is [PR #214](https://github.com/AnthonyPoschen/genos-engine/pull/214) (ADR 0042) on top of [PR #14](https://github.com/AnthonyPoschen/genos-engine/pull/14) and the approved scene store in [PR #131](https://github.com/AnthonyPoschen/genos-engine/pull/131). Those three are not merged here. Where they name a rate, an id, or a budget, animation uses that name.

- The wire id is the generational scene handle `{ index: u32, generation: u32 }`. There is no `NetId` column and no second tick.
- A component is dirty when its change tick is newer than the connection's acked baseline. A reader does not bump the tick.
- Snapshots are unreliable deltas at 20 Hz (every third `1/60` tick). The tick is `crates/scene/src/camera.rs:413`.
- Interest, eligibility, and the byte budget are ADR 0042's. The 32 m neighbourhood is eligible every snapshot. From there out to 128 m a character is eligible at most every fourth snapshot, which is 5 Hz. Past 128 m the character is omitted. The per-connection priority accumulator stops at 192 packed field bytes per snapshot (`192 × 20 = 3840` bytes/s, 3.75 KiB/s). `AnimCue` rides inside that budget. Animation does not name a second cap. A record header is still the 13 bytes ADR 0042 counts (index, generation, structural, count, component id, mask). Spawns sit outside the 192.
- The owning client predicts and rolls back. Everyone else interpolates the body from snapshots 100 ms back. Cross-platform `f32` bit identity is not a gate.

**Derived, and not a field.** Every peer runs this from the quantized pose the physics plan already replicates (millimetre position, smallest-three quaternion, linear velocity in cm/s as `i16`) and from the authority tick `T`.

- Horizontal speed is `vx * vx + vz * vz` on the stored `i16` cm/s samples, computed in `i32`. The speed bucket is the largest `k` in `0..15` with `(25 * k) * (25 * k) <= speed_sq`. That is the same 0.25 m/s step as dividing the square root by 25, and it does not call a square root. Facing is the XZ forward of the quaternion, scaled to integers. `cross = facing_x * vz - facing_z * vx` and `dot = facing_x * vx + facing_z * vz`. The strafe bucket is the sign of `cross` and whether `abs(cross) > abs(dot)`, four values. An equal magnitude takes the `dot` side. The `.ganim` locomotion space maps those two buckets to clip weights. The weights are not on the wire. No `atan2`.
- Locomotion phase at tick `T` is `(T + salt) mod cycle_ticks` on each blended clip. `cycle_ticks` is that clip's cooked length at 60 Hz, at least 1. `salt = (index * 17 + generation) mod cycle_ticks`. Two peers with the same handle, the same tick, and the same velocity sample produce the same phase and the same weights. They do not integrate phase, so a missed snapshot does not accumulate error. Speed changes the blend, not a stored phase.
- Foot IK is the visual plant in decision 5. It reads the local physics query. It is not a field and it is not in the checksum.
- Root motion writes the body. The body is the pose column. Animation does not send a second root.
- A notify whose time is a fraction of a locomotion clip fires when the derived phase crosses it. Both peers cross it on the same tick when the inputs match. The crossing is not a message.

**Sent: `AnimCue` only.** One column. Its change tick is the dirty signal. The component id is the next free `u16` in ADR 0042's registration table. `MeshRef` is 6 there. `AnimCue` is not 6. An outfit or a skeleton swap writes `MeshRef` (the source SHA-256, 32 field bytes). `AnimCue` does not repeat that hash. A toast that a late joiner does not need may stay the emote notice ADR 0042 already names. Anything still playing when a peer's baseline is 0 is this component, because a notice is not in the baseline.

The authority writes the column when the transition cannot be derived from velocity and facing: an emote, an attack, a hit reaction, or a montage. A write sets the change tick to `T`. The field is absolute, bit-packed, and omitted entirely once the baseline has that tick. That is the delta. There is no xor against the previous cue.

| Bits | Width | Meaning |
|---|---|---|
| kind | 3 | 0 correction, 1 emote, 2 attack, 3 hit, 4 montage. Other values fail the test. |
| state | 12 | State in `.ganim`. 4096 states. |
| start_low | 24 | Low 24 bits of the authority tick the cue began. |
| pad | 1 | Zero. A 1 fails the test. |
| phase | 16 | Present only for kind 0. Quantized phase at `start_low`, `0` at the start and `65535` just before the next loop. |

Kind 1–4 pack to 5 field bytes. A correction packs to 7. The receiver rebuilds the full tick as `(snapshot_tick & 0xFF00_0000) | start_low`, then subtracts `2^24` if that value lies more than `2^23` ticks ahead of the snapshot tick. Playback phase is `(T - start_tick) mod cycle_ticks` from the shared clock. The phase bits exist only so a correction can place the cycle when the clock function is the thing that diverged.

**Correction.** Every 15 ticks the authority compares gameplay-joint positions from the derived pose with the pose it simulated for the owning client. A joint more than 50 mm away writes kind 0 and bumps the tick. One write. The following snapshots omit it after the ack. Peers apply that phase at `start_tick` and then advance with the clock. Locomotion with no cue keeps a clean tick, including across a speed-bucket change.

**Interest.** The radii, the 5 Hz far slot, and the 192-byte accumulator are ADR 0042's. `AnimCue` adds none of them. `animation.far_distance_m` stays the local mesh sample in Addition C. It is not a network radius.

| Band | `AnimCue` |
|---|---|
| Inside the 32 m neighbourhood, or owned, or always-relevant | Eligible every snapshot. The field bytes join the same accumulator as the pose and `MeshRef`. |
| From 32 m out to 128 m | Eligible at most every fourth snapshot (5 Hz). A dirty cue waits for that slot. If those 192 bytes are already spent, it stays dirty and tries the next eligible snapshot. Playback uses `start_tick` on the shared clock, so the wait does not move the phase. |
| Past 128 m, not owned, not always-relevant | Omitted. |
| `competitive` and occluded | Omitted with the pose. |

A spawn still carries the current cue when one is active, the same way a spawn carries `MeshRef`, and a spawn is outside the 192 bytes.

**Bandwidth.** Field bytes are the 5 or the 7. Record bytes add the 13-byte header when the cue is the only dirty component on that object. Those bytes ride inside ADR 0042's 192 per snapshot. The scripted rates are 2% of characters beginning a cue each second (`max(1, N / 50)`) and 0.5% needing a correction (`max(1, N / 200)`). Each write is one send, once eligibility and the accumulator allow it. A character from 32 m to 128 m can be selected on at most 5 of the 20 snapshots in a second.

| Visible characters | Cues/s | Corrections/s | Field bytes/s | Record bytes/s | Rejected 36-byte design |
|---|---|---|---|---|---|
| 32 | 1 | 1 | 12 | 38 | `36 × 20 × 32 = 23040` |
| 200 | 4 | 1 | 27 | 92 | `36 × 20 × 200 = 144000` |
| 1000 | 20 | 5 | 135 | 460 | `36 × 20 × 1000 = 720000` |

A walking crowd with a clean `AnimCue` tick sends 0 animation bytes at every size. The 460 record bytes/s at 1000 characters sits under the 3.75 KiB/s ceiling. A burst of 1000 cues is `1000 × 5 = 5000` field bytes. One snapshot admits 38 of those 5-byte cues (190 bytes) and leaves 2 bytes of the 192. The other 962 stay dirty. Cues in the 32–128 m band are not candidates on the three snapshots between fourths.

**Authority.** The authority runtime is the only writer. A player enqueues a request. It does not write the column.

**Prediction and rollback.** The owning client runs the same graph from its inputs and from the same derived locomotion. On a snapshot for tick `T`, a mismatch of the quantized body or of `AnimCue` restores both for that handle and replays later inputs. Other characters are not resimulated. They draw the interpolated body and the cue phase from the clock. Question 2 stays open. Neither choice puts joint matrices, phase, or blend weights on the wire.

**Determinism.** Two worlds in one process, same handles, same `.ganim`, same quantized velocities, same tick, produce the same locomotion buckets and the same phase for 10 000 ticks, with zero `AnimCue` bytes while nobody starts a cue. Job splits are allowed only when the reduce is in joint-index order (decision 4). Wall time is not an input. A Windows authority and a Linux guest compare the cue bits and the quantized body, not raw `f32`.

**Headless server.** No window and no Vulkan device. It evaluates gameplay joints, applies root motion to the body, and writes `AnimCue`. It does not build a skinned vertex buffer. `LoadProfile` is not a type in the tree.

**Gate B.** In-process, no socket. `cargo test -p genos-load --test anim_cue` until `genos-scene` owns the column, then the test moves with it.

- Locomotion, 10 000 ticks, two worlds: phase and both buckets match, and `AnimCue` field bytes stay 0.
- The scripted rates above, for 32, 200, and 1000 characters, stay at or under 48, 128, and 512 field bytes/s per client. The 36-byte snapshot planted on every character fails the 1000-character cap.
- A packed cue is 5 field bytes, a correction is 7, the pad bit is 0, and the payload contains no `f32` and no joint matrix.
- Sixty-four near characters cue on one tick. The snapshot's `AnimCue` fields stay inside the 192-byte budget, the unsent cues ship on a later snapshot, and a spawn in the same datagram is still present.
- A character at 30 m is eligible on the next snapshot. A character at 40 m is eligible on every fourth snapshot and not on the three between. A character past 128 m is omitted.
- An owned-client mismatch at tick `T` restores `AnimCue` with the body and replays. A remote handle is not stepped.
- Foot IK does not change the checksum. A locomotion notify fires on the derived crossing and adds no bytes.

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
- A 36-byte animation snapshot at 20 Hz. `36 × 20 × 1000 = 720000` bytes/s per client. Locomotion phase and blend weights are derived. `AnimCue` carries only the transitions that derivation cannot see.
- A second character budget for the Steam Deck. Low is one table.
- A cross-platform `f32` bit-identity gate. The physics plan already refuses that for the same `genos-math`.

### Consequences of the addition

- Decision 7 is unchanged in order. Gate A can land before its f64 oracle as a byte check. Gate C lands with the settings struct. Gate B lands with the CPU evaluator.
- The asset plan's row that skins are `loaded, unsupported` changes in the commit that writes `GSK1`, not in this one.
- Question 1, question 2, question 3, and question 4 stay open. None of them puts phase or blend weights on the wire, and none of them changes the `.gskin` section list.
