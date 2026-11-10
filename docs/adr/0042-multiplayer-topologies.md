# 0042: One state authority, dedicated or listen-server

Status: **Proposed** (10 Nov 2026). Anthony reviews before any session code. Design: [`docs/systems/core-networking/topologies.md`](../systems/core-networking/topologies.md).

0040 is claimed by the vehicle-domains PRs (#176, #201) and by an audio draft (#215). 0041 is the other audio draft (PR #212). This number is the next free one.

This extends the server-authoritative plan in [PR #14](https://github.com/AnthonyPoschen/genos-engine/pull/14) (`docs/systems/core-networking/plan.md` on `cursor/core-networking-plan-c247`). It does not merge that plan. It reads the scene store from [PR #131](https://github.com/AnthonyPoschen/genos-engine/pull/131), which is approved and being implemented: a generational scene handle and a change tick on each component ([`docs/systems/ecs-scene.md`](../systems/ecs-scene.md), [`docs/systems/job-system.md`](../systems/job-system.md)). It binds to [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md), [ADR 0009](0009-rhai-is-the-scripting-language.md), and [ADR 0035](0035-cooked-package-runtime-load.md).

## Context

Master has no game socket and no entity store. `Scene` in `crates/scene/src/types.rs` is a floor, a `Vec<Wall>`, a `Vec<Solid>`, and a `Vec<Light>`. None of those structs has an id. `Handles::index` in `crates/mcp/src/live.rs` assigns `wall:<id>` from 1 whenever a host is built. `Host::with_frame` clones the whole `Scene` to see what changed. `World::insert` returns a `usize`. Removing a wall renumbers every later index.

PR #14 already specifies the session that should sit on a stable id: 60 Hz tick, unreliable snapshots at 20 Hz, deltas against the last acknowledged snapshot, a 32-deep ring, a 32 m interest neighbourhood, a 1024-byte pose budget, pawn-only prediction with restore-and-replay, 100 ms interpolation, 16-tick lag compensation, and `std::net::UdpSocket`. It refuses a second writer, peer-to-peer rollback of the whole world, and any crate that owns the socket. Its open questions D1–D54 are about replay, agent tools, mods, and session modes. They are not answered here.

That plan puts the columns in a new `genos-sim` store whose `EntityId` is also the physics `BodyId`. The approved scene store is the archetype tables in `genos-scene`. Its id is the generational scene handle, and each component carries a change tick. A replicated component has to be one of those columns, and the snapshot has to name that handle. A second id, or a second tick, would drift from the store the implementation is already building.

Players, NPCs, and props have to appear on every joined runtime. The authority that writes them has to be able to run as a headless process, and as the process a player is already playing (a listen-server). Reachability past a NAT, and moving that writer when the listen-server leaves, are real sessions. They are not required to prove the first replicated component.

`AssetId` is already the source SHA-256. `import_bytes` sets `ImportDoc.asset_id` to `sha256(bytes)` (`crates/load/src/import.rs`, `crates/load/src/sha256.rs`). The `.gcook` header stores that same 32 bytes (`crates/cook/src/format.rs`). The cook trailer is the SHA-256 of the record body, a different value. A mesh swap has an id to send.

## Options

| Option | For | Against |
|---|---|---|
| Dedicated authority only (PR #14 as written) | One writer a guest cannot see inside. Competitive stays small | A two-player session needs a process that is not playing. NAT and a host leaving are out of scope forever unless a later ADR adds them |
| Shared state, one owner per object (Photon shared mode) | A peer keeps its own prop without a host | Every owner is a writer. A guest can publish a pose the others must trust. Physics has no cross-machine `f32` identity to fall back on |
| Lockstep of the whole world (GGPO, Factorio) | No server, and every peer simulates every object | Both peers must resimulate every body bit-identically. [physics.md](../systems/physics.md) refuses a cross-machine golden hash. A 4096-body world would ship every body to every peer |
| **One writer, two places it can run** | Dedicated and listen-server share the codec, the columns, and the cheat boundary for guests. Relay, punch, and migration reuse the same bytes later | The listen-server player shares a process with the writer, so that player can read and edit columns the guests never see |

## Decision

1. **One state authority.** Replicated components have one writer, the authority runtime. A player runtime sends commands and notices. It does not write a column. Input authority is the only per-object grant that moves, and it moves at a tick boundary. Per-object state authority does not move. Lockstep of the whole world stays refused.
2. **Launch runs that writer in two topologies.** *Dedicated:* a headless process, no window, no Vulkan, no audio, the process Genos starts. *Listen-server:* the same tick inside a player runtime. The local player is a connection with no socket delay. Guests connect to it. `competitive` refuses a listen-server and refuses host migration. `vanilla`, `modded`, and `sandbox` allow a listen-server. When the listen-server process leaves, the session ends. Nothing is migrated.
3. **Later, same codec.** A relay that copies datagrams and does not simulate. NAT punch that falls back to that relay. Host migration that hands the whole writer to one nominee with a baseline snapshot. Competitive does not migrate. None of the three is in the first session.
4. **The scene handle is the wire id.** The generational id the ECS implementation stores, `{ index: u32, generation: u32 }`, is what a snapshot names. PR #14's `EntityId` and the physics plan's `NetId` are that handle. There is no second generational allocator and no `NetId` column. `BodyId` stays the physics id. The per-component change tick is the dirty signal. Replication does not write another tick. There is no `genos-sim`. `genos-scene` holds the columns and the pure snapshot bytes. `genos-net` holds `Link` and the `UdpSocket`. The scene crate does not call `std::net`.
5. **The wire names `AssetId`, not mesh bytes.** `MeshRef` replicates the source SHA-256 and a mesh index. The `.gcook` body, the vertex buffer, and the cook trailer stay off the gameplay datagram.
6. **Transport stays the standard library.** [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md) is not amended. `quinn`, `tokio`, `renet`, and Steam Networking Sockets are sources. They do not own the socket. Reliable notices and unreliable snapshots are channels this repository writes on `std::net::UdpSocket`.

The rates, the interest rule, the pawn prediction, and the interpolation delay are the ones PR #14 locked. They are restated in the design so a session commit does not have to re-derive them. D1–D54 stay open. This ADR does not start N1.

## What the ECS implementation provides

PR #131 is approved. Its implementation is the generational scene handle and a change tick on each component. Replication reads those. It does not add them again.

- The handle is `{ index: u32, generation: u32 }`, stored on the object. A reload keeps it. A stale generation misses. The snapshot record is that index and that generation. Apply is a lookup of the handle. A shared scene file restores the same handles on every runtime, so those objects agree before a spawn message. A runtime spawn uses the handle the authority just allocated. The guest inserts that index and generation. It does not allocate a second handle and map it.
- The change tick sits on the component. The writer bumps it. A reader does not. Lighting change records are one consumer, in handle order. The snapshot is the other: a component whose tick is at or before the connection's baseline is omitted. A one-field edit does not clone `Scene`.
- `BodyId` stays the physics id. It is not the handle and it is not on the wire as identity.
- The floor keeps index 0 when the scene file gives it that handle. The sector component rides on that object. Replication does not reserve index 0 for a second origin object.

Still to add for the session, on top of that store: which components are registered, `MeshRef` as the source `AssetId` plus a mesh index, and `PrefabSource`'s content hash when a spawn is a prefab. Undo, the parent walk, and Rhai `get`/`set` are not on this path. The authority tick runs on the caller, in serial order, which is the job system's oracle. A cosmetic child with no scene handle is not on the wire.

The registration table is hand-written `write` and `read` functions and a stable `u16` component id. A derive macro may emit those functions later. Serde is not the codec. An unregistered column is absent from every datagram.

## Rejected

- **Replicating `Vec<Wall>` and the MCP handle.** An index shifts when a neighbour is removed. PR #14 and ADR 0032 already refuse this.
- **`genos-sim` as a second entity store.** The snapshot and the picture would name different ids for one prop.
- **A `NetId` column beside the scene handle, and a replication tick beside the component change tick.** The ECS implementation already has the id and the tick. A second pair would be the bolt-on this decision refuses.
- **`quinn` or any QUIC crate.** The crate would own the socket, and the maintained runtime is `tokio`, which ADR 0007 rejects. QUIC does not punch NAT.
- **Steam Networking Sockets, `renet`, `lightyear`, `bevy_replicon`.** Each owns reliability and the socket. Steam also owns the relay. The datagram would be that library's.
- **Shared per-object state authority, and lockstep.** Stated in the option table. Input authority still moves. State authority does not.
- **Huffman or xor-deltas on the first codec.** PR #14 already omits unchanged fields and writes absolute values. That is the compression.
- **Spending PR #14's flag bit 128 on `MeshRef`.** That bit is reserved for the cause word. The record grows a component list instead. The list is in the design.

## Consequences

- A guest of either launch topology sees the interest set and nothing else. The listen-server player can read and write every column in-process. The mitigation is the session mode, not a detector. `competitive` does not start a listen-server.
- A host behind a NAT is not reachable at launch. Direct `--bind` is the launch path. Relay and punch are the later path, and they do not change who writes.
- Host migration, when it exists, shows the nominee the whole replicated world at the handoff. That leak is why `competitive` never nominates.
- The outfit swap in the design is the acceptance test: a dirty `MeshRef` sends 32 source-hash bytes, a late joiner applies a baseline of the cloak, and the coat's hash is not in that datagram.
- No crate, socket, or test is added by this decision. Gameplay networking still waits in [architecture](../architecture.md).

## Five lines

- Replicated state: opted-in columns on scene objects, named by the generational scene handle. Launch columns include the quantized pose, input authority, and `MeshRef` as a source `AssetId` plus a mesh index. The dirty signal is that component's change tick. Mesh bytes, residency, lighting caches, and menu settings stay local.
- Authority: one writer. Dedicated or listen-server. Guests send commands and notices. `competitive` is dedicated only. Input authority moves at a tick boundary. State authority does not move until a later migration, and not in `competitive`.
- Prediction / rollback: the owned body only, restore and replay of `step`. Other objects, including other players, NPCs, props, and a driven vehicle, interpolate. `MeshRef` is not in the replay. A local outfit preview is corrected by the snapshot.
- Determinism: one process, serial tick, `step` at `1/60`. The wire compares quantized poses and `AssetId` bytes. Cross-machine `f32` identity is not required. Clients do not run the tyre solver.
- Headless server: the dedicated process links no window, no Vulkan, and no audio. It still stores `MeshRef` so a snapshot can name the outfit. It does not upload a texture. A listen-server is the player process and is not this headless build.
