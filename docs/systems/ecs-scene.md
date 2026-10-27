# Scene ECS (design)

Status: proposed, 27 Oct 2026. Owner: Engine. No code in this change.

The live scene is four arrays and a side table of names. That is enough for one scripted room. It is not enough for the four systems that have to name an object and find it again later. [Animation](animation.md) needs a parent link for joints. The [physics plan](physics/plan.md) scales to thousands of bodies and says snapshot order is `NetId` order once bodies live on stable ids. Save, load, and MCP undo need a name that survives a removal and a reload. Lighting invalidation needs a change record per object that moved, which [ADR 0013](../adr/0013-lighting-invalidation-is-local.md) already specifies and the scene does not yet emit. The [job system](job-system.md) runs the wide passes; this design is the storage those passes iterate.

Binds to [ADR 0009](../adr/0009-rhai-is-the-scripting-language.md), [ADR 0013](../adr/0013-lighting-invalidation-is-local.md), [ADR 0014](../adr/0014-pbr-materials.md), [ADR 0019](../adr/0019-agent-operable.md), [the scene record](scene.md), and [the settings principles](../engine/settings-principles.md).

The public name stays **scene handle**. `CONTEXT.md` defines that term and lists "entity id" as a word to avoid. Bevy, hecs, and flecs call the same idea an entity id. The implementing commit updates the `CONTEXT.md` entry so a handle is an index plus a generation, and adds **Prefab** there. This design uses those words in that sense.

## 1. Goal

One table of placed objects, each with a scene handle that a save file, an undo step, a live MCP edit, and a net snapshot can name. A parent handle plus a local transform builds the world transform. A tick on a transform, a light, or a material emits one [change record](../lighting/GLOSSARY.md). A data file can place a prefab. Rhai reads and writes by handle, through registered functions.

## 2. Where it sits today

| Piece | State | Code |
|---|---|---|
| Scene storage | One `Floor`, `Vec<Wall>`, `Vec<Solid>`, `Vec<Light>`, plus settings structs | `Scene` in `crates/scene/src/types.rs` |
| Ids on the objects | None. `Wall`, `Solid`, and `Light` have no handle field | `types.rs` |
| Live names | A side table. `Handles::index` assigns from 1 every time a `Host` is built. The strings are `floor`, `wall:<id>`, `solid:<id>`, `light:<id>` | `crates/mcp/src/live.rs` |
| Edit path | `set_object` / `spawn_object` / `remove_object`. A geometry edit calls `Camera::attach_scene`, which rebuilds colliders. The `.rhai` file is not written | `live.rs`; [scene record](scene.md) limits |
| Change detection | `Host::with_frame` clones the whole `Scene` and compares it | `live.rs` `with_frame` |
| Draw keys | `FixedPart::Wall(usize)` and `FixedPart::Solid(usize)` are indices into those vecs. A removal shifts every later index | `World::from_scene` in `crates/render/src/world.rs` |
| Physics handles | `World::insert` returns a `usize`. `Body` has no id. The plan's `BodyId` is `{ index: u32, generation: u32 }` and is not built yet | `crates/physics/src/lib.rs`; [physics plan](physics/plan.md) P2 |
| Codimation | Keyed by the solid's vec index | `Camera::codimate` in `crates/scene/src/camera.rs` |
| Hierarchy | None. `Solid.yaw` is one angle about +Y. The picture places a wall or a square solid from `position.x`, `position.z`, and `height`, and ignores `position.y` | [scene record](scene.md) limits |
| Authoring | Rhai `floor`, `wall`, `raised_wall`, `solid`, `light`, `roof`, once, into those vecs | `load_str` in `crates/scene/src/script.rs` |
| Lighting changes | Destruction emits `ChunkChange` when `emit_change_records` is set. A headless server sets that flag false. A moved solid does not emit a change record | `crates/physics/src/destruction.rs`, `crates/physics/src/lib.rs` |
| Undo and prefabs | Not built. [Agent-driven editing](agent-driven-editing.md) names both as later work | — |

`CONTEXT.md` already says a scene handle is the stable name of one floor, wall, solid, or light, and that the floor's name is `floor`. The side table is stable only until the process rebuilds the `Host`. A reload of the same room assigns new ids.

## 3. Prior art

