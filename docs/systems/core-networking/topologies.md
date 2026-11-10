# Multiplayer topologies

Status: proposed, 10 Nov 2026. Decision: [ADR 0042](../../adr/0042-multiplayer-topologies.md). No code in this change.

This file extends the server-authoritative plan in [PR #14](https://github.com/AnthonyPoschen/genos-engine/pull/14). That plan's file is `docs/systems/core-networking/plan.md` on `cursor/core-networking-plan-c247`. It is not on master. Where this file and that plan disagree, [ADR 0042](../../adr/0042-multiplayer-topologies.md) wins. Everywhere else, that plan's recommendation stands, including its open questions D1–D54.

The scene store this file reads is the approved implementation of [PR #131](https://github.com/AnthonyPoschen/genos-engine/pull/131): a generational scene handle and a change tick on each component ([`ecs-scene.md`](../ecs-scene.md)). The asset id is the one [`crates/load`](../../../crates/load/src/sha256.rs) and the cook record already write.

## 1. Goal

A player, an NPC, and a prop are scene objects. Joined runtimes show the same objects. One runtime writes the replicated columns. That writer runs headless, or inside the player runtime that is hosting. A client predicts what it owns and derives what the tick already implies. The packet carries what the client cannot derive, and only for the entities that connection can see, under a byte budget. A mesh swap replicates the source `AssetId`. A runtime that joins afterwards sees the mesh that is current.

## 2. What is already true

| Piece | Where | What it does |
|---|---|---|
| Scene storage | `Scene` in `crates/scene/src/types.rs` | `Floor`, `Vec<Wall>`, `Vec<Solid>`, `Vec<Light>`. No id on the object |
| Live names | `Handles::index` in `crates/mcp/src/live.rs` | `wall:<id>` assigned from 1 when the endpoint host is built. Not sent anywhere |
| Change detection | `Host::with_frame` | Clones `Scene` and compares it |
| Draw keys | `FixedPart::Wall(usize)` in `crates/render/src/world.rs` | Vec index. A removal shifts the tail |
| Bodies | `World::insert` in `crates/physics/src/lib.rs` | Returns `usize`. Two AABB trees, static and dynamic (`crates/physics/src/broadphase.rs`) |
| Source hash | `sha256` in `crates/load/src/sha256.rs` | `ImportDoc.asset_id = sha256(bytes)` in `crates/load/src/import.rs` |
| Cook header | `crates/cook/src/format.rs` | Writes `Cooked.asset_id` after the magic and the version. The trailer is `sha256` of the body, and it is not the asset id |
| Package lookup | `Package::line_for` in `crates/load/src/cooked.rs` | Finds the manifest line whose first field is that hex id |
| Agent endpoint | `genos-mcp` on `127.0.0.1` | Loopback edits of scene handles. [CONTEXT.md](../../../CONTEXT.md) says this is not a game network |
| Session | none | [Architecture](../../architecture.md) still says gameplay networking waits |

PR #14 locks the numbers this file uses. They are copied here so a session commit does not depend on an unmerged branch for the values:

| Name | Value |
|---|---|
| Tick | `1/60` s (`SIM_HZ = 60`) |
| Snapshot | every 3rd tick, 20 Hz, and also when a spawn or despawn is waiting |
| Snapshot ring | 32 per connection. A baseline that has fallen out of the ring is baseline 0 |
| Byte budget | 192 bytes of packed fields per snapshot per connection (`BYTE_BUDGET`). 192 × 20 = 3840 bytes/s, which is 3.75 KiB/s |
| Interest | near: the view cell and the eight neighbours, cell 32 m (`CELL_MM = 32_000`), eligible every snapshot. Far: out to 128 m, eligible every 4th snapshot (5 Hz). Past 128 m: omitted, except always-relevant and the owned object |
| Correction deadband | 50 mm on an axis, or 512 yaw units (about 2.8°). Inside it, the pose is not sent and a remote picture does not snap |
| Prediction ring | 128 ticks, every owned entity |
| Interpolation | 6 ticks behind the newest snapshot, 100 ms. A gap extrapolates for 3 ticks (50 ms), then holds |
| Lag compensation | 16 ticks of hit poses. Damage is not predicted |
| Command repeat | the last command, at most 4 ticks, then a zero command |
| Command window | every command newer than the ack, capped at 64 |
| Notices | reliable, ordered, at most 4 per datagram, payload at most 256 bytes |
| Lost snapshot | not retransmitted. The next delta uses whichever ack is still in the ring |
| Checksum | FNV-1a over replicated fields in scene-handle index order. Not a cross-machine physics golden |

PR #14's `EntityId` is the scene handle. PR #14's flag bit 128 stays reserved for the cause word. `MeshRef` does not use it.

## 3. Revisions

These are the only places this file changes PR #14.

1. **The scene handle is the wire id.** Columns are archetype columns in `genos-scene`. `genos-sim` is not added. The snapshot names the generational handle the ECS implementation stores. PR #14's `EntityId` and the physics plan's `NetId` are that handle. There is no second generational id. `BodyId` stays the physics id. The dirty bit is the per-component change tick. Replication does not store another tick.
2. **The writer may be a listen-server.** PR #14's refusal of shared mode stays. One process is still the only writer. That process may be the player runtime.
3. **The record is a component list, and a pose is a packed residual.** The fixed flag byte does not grow a bit for every new component. Structural flags stay spawn and despawn. A hash field is absolute. A quantized pose is the difference from the last acked quantized pose, bit-packed, and omitted inside the deadband.
4. **A datagram payload is at most 1200 bytes.** A snapshot that does not fit is sliced. Apply waits for every slice of that tick. A missing slice is a lost snapshot, and the next tick sends again. This is the late-joiner rule PR #14 left to UDP fragmentation. The steady field budget is 192 bytes, not PR #14's 1024.
5. **Interest is a near rate and a far rate, under that budget.** The 32 m cell is the near set. Out to 128 m is the far set, eligible less often. `Broadphase::fill` stays the contact-pair list. It is not the interest query.
6. **Owned entities predict. Remote entities interpolate and briefly extrapolate.** PR #14 predicted the pawn only and held on any gap. A correction inside the deadband does not replay and does not snap.
7. **Continuous animation is not a field.** Phase and blend weights are computed locally. An emote or an attack is a start tick. PR #14 had no animation record. PR #163's 36-byte record is the one this file refuses.
8. **Relay, NAT punch, and host migration are specified and not built first.** PR #14 stopped at "state authority does not move." Migration, later, moves the whole writer. It does not move one object's writer.

D1–D54 are untouched. Replay framing wraps whatever datagram the session sends. The agent tool, the mod loader, and the lockdown profiles stay that plan's problem. Dev and staging grants, and the release player compiling the tool out, stay as that plan recommends. This file does not reopen them.

## 4. Topologies

State authority is the right to write a replicated column. Input authority is the right to send commands for one object. A guest is a player runtime that is not the writer.

| Topology | Who writes | What a guest receives | What the hosting player can do | When |
|---|---|---|---|---|
| Dedicated | Headless authority runtime | The interest set | There is no player on that process | Launch |
| Listen-server | The player runtime that is hosting | The interest set | Read and write every column before the snapshot is built | Launch, except `competitive` |
| Relay | The same writer. The relay copies UDP and holds no scene | The same interest set. Peer addresses stay at the relay | Unchanged | Later |
| NAT punch | The same writer, on a direct UDP path when the punch works | Unchanged | Unchanged | Later. Failure stays on the relay |
| Host migration | The nominee, after a baseline | The interest set. The nominee has already applied an omniscient baseline | The nominee learns every replicated column at the handoff | Later. Refused in `competitive` |
| Shared object owner, or lockstep | Every simulating peer | The world it simulates | Write the truth | Refused |

`competitive` calls `SessionSettings` with `topology=dedicated`. A `topology=host` start in that mode returns the error `competitive refuses a listen-server` and does not tick. The other modes (`vanilla`, `modded`, `sandbox`) accept `host`. Those mode names are PR #14's. This file uses them and does not redefine them.

Launch reachability is `--bind IP:PORT` on an address the guests can send to. A listen-server behind a NAT does not become reachable by punching. That limit is why the relay is the next stage rather than a hidden dependency of the first one.

### Dedicated

`genos-session` links `genos-scene`, `genos-net`, and `genos-json`. It does not link `genos-window`, `genos-render`, or `genos-audio`. Arguments `--scene`, `--bind`, `--frames`, `--mode`, `--topology`. `--topology dedicated` is the default. `--bind off` runs the tick in-process and prints one JSON report. The hosted process uses a real bind. Genos, the other repository, starts that process. This repository does not grow a matchmaker.

The dedicated process stores every replicated column, including `MeshRef`. It loads the collision subset for hashes it simulates. It does not upload a texture and it does not draw the outfit.

### Listen-server

The player binary runs the same tick function. `--topology host` (argument `--host` is the same switch) binds the socket and creates the local player as a connection whose commands skip `Link`. Guests are ordinary connections. The local player still has input authority on one object. It does not get a second writer path. A guest's notice is applied by the tick, or refused. The guest's process never calls the column setter for a replicated component.

The listen-server player sees columns outside its interest set because the columns are in its process. That is the cheat. A wallhack or a health edit on that process is a write the guests will replicate. There is no second check that catches it. Sessions that cannot accept that player use dedicated. When this process exits, every guest drops the session. There is no nominee at launch. The test for that drop is `host_leave_ends_the_session`.

### Relay and NAT punch

Both are edges under the same datagram. `Edge` is `send` and `recv`. The three implementations are `Link` (in-process loss), `UdpEdge` (`std::net::UdpSocket`, nonblocking), and later `RelayEdge` (UDP to a relay address). The codec does not branch on which one is installed.

The relay copies one datagram from one mapped address to another. It does not parse a snapshot and it does not link `genos-scene`. Genos is the product that runs a public relay. This repository's later test is an in-process forwarder: bytes in equal bytes out, and the forwarder has no `Scene`. Keeping the products apart means this repository does not become the hosting service.

Punch, later, uses the relay as the introducer. Each peer learns the other's public endpoint. Each sends one punch datagram. If both have seen a punch within 2 seconds, later datagrams go direct. Otherwise they stay on the relay. The test is a fake NAT table that drops inbound until that flow has sent outbound. A failed punch still delivers the outfit snapshot, through the relay. Punch is an optimisation of the path, not a second protocol.

`quinn` is not how this gets reliable streams. Steam Networking Sockets is not how this gets a relay. ADR 0007 keeps the socket in this repository, on the standard library. A game that later needs Valve's relay asks for a new ADR that adds one `Edge` and leaves the datagram alone. This file does not grant that.

### Host migration

Migration moves the writer. It does not split the writer across peers.

1. The authority stops taking new commands at tick `T`.
2. It sends the nominee a baseline-0 snapshot of every replicated scene handle, sliced at 1200 bytes, outside `BYTE_BUDGET`. The nominee learns objects that were outside its interest. That is the leak.
3. Each guest receives a baseline-0 of its own interest set at tick `T`, and a reliable notice `migrate` (notice name `4`) that names the nominee. Guests do not receive the omniscient snapshot.
4. Guests clear the prediction ring and the interpolation buffer, then apply the baseline. They do not resimulate from commands.
5. Datagrams whose writer epoch is the old one are dropped. The epoch is the handoff counter. It is not the scene handle's generation.
6. The nominee's next tick is `T+1`. The baseline header carries the next scene-handle index and the live generations, so a spawn cannot reuse a live id or resurrect a dead one.

`competitive` refuses steps 1–6 and ends the match instead. A driven vehicle's wheel scalars have to be in the schema before migration is turned on, or the nominee would respawn the car from the prefab default. `MeshRef` is the same requirement, which is why the outfit test exists before this stage.

## 5. Identity, spawn, and prefabs

The scene handle is `{ index: u32, generation: u32 }`. That is the generational id the ECS implementation stores, and it is the id on the wire. The physics plan's `NetId` is this handle. There is no second allocator. A read whose generation mismatches returns absent. A despawn bumps the generation. A stale snapshot cannot resurrect the object. The floor keeps the handle the scene file stored, which is index 0 when the file says so. The sector component rides on that object. Replication does not take index 0 away from the floor for a second origin object.

An object placed by the shared scene file keeps the handle the file stored. Two runtimes that loaded the same bytes agree on those ids before anyone sends a spawn. A runtime spawn is allocated by the authority. The guest inserts that index and generation. It does not allocate a local handle and map it. Vec order is not on the wire. A listen-server is the authority, so its allocator is the session allocator. A guest-local cosmetic that is not replicated is not inserted into that table.

Spawn and despawn are structural. They are queued during the tick and applied at the end, so a pass does not scan a set it is also mutating. The snapshot sets structural flag `1` for a spawn and `2` for a despawn. Those records are written even when the 192-byte field budget is spent. They still obey the 1200-byte slice cap. A despawn record carries no component payload.

An object that enters a connection's interest set is encoded as a spawn for that connection, even when the world spawned it earlier. An object that leaves the set is omitted. It is not sent with a hidden flag. The next time it is relevant, it is a spawn again, with every present component.

A prefab spawn is a notice. The authority copies the prefab's components onto a new scene handle and sets `PrefabSource` to the content hash. The snapshot carries that hash and the overrides, not the prefab file. A `.gprefab` hash is the SHA-256 of that file. A glTF model uses the source `AssetId`, which is the hash [asset loading](../asset-loading.md) already stores on `PrefabSource`. The record's kind byte says which. Peers that have the hash instantiate it. Peers that do not have it take the reliable file transfer PR #14 already describes, then apply the spawn. They do not allocate the handle. A tick pass does not eval Rhai to spawn. [ADR 0009](../../adr/0009-rhai-is-the-scripting-language.md) stays: a script enqueues a notice or it does not run.

Players, NPCs, and props are the same object. A player has `InputAuthority` set to its connection. An NPC and a prop have `InputAuthority` empty, so every player runtime interpolates them and extrapolates for a short gap. A cosmetic child with no scene handle is not an object on the wire. Its local transform is recomputed from the replicated root.

## 6. Schema, dirty ticks, and deltas

A column replicates only when it is in the registration table. The table is static: `component_id: u16`, field list, `write`, `read`. Launch ids:

| Id | Component | Fields |
|---|---|---|
| 1 | Pose | quantized `x_mm`, `y_mm`, `z_mm`, `yaw`. A delta sends the packed residual, not these widths |
| 2 | InputAuthority | `connection u16`, or `0xffff` for none |
| 3 | StunUntil | `tick u32` |
| 4 | Health | `i32` |
| 5 | Origin | `sector_x i32`, `sector_z i32`, on the origin object |
| 6 | MeshRef | `asset_id [u8; 32]`, `mesh u32` |
| 7 | AnimEvent | `kind u8`, `id u16`, `start_tick u32` |

`asset_id` is `ImportDoc.asset_id`. It is the 32 bytes in the `.gcook` header. It is not the cook trailer, not a path, and not a vertex.

Pose on the wire is the quantized root, millimetres and yaw, truncated toward zero. The stored column is the transform (and, once a body exists, the body pose the physics plan already quantizes). That component's change tick dirties component 1. There is no second stored pose, and there is no second tick beside the one the ECS stores. `InputAuthority`, `StunUntil`, `Health`, `MeshRef`, and `AnimEvent` are stored columns. `Origin` is stored on the floor object.

The writer sets the component's change tick to the tick it writes. A reader does not. Writing the same bytes still sets the tick. A component whose change tick is less than or equal to the connection's baseline tick is omitted. An object with no components and no structural flag is omitted.

A pose that is sent is not three `i32`s and a `u16`. It is the residual against the last acked quantized pose. `dx`, `dy`, `dz` are signed millimetres. `dyaw` is a signed yaw-unit difference, wrapped through the `u16`. If every absolute residual is inside the deadband (50 mm, 512 yaw units), the pose is omitted even though the change tick moved. The owner's own connection also omits that pose when the authority pose and the pose those inputs produce agree inside the deadband. Other connections still receive it when their own baseline is outside the deadband.

The residual is packed, not Huffman-coded:

| Piece | Common case (near, ≤ 127 mm and ≤ 127 yaw units) | Wider residual |
|---|---|---|
| Index | `u8` gap from the previous record in this datagram. `0` escapes to a full `u32` index | same |
| Pack byte | bits 0–1 are `0` (8-bit axes). Bit 2 is `0` (8-bit yaw) | bits 0–1 are `1` (16-bit) or `2` (32-bit). Bit 2 is `1` for 16-bit yaw |
| Axes | three `i8` | three `i16` or three `i32` |
| Yaw | one `i8` | one `i16` |

The common case is 6 bytes: gap, pack, three axes, yaw. A 16-bit residual with 16-bit yaw is 10 bytes. A baseline-0 spawn still writes the absolute quantized pose (`3 × i32` and one `u16`), because there is no ack to subtract from. `MeshRef` and `AnimEvent` stay absolute. A delta of a hash is not smaller.

`AnimEvent.kind` is `1` for an emote and `2` for an attack. `id` is the clip or action id in the cooked set. `start_tick` is the sim tick the authority started it. The component is present only while `start_tick + duration_ticks` is still ahead of the snapshot tick. Duration is read from the clip on each runtime. It is not a field. Clearing the component sends id 7 with a field mask of 0. A late joiner's baseline includes the event only when the clip is still playing.

Phase is `(tick - start_tick) / duration_ticks`, integer division on the sim tick, never wall time. The locomotion blend is the horizontal speed of the last two quantized poses, in millimetres over that interval, computed where the mesh is drawn. Blend weights, clip phase, and joint transforms are not registered. [PR #163](https://github.com/AnthonyPoschen/genos-engine/pull/163) Addition B's 36-byte record (state, phase, weights, blend parameters) is the continuous snapshot this table does not send. That pull request is being revised to these events. An outfit change is not an `AnimEvent`. It is `MeshRef`, because the current asset id has to be in the baseline after any "change" clip has ended.

Baseline 0 writes every live relevant object, as a spawn, with every field. That is the late joiner's snapshot and the migration baseline. Applying the same baseline and the same payload twice is the same columns.

Unregistered columns stay local: texture residency, the lighting cache, menu settings, interpolation scratch, the contact cache, island ids, broadphase nodes. [physics.md](../physics.md) already lists the body fields that stay local. Those lists still hold.

The datagram, authority to player, little-endian:

- `u32` packet sequence
- `u32` snapshot tick
- `u32` baseline tick (`0` is a full baseline)
- `u8` slice index, `u8` slice count
- `u16` object records, in ascending scene-handle index
- each record: `u32` index, `u32` generation, `u8` structural (`1` spawn, `2` despawn), `u8` component count, then each component as `u16` id, `u8` field mask, then the masked fields in registration order
- `u16` notice count, then the notices PR #14 already defined

A slice is applied only when every slice of that snapshot tick has arrived. Component field bytes, excluding spawn and despawn records, stop at `BYTE_BUDGET` (192). `MeshRef`'s 32 bytes count against that budget. A packed pose residual counts its packed size, not the unpacked `i32` width. Spawns are written first. If the spawns alone exceed 1200 bytes, they continue on the next slice of the same tick. Player-to-authority datagrams stay PR #14's command block: the acked snapshot tick, the acked notice sequence, and the unacked commands.

An unknown component id, a truncated field, or a slice count of 0 drops the datagram and increments `net.rejected_datagrams`. The connection keeps its old ack. The next snapshot rebuilds from that ack. One bad id does not apply a prefix of the record.

There is no serde. A derive macro is allowed later only if it emits the same `write` and `read` the table calls. The first seven components are written by hand.

## 7. Authority, prediction, interpolation

The authority runs every tick pass on every replicated object, in registration order, on the calling thread. The job system's serial mode is that order ([`job-system.md`](../job-system.md)). `step_parallel` may run on the authority only when `hash_world` equals `step` for that world. The predicting runtime calls `step`. A listen-server is the authority, so it uses the authority rule, not the guest rule.

A guest runs `step` for every object it has input authority for, and keeps each pose for 128 ticks. That set is the owned pawn and any other entity the connection owns, including a driven chassis. A snapshot for tick `T` restores that object and reruns `T+1 ..= local_tick` only when the quantized error is outside the deadband (50 mm, or 512 yaw units). Inside the deadband the predicted pose stands and `net.corrections` does not move. Other objects are not resimulated. Rhai does not run inside the rerun. Hierarchy under the owned body recomputes after the rerun and is not in the checksum.

Everyone else interpolates. The render tick is the newest snapshot tick minus 6. Millimetres lerp in integer math between the two samples that bracket that tick. Yaw uses the short arc. When the next sample is missing, the last step is extended for at most 3 ticks and then the picture holds. A sample inside the deadband of the rendered pose does not snap. A sample outside it replaces the rendered pose. NPCs, props, other players, and a vehicle this connection does not own use this path.

A driven vehicle sends throttle, brake, steer, and the gear request on the owner's command. The driver predicts the chassis with `step` and the deadband above. A passenger does not. The tyre solver stays out of the client step until that step hash-matches the authority, which [the physics plan](../physics/plan.md) already requires. Until then the client's chassis is the step without that solver. An error the solver would have corrected is outside the deadband and is sent. That is the bandwidth the missing solver costs. It is not a reason to leave the chassis unpredicted.

`InputAuthority` written during tick `T` selects commands at `T+1`. Commands from the old connection after `T` are dropped. Leave clears the component. State authority is not a field on the object.

`MeshRef` is not predicted and not replayed. The runtime that requested a swap may draw the requested mesh immediately. The next snapshot of that scene handle is the correction. If the authority refused, the picture returns to the previous id. Other runtimes draw the snapshot only.

Lag compensation stays PR #14's 16-tick rewind of hit poses. The strike is a button on the command, not a notice. The authority rewinds, queries, writes `Health`, and leaves the live poses. A guest does not register its own hit.

## 8. Interest and the broadphase

The near set is the 3 by 3 block of 32 m cells around the view, plus always-relevant objects, plus the object this connection has input authority for. A near object that left the deadband is eligible every snapshot. The far set is every other object inside 128 m of the view. A far object is eligible every fourth snapshot, which is 5 Hz. Past 128 m the object is omitted, not sent slowly. 4096 stored objects stay cheap because a connection's packet names this set. The scale test is eight views and one object past 128 m that must not appear.

Until bodies have stable ids, the cell is computed from the quantized pose, which is PR #14's grid. Once `BodyId` exists, the candidate set is an AABB overlap of those cells against the static tree and the dynamic tree in `crates/physics/src/broadphase.rs`. The trees are the index. The cell rule is the policy. `Broadphase::fill` emits contact pairs for the solver. A body in the cell with no contact is still relevant, so the interest query is not `fill`. Asleep bodies with an unchanged sleep bit send no pose, which [physics.md](../physics.md) already requires, and they remain in the set so a wake is sent.

Among the eligible dirty objects, priority starts at 0, gains 1 each snapshot opportunity, and gains 1 more for a near object. Selection sorts by priority descending, then by snapshots skipped, then by scene-handle index, and stops at 192 bytes. A written object resets both counters. A far object that has waited is eligible on its next fourth snapshot, not on every one. This is the Iris accumulation PR #14 already copied, under a smaller cap and a distance rate. There is no replication graph.

`competitive` may also drop an object the authority's occluders hide, which is PR #14's visibility cut. The object is omitted, not flagged. `sandbox` does not run that cut. The agent endpoint is not this cut. It remains the loopback scene editor.

## 9. Channels

Unreliable, not retransmitted: snapshots, including `MeshRef` deltas and baseline slices.

Reliable, ordered, resent until acked: notices. Launch names are PR #14's `1` join, `2` leave, `3` request, plus `4` migrate when that stage exists. A request does not write a column. The authority writes the column, and the snapshot publishes it. The request is not echoed to other players. A guest who applied a request payload would be a second writer.

`BondBreak` stays a notice. The result the picture needs after a break is still the spawned chunk poses in the snapshot. A chat line and a UI toast are notices. An emote, an attack, and an outfit are not: a late joiner must be able to see an emote that is still playing and the outfit that is current. Anything a late joiner must see as current state is a component, because a notice is not in the baseline. The outfit is `MeshRef` for that reason. The emote is `AnimEvent` for that reason, and it drops out of the baseline once the clip has ended.

The reliable file transfer of a missing asset or prefab is a separate stream of file bytes, still on `std` sockets, still not a gameplay datagram. PR #14 and the [asset plan](../assets/plan.md) already split it that way. `missing_subsets` is that list. The snapshot names the `AssetId` and leaves the bytes out.

## 10. Bandwidth

A connection gets 192 bytes of packed fields per snapshot, 20 times a second. That ceiling is 3840 bytes/s, 3.75 KiB/s. Spawns and the four notices sit outside those 192 bytes and inside the 1200-byte slice. The slice cap stays so a datagram does not depend on IP fragmentation.

A `MeshRef` change that only replaces the asset id is 32 field bytes, plus the record header (index, generation, structural, count, component id, mask: 13 bytes). The 45-byte record fits in the 192-byte budget once. The following snapshots omit it while the tick is clean. An outfit is not a glTF upload.

A late joiner's baseline is the interest set, not the 4096-object world. Spawns go out first, across slices if needed, still as baseline 0, until that connection acks. Poses that did not fit rotate in by priority on the following snapshots, still against baseline 0 until the spawn's tick is acked. The authority file and an omniscient migration baseline ignore `BYTE_BUDGET` and still slice at 1200. PR #14's 140 KB estimate for a full 4096-object baseline stays the upper bound for that omniscient case. A player connection does not receive it.

### A thousand visible characters

This is the steady picture the budget is for. One connection, 1000 characters inside 128 m, no spawns in this second. Idle characters send no pose. Locomotion sends no phase and no weight. The owner's own pose is omitted on the owner's connection.

| Group | Who | What is sent | Bytes |
|---|---|---|---|
| Near movers | 24 of 48, every snapshot, 6-byte residual | 24 × 6 × 20 | 2880 |
| Far movers | 80 of 952. The 5 Hz cap would be 400 updates/s. After the near rows, 48 bytes remain and 4 updates of 10 bytes fit | 4 × 10 × 20 | 800 |
| Idle | 896 | nothing | 0 |
| Events | none in this second | | 0 |
| Total | | | 3680 bytes/s, 3.6 KiB/s |

3680 is under the 3840 ceiling. The far movers wanted a 5 Hz cap and the accumulator gives them 80 updates/s across 80 characters, which is 1 Hz each. That is the reduced rate. A second with four `AnimEvent` or `MeshRef` writes spends the spare 8 bytes and then drops the lowest-priority far update. `net.budget_drops` counts those.

The same 1000 characters with an absolute pose (`3 × i32` plus a `u16`, 14 bytes) at 20 Hz would be 1000 × 14 × 20 = 280,000 bytes/s. Adding PR #163's 36-byte phase and weight record for the 104 movers, every snapshot, would add 104 × 36 × 20 = 74,880 bytes/s, because a playing clip changes phase every tick. Neither number is a budget this connection has.

The gate is `a_thousand_visible_characters_stay_under_four_kib`. It builds this second from the scheduler, with no socket, and fails if the payload exceeds 3840 bytes or if a locomotion row contains a phase or a weight.

## 11. Determinism

Inside one process, the same spawns, the same scene-file bytes, and `step` at `1/60` on one thread produce the same quantized poses and the same `AssetId` bytes. The parallel hierarchy reduce matches the serial reduce or it is not used. Wall-clock time, steal order, and datagram arrival time are not inputs. `Link` decides delivery. The tick consumes commands.

`genos-math` stays `f32`. Replicated positions are integer millimetres. A poke below a millimetre does not change the pose checksum. Cross-machine bit identity of the physics step is not a gate. A Windows authority and a Linux guest compare the quantized pose and the 32 hash bytes.

The outfit hash is exact on every machine because it is the SHA-256 of the source bytes, not a float. It is not folded into the pawn pose checksum. The outfit test compares the 32 bytes.

## 12. Security

The guest is not trusted with a column.

- A command tick outside `(acked, acked + 64]` is dropped.
- `aim_tick` is clamped into the 16-tick window. Older strikes do not rewind.
- A `MeshRef` request whose `AssetId` is absent from the session table is refused. The column stays. `net.refused_requests` counts it. In `vanilla` and `competitive` the table is the developer set. In `modded` it is that set plus the manifest. In `sandbox` it is any id the authority has loaded. A guest that has not got the bytes yet still only receives the id.
- Notice payloads longer than 256 bytes, unknown component ids, and bad generations do not apply.
- The release player binary does not contain the edit tool. That compile-out is PR #14's lockdown. This file does not weaken it.
- Join still hashes gameplay-relevant paths in the locked modes. A replaced file fails the join. A runtime that reports the official hash and draws something else is not caught by the hash. The interest cut is what keeps an occluded pose out of the packet.
- The listen-server player is inside the trust boundary. Joining one is trusting that process. `competitive` does not offer it.
- Migration hands the nominee the omniscient baseline. `competitive` does not offer that either.

There is no client-side authority "if the server agrees later." The snapshot is the agreement.

## 13. What the scene store has to expose

PR #131 is approved and being implemented. The first three rows are that implementation: the generational handle, the columns, and the per-component change tick. Replication reads them. It does not add a second id or a second tick.

| Need | Where it comes from | Done when |
|---|---|---|
| Generational scene handle. A reload keeps it. A stale generation misses | the ECS implementation | `reload_keeps_scene_handles` |
| Columns behind the vecs, signatures of `walls()`, `solids()`, `lights()` unchanged | the ECS implementation | the camera room tests still pass |
| A change tick on each component. A one-field edit does not clone `Scene`. A reader does not bump the tick | the ECS implementation | `scene.full_compares == 0` and `a_write_bumps_the_component_tick_and_a_read_does_not` |
| Apply by that handle. Shared-file handles match on two loads. A guest inserts the authority's handle. A removal does not retarget the survivor | replication, on the handle above | `shared_file_handles_match_on_two_loads`, `a_guest_inserts_the_authority_handle`, `a_removal_does_not_retarget_the_survivor` |
| Prefab hash on `PrefabSource` | the ECS prefab step | `prefab_override_does_not_write_the_file` |
| `MeshRef` column of source `AssetId` plus mesh index | replication, on that store | `mesh_ref_stores_the_source_sha256` |

E2, E4, and E7 are not in this table. E4's parent link is required before a replicated child transform is believed. It is not required to replicate a root `MeshRef`. The snapshot module in `genos-scene` is a pure function of the columns. It does not call `std::net` and it does not eval Rhai. `genos-net` calls that function. `genos-scene` does not depend on `genos-net`.

## 14. Worked example: an outfit swap

One scene object, the player, scene handle `{ index: 4, generation: 1 }`. It has a `MeshRef`. The coat file's bytes hash to `COAT = sha256(coat.glb)`, 32 bytes, the value `import_bytes` would store. The cloak file hashes to `CLOAK`. Both ids are in the session table. The mesh index stays `0`. The object is inside the other player's 32 m neighbourhood. Pose is clean.

The owning player runtime wants the cloak. It does not set the column. It enqueues notice name `3` (request), payload under 256 bytes: request kind `1` (set `MeshRef`), index `4`, generation `1`, the 32 bytes of `CLOAK`, mesh `0`. It may draw the cloak locally at that moment.

The authority tick checks the generation and looks up `CLOAK` in the session table. The id is present, so it writes `MeshRef { asset_id: CLOAK, mesh: 0 }` and sets that component's change tick to `T`. The transform component's change tick is unchanged, so the wire pose stays clean. A miss would leave `COAT` in the column, count `net.refused_requests`, and write nothing for the other runtime to apply. The owner's local draw would snap back when the next snapshot still says `COAT`.

The next snapshot tick (`T` divisible by 3) builds a delta against the connection's acked baseline, where this object still has `COAT` and mesh `0`. The record is:

```text
index            4
generation       1
structural       0
count            1
component_id     6          MeshRef
field_mask       0x01       asset id only; mesh bit 0x02 is clear
asset_id         CLOAK      32 bytes
```

Pose is absent because the transform tick is older than the baseline. The vertex buffer is absent because it was never a field. The cook trailer is absent because it is not `asset_id`. The field payload is the 32 bytes of `CLOAK`, which counts against the 192-byte budget. The record is 45 bytes and fits in one 1200-byte slice (`slice index 0`, `slice count 1`).

The guest applies by the scene handle, not by vec index. It writes the 32 bytes onto `MeshRef`. It resolves them the way the cook package already does: `Package::line_for(&CLOAK)`, or the object cache path for that hex. `decode_geometry` on the `.gcook` returns a header `asset_id` that must equal `CLOAK`. The renderer then draws that mesh. Residency stays a local table. A missing file does not stall the tick. The guest keeps the previous mesh, or draws nothing while bounds stay, which is the missing-geometry rule in [asset loading](../asset-loading.md), and the reliable file stream fetches the bytes. The gameplay datagram does not grow to hold them.

A runtime that joins after the swap has no ack. Its baseline is 0. The object is a spawn in its interest set:

```text
index            4
generation       1
structural       1          spawn
count            3          Pose, InputAuthority, MeshRef
```

Pose and input authority are the other two components, every field set. The `MeshRef` component in that same record is:

```text
component_id     6
field_mask       0x03       asset id and mesh index
asset_id         CLOAK      32 bytes
mesh             0          u32
```

`COAT` is not in the datagram. The joiner never applies the coat. The entity log on the authority may still mention the coat. That log is not a player datagram, which is PR #14's rule, and it is why the late joiner sees the current outfit rather than the history.

The same baseline is what a later migration nominee applies. If `MeshRef` were unregistered, the nominee would dress the player from the prefab default. The spawn record above is the reason registration is a gate before migration.

A guest outside the neighbourhood receives neither record. Walking into the cell sends the spawn form, with `CLOAK`, because entering interest is a spawn for that connection.

A planted bug that writes the `.gcook` body, or the vertices, into the snapshot fails `outfit_delta_is_the_source_asset_id`: the payload length is 32, and those 32 bytes equal `sha256` of the source file.

## 15. Settings, probes, and the debug binary

`SessionSettings` is an `EngineSettings` value on `Scene`, the same three surfaces as [the settings principles](../../engine/settings-principles.md). Keys: `topology` (`dedicated` or `host`), `mode` (`vanilla`, `competitive`, `modded`, `sandbox`), `cell_m` (default 32), `far_m` (default 128), `byte_budget` (default 192), `snapshot_every` (default 3). `--host`, `--bind`, `--mode`, `--cell-m`, `--far-m`, `--byte-budget`, and `--snapshot-every` are the arguments. `GENOS_SESSION_TOPOLOGY` and the matching `GENOS_SESSION_*` variables apply only when that argument was omitted. MCP `get_session_settings` and `set_session_settings` read and write the text form. The debug menu edits the same keys.

The binary is `debug/networking`, package `genos-net-view`, on the shared scaffolding in [debugging](../debugging.md). PR #14 called this `examples/net-view`. The folder moves under `debug/` so it matches the other systems. Headless: `--headless --players N --frames N --latency MS --loss PERCENT --jitter MS --seed U --topology dedicated|host --check --out DIR`. Default output is `./genos-debug/networking/`. `--out` overrides it. The process does not write under `$HOME`. The report is JSON through `genos-json`: rows of scene handle, authority millimetres, guest millimetres, corrections, and the `MeshRef` hex. Two runs with the same arguments write the same bytes. The window, when a later commit opens one, is the slider panel PR #14 specified. The headless test does not open it.

| Failure | Probe |
|---|---|
| A guest wrote a column | `net.guest_writes`, must stay 0. Test `a_guest_cannot_write_mesh_ref` |
| A datagram was malformed | `net.rejected_datagrams` |
| An outfit id was not in the table | `net.refused_requests` |
| The budget dropped a dirty neighbour | `net.budget_drops` |
| The steady crowd exceeded 3.75 KiB/s | `net.payload_bytes`, test `a_thousand_visible_characters_stay_under_four_kib` |
| Interest included an object past 128 m | `net.interest_count`, test `a_far_entity_is_outside_the_neighbourhood` |
| A sub-threshold pose was sent, or a locomotion row carried phase | `net.payload_bytes` on that test |
| Prediction disagreed outside the deadband | `net.corrections` |
| The listen-server started in `competitive` | the start error `competitive refuses a listen-server` |
| The outfit payload was mesh bytes | `outfit_delta_is_the_source_asset_id` |
| A late joiner applied the coat | `a_late_joiner_sees_the_cloak_and_not_the_coat` |

`debug/networking --check` runs the two-client outfit case and exits non-zero if the guest's `MeshRef` hex differs from the authority's.

## 16. Stages

Each stage is one later commit, with one test. This change is none of them. G1 and G2 are the ECS implementation that is already approved: the generational handle and the per-component change tick. Replication consumes them. G3 and G4 sit on that store and do not open a socket, so they do not wait on D1–D54. G5–G10 are the session. They wait until this ADR is accepted. They do not build the replay file, the agent tool, or the mod loader.

| Stage | Change | Gate |
|---|---|---|
| G1 | Generational scene handle, from the ECS implementation | `reload_keeps_scene_handles`. Plant `renumber_on_load`: the test fails |
| G2 | Per-component change tick, from the ECS implementation | `scene.full_compares == 0` and `a_write_bumps_the_component_tick_and_a_read_does_not`. Plant `clone_to_detect`: the counter is non-zero. The snapshot reads this tick |
| G3 | Apply by that handle. No second id | `shared_file_handles_match_on_two_loads`, `a_guest_inserts_the_authority_handle`, `a_removal_does_not_retarget_the_survivor` |
| G4 | Registration table, `MeshRef`, and `AnimEvent` | `an_unregistered_component_is_absent_from_the_delta`, `mesh_ref_stores_the_source_sha256`, `outfit_delta_is_the_source_asset_id`, `an_emote_is_a_start_tick_and_locomotion_sends_no_phase` |
| G5 | `Link` with latency, jitter, loss, seed | `a_seeded_link_drops_the_same_datagrams`. Two scene stores and one `Link`, no socket. Checksums match under loss |
| G6 | Owned-entity restore past the deadband, remote interpolation and 3-tick extrapolation | `a_stun_replays_the_owned_pawn_and_leaves_the_bystander`, `a_sub_threshold_error_does_not_replay`, `a_remote_pose_lerps_and_a_gap_extrapolates_then_holds`, `the_driver_predicts_the_chassis_and_a_remote_vehicle_interpolates` |
| G7 | Near and far interest, priority, 192-byte budget, 1200-byte slices, the thousand-character second | `a_far_entity_is_outside_the_neighbourhood`, `the_budget_rotates_three_dirty_neighbours_and_still_spawns`, `a_thousand_visible_characters_stay_under_four_kib`. Plant `interest_uses_contact_pairs`: a body in the cell with no contact disappears, and the test fails |
| G8 | Dedicated process, listen-server, and `SessionSettings` | `session_settings_round_trip`. `a_headless_session_ticks_without_a_window` (the `genos-session` manifest contains no `genos-window`). `a_guest_cannot_write_mesh_ref`. `competitive_refuses_a_listen_server`. `host_leave_ends_the_session` |
| G9 | Two guests, one writer, the outfit, then a late joiner | `a_late_joiner_sees_the_cloak_and_not_the_coat`. Latency 100 ms, loss 0, seed 1. The guest's 32 bytes equal `CLOAK`. The joiner's first applied datagram contains `CLOAK` and does not contain `COAT`. A second run writes the same report |
| G10 | `debug/networking --check` | the G9 report under `./genos-debug/networking/`, exit 0. `--out` moves that directory |

G7's tree query waits until the broadphase can answer an AABB. The pose grid satisfies G7 before that. The planted bug `interest_uses_contact_pairs` is the test that the tree query, once it exists, is still the neighbourhood and not `fill`.

Later, each with the same harness:

| Stage | Change | Gate |
|---|---|---|
| L1 | Relay edge | Bytes the authority sends equal bytes the guest receives. The forwarder does not link `genos-scene` |
| L2 | Punch, relay fallback | A fake NAT that never delivers a punch still completes the G9 outfit through the relay. A NAT that delivers the punch completes it with no relay datagram after the punch |
| L3 | Host migration | `a_migrated_host_keeps_the_cloak`: the nominee's `MeshRef` is `CLOAK`, generations are unchanged, a datagram from the old writer is rejected. The nominee's baseline contains an object that was outside its interest, which is the leak, recorded and not treated as a bug. `competitive_refuses_migration` |

The two-client CI test is G9. It uses `Link`, not a GPU and not a second machine. G8's loopback socket test is one datagram on `127.0.0.1:0`, the same shape as PR #14's N20, and it is the only test that binds a port.

## 17. Five lines

- Replicated state: opted-in columns on objects named by their scene handle. The steady pose is a packed residual against the last ack, omitted inside the deadband. `MeshRef` is the source `AssetId`. An emote or an attack is `AnimEvent` (kind, id, start tick). Phase, blend weights, and joint transforms are not fields. Spawns carry the prefab hash. Mesh bytes stay local.
- Authority: the dedicated process or the listen-server. Guests send commands and notices. `competitive` is dedicated only. Input authority moves at a tick boundary. State authority moves only in a later migration, and not in `competitive`.
- Prediction / rollback: every owned entity, including a driven chassis, by replaying `step`. A miss inside the deadband does not replay. Other players, NPCs, props, and a remote vehicle interpolate 100 ms back, extrapolate for at most 3 ticks, then hold. `MeshRef` previews on the requester and corrects from the snapshot.
- Determinism: serial `step` at `1/60` in one process. The wire compares quantized poses and SHA-256 bytes. Cross-machine `f32` identity is not required.
- Headless server: `genos-session` has no window, no Vulkan, and no audio. It stores `MeshRef` and does not draw it. A listen-server is not this binary.

## References

- [ADR 0042](../../adr/0042-multiplayer-topologies.md)
- [ADR 0007](../../adr/0007-native-calls-go-to-the-system-and-vulkan.md)
- [ADR 0009](../../adr/0009-rhai-is-the-scripting-language.md)
- [ADR 0035](../../adr/0035-cooked-package-runtime-load.md)
- [ADR 0033](../../adr/0033-physics-scale-broadphase-and-sleep.md)
- [Scene ECS](../ecs-scene.md)
- [Job system](../job-system.md)
- [Physics](../physics.md) and [the physics plan](../physics/plan.md)
- [Asset loading](../asset-loading.md) and [the asset plan](../assets/plan.md)
- [Settings principles](../../engine/settings-principles.md)
- [Debugging](../debugging.md)
- [Architecture](../../architecture.md)
- [PR #14](https://github.com/AnthonyPoschen/genos-engine/pull/14), the server-authoritative plan this file extends
- [PR #131](https://github.com/AnthonyPoschen/genos-engine/pull/131), the job system and the scene ECS
- [PR #163](https://github.com/AnthonyPoschen/genos-engine/pull/163), the animation ADR whose 36-byte phase record this file replaces with `AnimEvent`