- Frykholm, "Building a Data-Oriented Entity System", Bitsquid, 2014. <http://bitsquid.blogspot.com/2014/08/building-data-oriented-entity-system.html>. A packed column per component, and an id that is an index plus a generation so a stale id misses after the slot is reused.
- hecs (Ralith). <https://github.com/Ralith/hecs>. Archetypes: entities with the same component set live in one table, columns are contiguous, the id is a generation plus an index. No parent link, no change tick, no prefab.
- `bevy_ecs`. <https://github.com/bevyengine/bevy/tree/main/crates/bevy_ecs>. Archetypes, a `Tick` on each component, and `Changed` / `Added` filters (<https://docs.rs/bevy_ecs/latest/bevy_ecs/change_detection/>). Bevy 0.15 added relationships, including `ChildOf`, following flecs (<https://bevyengine.org/news/bevy-0-15/>). The crate also owns the schedule.
- Mertens, "Building Games in ECS with Entity Relationships", 2020. <https://ajmmertens.medium.com/building-games-in-ecs-with-entity-relationships-5913c5a0c9d4>, and flecs (<https://github.com/SanderMertens/flecs>). A relationship is a pair. `ChildOf` is the hierarchy. `IsA` is the prefab. Query order follows the relation graph.
- Khronos glTF 2.0, nodes and hierarchy. <https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#nodes-and-hierarchy>. A node has a local transform and a parent. Skins index those nodes. The [animation design](animation.md) will sample those nodes; this design is the parent link they attach to.
- The generation in the physics plan's `BodyId` (`{ index: u32, generation: u32 }`, P2) is the same shape Jolt uses. The scene handle uses that shape so the two ids are easy to tell apart and easy to store the same way.

## 4. Options

| | bevy_ecs | hecs | flecs-style relationships | Custom archetypes in `genos-scene` |
|---|---|---|---|---|
| Storage | Archetype tables | Archetype tables | Archetype tables plus a relation index | Archetype tables |
| Id | `Entity` (index, generation) | `Entity` (index, generation) | `entity_t` | Scene handle `{ index: u32, generation: u32 }` |
| Hierarchy | `ChildOf` since 0.15 | A component the caller adds | `ChildOf` as a relationship | A `Parent` component, one link |
| Prefab | A scene and a spawn | A spawn the caller writes | `IsA` overrides | A prefab file and a `PrefabSource` component |
| Change detection | Per-component tick | None | Component hooks, query order is graph order | A `u32` tick per column, copied from the bevy filter idea |
| Who owns the world | The bevy crate, including its schedule | The hecs crate | A C runtime | `genos-scene`, which already owns `Scene` |
| Deterministic iteration | The caller's order | The caller's order | Relationship traversal order | Column order, then scene-handle order at the stage join |
| Fit | Pulls the scheduler this repo is designing separately, and would own the scene type MCP and the renderer already use | The right storage, none of the handle, parent, tick, or file behavior, and the scene type would still move into that crate | A relation engine for two links, and a C ABI under ADR 0007 | The storage stays next to `Wall` and `Solid` |

## 5. Recommendation

Keep the scene in `genos-scene`. Store objects in archetype tables. Give every placed object a scene handle. Implement exactly two links, each as a component: `Parent` and `PrefabSource`. Emit change records from column ticks, in handle order.

**Why archetypes, and why here.** A wall does not gain and drop components every frame, so the archetype move on add and remove stays rare. The columns a job walks are contiguous: every local transform at one depth, every light. That is the layout the [job system](job-system.md) reduces by index. Frykholm's packed column is the same idea without the archetype grouping; hecs and bevy show the grouping. Putting it in a new crate, or inside hecs, would split `Scene` away from the renderer and from MCP, which already take it by value.

**Why not the other three.** `bevy_ecs` would own the world and bring a schedule on top of the job system. hecs has the tables and stops there: the handle persistence, the parent, the tick, and the prefab file would be this design anyway, living outside the crate that owned the bytes. flecs relationships are the right model for a generic graph. This scene has two links. A query that walks a relation graph has an order the server hash would have to pin down. A `Parent` component iterated by depth, then by handle, has an order that is a sort.

**What we take from each.**

- From Frykholm and from physics P2: the id is an index plus a generation. A stale handle misses. The slot can be reused.
- From hecs and bevy: one table per component set, contiguous columns.
- From bevy's tick: a column stores the frame number of the last write. A reader compares ticks. The detection does not clone `Scene`.
- From flecs: the hierarchy is data (`Parent`), and a prefab instance records which file it came from (`PrefabSource`). Those are components, not a relationship type.

**Departures.**

- From flecs and from Bevy 0.15 relationships: there is no general relation, no wildcard query, and no `IsA` inheritance. An instance copies the prefab's components and then applies overrides. A later socket on a skeleton is a new component, not a new relationship kind.
- From hecs: we add the tick and the parent hecs leaves out.
- From glTF's node tree: the runtime parent is a scene handle, not a glTF node index. The loader's job is to write those handles when skins arrive. This design does not parse skins. [Loading](loading.md) still ignores them.
- `FixedPart::Wall(usize)` stays an index into the vec `World::from_scene` builds. That vec is rebuilt in handle order in the same frame as the edit, and the change record carries the handle, so a lighting key that still uses the vec index is rebuilt rather than silently pointed at a different wall. The record is what ADR 0013 consumes.

Three ids, three owners. They share a shape and they are not interchangeable.

| Id | Owner | Used for |
|---|---|---|
| Scene handle | This design | The save file, the undo stack, MCP, Rhai, the parent pointer |
| `BodyId` | Physics plan P2 | The physics world. A component on the object stores it. `insert` still returns `usize` until P2 |
| `NetId` | The core-networking plan, as the [physics plan](physics/plan.md) names it | The snapshot. Server-assigned. Storage and hash order are `NetId` order |

For an object that came from the shared scene file, the server's `NetId` is that file's scene handle, so two peers that loaded the same file agree before any spawn message. A runtime spawn gets a server `NetId` that is not a client-local index. The client stores that `NetId` on the component and applies the snapshot by it. Local vec order is not on the wire.

## 6. Handles, undo, and the files

A scene handle is `{ index: u32, generation: u32 }`.

- The floor is index 0, generation 1, and the MCP string stays `floor`.
- Every other object keeps the string it has today: `wall:<index>`, `solid:<index>`, `light:<index>`. The JSON gains `generation`. A client that omits `generation` means "the live one", which is how every current tool calls `set_object`. A recorded undo sends the generation it saw.
- A saved file stores the handle. Loading restores those handles. It does not run `Handles::index`, which renumbers from 1.
- A live spawn takes the lowest free index. If that index was used before, the generation increments.
- Undo of a delete puts the object back at its old index and generation when that index is free. When a later spawn has taken the index, the undo returns `stale` and writes nothing.
- `CONTEXT.md` still forbids the name "entity id". MCP, Rhai, the files, and the command catalogue say scene handle.

The undo stack is a ring of patches: handle, generation, field, bytes before, bytes after. `set_object` pushes one patch. Undo applies `before` and checks the generation. This replaces the full `Scene` clone in `Host::with_frame` for the edit path. A counter `scene.full_compares` stays at 0 for a one-field edit. The clone can stay for the pose publish until the tick path lands, and the harness fails if a one-field edit still clones.

The scene file `.gscene` is the saved world: one object per line, the handle, the parent handle, and the components in the `EngineSettings` text form (`to_text` / `parse`). `.rhai` stays a script that builds a scene, which is what `load_str` and `examples/camera/scene.rhai` do. An MCP edit dirties the `.gscene`. It still does not rewrite the script, which is the rule in the [scene record](scene.md).

A prefab is a `.gprefab` file in that same grammar, without the world settings (no lighting preset, no menu). `prefab(path, x, z)` in Rhai, and `spawn_prefab` on the host, copy the components onto new handles and set `PrefabSource` to the file's hash. An override is a component on the instance. Saving the instance writes the override. It does not write the prefab file. Saving the prefab is an explicit command, `save_prefab`, and it writes only when the caller names the file.

## 7. Transform hierarchy

Each object has a local translation, a quaternion, and a scale, and a world transform computed from them. `Parent` is a scene handle. The root's world transform equals its local transform. A child's world transform is the parent's world transform times the child's local transform. The multiply is the `genos-math` matrix and quaternion the renderer already uses. Scale is uniform until a mesh import needs a non-uniform scale; the column stores three floats so the glTF node can land later, and the first cut writes `(1, 1, 1)`.

The update walks depth by depth. Every node at one depth is independent, so the [job system](job-system.md) can run that depth as a parallel Must stage and reduce in handle order. A cycle is rejected at the moment `Parent` is written. The write does not apply.

Who writes the root:

- Codimation writes the local translation of that solid, then the physics step resolves it, then the resolved position is written back. That is the order `Camera::codimate` and `update` already use. The index stays a lookup until step E4, which adds `codimate_handle`.
- The physics step writes the root body it simulates. Children are not bodies unless they have their own `BodyId`. After the step, the hierarchy stage recomputes children. Those children are not a second integration.
- Animation, when it lands, writes local joint transforms under the skinned object. The joints are children. The clip player is the [animation design](animation.md). This design is the parent pointer.

`position.y` is part of the local translation. The picture's current habit of ignoring it for a wall or a square solid is a limit in the [scene record](scene.md). Step E4 updates that record and feeds the world translation, including Y, to `World::from_scene`. A root with no parent and a zero Y keeps today's picture.

## 8. Change records for lighting

A [change record](../lighting/GLOSSARY.md) is one frame's description of what moved: a mover's old and new bounds, a lamp's old and new position and reach, or the sun. ADR 0014 adds a material edit to that list. The scene emits one record when a column tick says the transform, the light, or the reflectance / color / emission changed. It emits none when the edit was `menu`, `debug`, `fps_counter`, `event_loop`, or another settings struct that does not change the lit world.

Records are sorted by scene handle before they are published, so two runs with the same edits match. The headless server keeps `emit_change_records = false`, which `crates/physics/src/lib.rs` already uses for chunk changes. The client builds the records after the hierarchy stage. Lighting consumes them under ADR 0013's caps. This design does not add a full-frame GI flush, and it does not change the GI passes.

`World::from_scene` rebuilds `FixedPart` indices from the handle order in the same frame. A removed object is a record whose new bounds are empty. The vec shift and the record land together, so a lighting key that still holds a `usize` is rebuilt from the new vec rather than left pointing at a neighbor.

## 9. Rhai

[ADR 0009](../adr/0009-rhai-is-the-scripting-language.md) stays. A script calls registered functions. It does not receive the table, a pointer, or a Vulkan handle.

- Authoring, unchanged: `floor`, `wall`, `raised_wall`, `solid`, `light`, `roof`, and the settings calls `load_str` already registers.
- New, registered in `crates/scene/src/script.rs`: `prefab(path, x, z)`, `parent(child, parent)`, `get(handle, field)`, `set(handle, field, value)`.
- `get` and `set` address a scene handle. A stale generation is an error the script can see. A missing name is an error, as every unregistered name already is.
- A per-frame script, if a later commit adds one, runs in the serial Commands stage or a serial Must stage of the [job system](job-system.md). It does not run on a worker. The Rhai engine stays `Rc<RefCell<_>>`.
- The physics rollback does not call Rhai. The [physics plan](physics/plan.md) already says so. The replay calls `step` only.

## 10. Migration

The work stays in `genos-scene`, `genos-mcp`, and `genos-render`. No new crate. Each step is one commit, keeps the camera and scene tests green, and adds the test on that row. The scene record and `CONTEXT.md` change in the step that changes the handle's meaning or the picture.

| Step | Change | Test |
|---|---|---|
| E1 | Store the handle on `Wall`, `Solid`, and `Light`. `Handles::index` reads it. A reload of a document keeps the ids. The floor string stays `floor`. An omitted `generation` still edits the live object | `reload_keeps_scene_handles`, and the existing MCP live-session test |
| E2 | Undo ring of patches. A one-field edit leaves `scene.full_compares` at 0. A stale generation returns `stale` | `undo_restores_one_field`, `stale_handle_writes_nothing` |
| E3 | Archetype table behind the vecs. `walls()`, `solids()`, and `lights()` stay. `World::from_scene` and `attach_scene` keep their signatures | `camera` room tests and `crates/scene/tests/scene_script.rs` |
| E4 | `Parent`, local and world transforms, depth walk, cycle reject. `codimate_handle` looks up the solid. The picture uses world Y. Update [scene.md](scene.md) and the scene-handle paragraph in `CONTEXT.md` | `child_follows_parent`, `a_cycle_is_rejected` |
| E5 | Ticks emit change records in handle order. Settings edits emit none. A material edit emits one (ADR 0014). The server flag stays off | `a_moved_solid_emits_one_record`, `a_menu_edit_emits_none` |
| E6 | `.gscene` and `.gprefab` round-trip. Rhai `prefab`. An override does not change the prefab file's hash | `prefab_override_does_not_write_the_file` |
| E7 | Rhai `get` / `set` / `parent`. The physics replay path does not call them | `set_handle_emits_a_change_record`, `replay_does_not_run_rhai` |
| E8 | `NetId` component. Snapshot apply by `NetId`. `debug/scene` on the shared scaffolding | `a_removal_does_not_retarget_the_survivor` |

E4's hierarchy walk is a function the job system's Must stage can call. `genos-scene` does not spawn threads. Until the pool exists the walk runs on the caller, in handle order, which is the serial reduce.

## 11. Verification harness and planted-bug mutation tests

Crate tests, one per step above, are the fast harness. They use a fixed scene, no window, and no Vulkan. `debug/scene` is the binary on the scaffolding in [Debugging](debugging.md). It loads `debug/scenes/` (the camera room and a parented prop). The picker, the checkpoint, reset, and the replay are the shared ones. Replay here is the serial list of handles and world transforms, plus the change-record list. Screenshots are for the owner's PC when E4 changes the picture; the fast check is the transform dump and the record list. `--out` writes `./genos-debug/scene/`. The process does not write under `$HOME`.

Oracles, independent of the table layout:

- A recursive walk in the test, in handle order, matches the world transforms.
- A reload of `.gscene` matches the handle, the parent, and the local transform of every object.
- Two runs emit the same change-record bytes.
- The prefab file's hash is unchanged after an instance override.

Planted bugs, `--mutate <name>`. The harness fails while the mutation is on.

| Mutation | What it breaks | The check that fails |
|---|---|---|
| `renumber_on_load` | `Handles::index` assigns from 1 again | `reload_keeps_scene_handles` |
| `index_after_remove` | `set_object` addresses the vec index after a deletion | `a_removal_does_not_retarget_the_survivor` |
| `clone_to_detect` | A one-field edit clones `Scene` again | `scene.full_compares` must stay 0 |
| `settings_are_movers` | A `menu` edit emits a change record | `a_menu_edit_emits_none` |
| `record_vec_order` | Records follow vec order instead of handle order | the two-run record compare |
| `drop_parent` | A child's world transform is its local transform | `child_follows_parent` |
| `allow_cycle` | A parent cycle is stored | `a_cycle_is_rejected` |
| `override_writes_prefab` | An instance color is written into the `.gprefab` | `prefab_override_does_not_write_the_file` |
| `rhai_in_replay` | The rollback calls `set` | `replay_does_not_run_rhai` |

## 12. Agent control and verification

- Arguments: `--scene-file`, `--prefab`, `--undo`, `--mutate`, `--check`, `--out`.
- MCP, `POST /cmd`, and Rhai share `get_scene`, `set_object`, `spawn_object`, `remove_object`, `undo`, `redo`, `spawn_prefab`, `save_prefab`, `get_scene_stats`. `set_object` gains optional `generation`. The existing string handles stay. The catalogue is `crates/debug/src/commands.rs`.
- `get_scene_stats` returns `scene.objects`, `scene.handles_alive`, `scene.stale_rejects`, `scene.cycle_rejects`, `scene.change_records`, `scene.full_compares`, `scene.prefab_writes`, `scene.hierarchy_ms`.
- The wiring check: `debug/scene --check` spawns a solid, reads `scene.objects == 1`, moves it, and reads `scene.change_records == 1`. A build that ignores `set_object` leaves both at 0 and `--check` exits non-zero.
- Undo check: `undo` after `set_object` restores the field, and `get_scene` shows the generation the patch recorded.
- An agent saves `.gscene`, reloads it, and compares handles. It does not need a GUI. The picture check, when E4 changes Y, is a screenshot from `debug/scene` on the owner's PC; the dump is the gate in CI.

The debug menu edits the same fields the commands write. [ADR 0019](../adr/0019-agent-operable.md) is the rule: the menu is a view on those commands.

## 13. CPU budget on a 1060-class machine

The [job system](job-system.md) sets the frame ceiling: Must work ≤ 6.0 ms wall on a 4-core CPU of the GTX 1060's era, Low, 1080p60. This system spends the following inside that ceiling. The numbers are ceilings for the fake-clock test and, later, for the owner's PC. They are not lavapipe times.

| Work | Ceiling |
|---|---|
| Hierarchy, 4096 nodes, serial | ≤ 0.30 ms |
| Hierarchy, same nodes, 3 workers, wall | ≤ 0.30 ms, and the reduce matches serial |
| Change records, 256 dirty objects | ≤ 0.10 ms |
| Handle lookup | One index check and one generation check |
| A one-field edit | No `Scene` clone (`scene.full_compares` stays 0) |

4096 nodes is the physics plan's scale gate, used here as a hierarchy stress. The camera room is far under it. If the hierarchy exceeds 0.30 ms, `scene.hierarchy_ms` shows it and the job stat `jobs.over_budget` counts the frame. The update still finishes. A dropped hierarchy would desync children from the body the server simulated.

## 14. Networking

`docs/systems/core-networking/plan.md` is not in the tree. The names below are the ones the [physics plan](physics/plan.md) assigns to that file. A name there wins when it lands. This crate does not open a socket.

- Replicated state: for each replicated object, the `NetId`, the quantized root pose the physics plan already lists, the parent `NetId`, and the prefab content hash. Local transforms of cosmetic children stay on the client. The scene handle is the save id; on the wire the id is `NetId`.
- Authority: the server assigns `NetId` and is the only writer of a replicated root. An owning client predicts the view body through the physics plan, not through this table.
- Prediction / rollback: the owned view body only, by replaying `step`. Hierarchy under that body recomputes after the replay and is not in the checksum. Other objects interpolate from snapshots. Rhai does not run inside the replay.
- Determinism: the same spawns, the same `.gscene` bytes, and `step` at 1/60 produce the same world transforms in handle order in one process. The parallel hierarchy reduce matches that order. Cross-machine `f32` identity is not required; the snapshot compares quantized poses.
- Headless server: no window, no Vulkan, no audio. `emit_change_records` is false. The tick uses the job system's serial mode. The server still holds the table, the handles, and the bodies.

## 15. Failure modes and probes

| Failure | Probe |
|---|---|
| A reload renumbered handles | `reload_keeps_scene_handles`; `scene.handles_alive` |
| An edit hit a reused slot | `scene.stale_rejects`, the command error `stale` |
| A parent cycle was stored | `scene.cycle_rejects` |
| A settings edit dirtied lighting | `scene.change_records` on a menu-only frame, must be 0 |
| Records differ across runs | the record bytes from `get_scene_stats` |
| Undo cloned the whole scene | `scene.full_compares` |
| An override rewrote the prefab | `scene.prefab_writes`, the file hash |
| Children did not follow a moved root | `scene.hierarchy_ms` and the transform dump |
| A removal retargeted a survivor | `a_removal_does_not_retarget_the_survivor` |

## 16. Decisions for Anthony

1. Custom archetypes in `genos-scene`, scene handle as index plus generation. bevy_ecs, hecs, and a flecs relationship graph stay out. The public word stays scene handle.
2. Two components, `Parent` and `PrefabSource`, instead of a general relationship. Prefabs are `.gprefab` data files. `.rhai` stays the room script.
3. Change records in handle order, fed to ADR 0013. A menu edit emits none. The server emits none.

## References

- Frykholm, Bitsquid, 2014. <http://bitsquid.blogspot.com/2014/08/building-data-oriented-entity-system.html>
- hecs. <https://github.com/Ralith/hecs>
- bevy_ecs, including the 0.15 relationships note. <https://github.com/bevyengine/bevy/tree/main/crates/bevy_ecs>, <https://bevyengine.org/news/bevy-0-15/>
- Mertens, 2020. <https://ajmmertens.medium.com/building-games-in-ecs-with-entity-relationships-5913c5a0c9d4>
- glTF 2.0 nodes. <https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#nodes-and-hierarchy>
