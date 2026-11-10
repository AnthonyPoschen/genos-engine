# 0032: The scene becomes an archetype ECS, with one Parent link

Status: **Accepted** (10 Nov 2026). Anthony approved the design in PR #131. Design: [`docs/systems/ecs-scene.md`](../systems/ecs-scene.md). Depends on [ADR 0031](0031-job-system.md); feeds [ADR 0013](0013-lighting-invalidation-is-local.md).

## Context

`genos-scene` holds `Scene` as plain vecs of walls, solids and lights, built from `.rhai`. `render/world.rs` turns that into the GPU layout by vec index (`World::from_scene`, `FixedPart::Wall(usize)`). There is no stable id across reloads, no parent/child transform, and change detection compares whole scenes. Animation needs bone and socket parents; networking needs stable ids and a replication order; save/load needs a snapshot that round-trips; lighting needs per-object change records.

## Options

| Option | For | Against |
|---|---|---|
| bevy_ecs | Full-featured, change ticks, hierarchy | Owns the world and brings its own schedule over ADR 0031; large dependency |
| hecs | Small, archetype tables | Has no ticks, parent, persistent handle or prefab; we would build those outside the crate that owns the bytes |
| legion | Archetypes, schedules | Unmaintained; own scheduler |
| flecs-style relations | General graph | Query order over a relation graph is hard to pin for a server hash; we need two links |
| **Hand-rolled archetypes** | Fits the scene, MCP and the renderer; order is a sort; we choose the ticks | We write and maintain the storage |

## Decision

1. Store objects in archetype tables with contiguous columns. `walls()`, `solids()`, `lights()`, `World::from_scene` and `attach_scene` keep their signatures through the migration. The scene crate still owns the placed objects the renderer draws.
2. Every object has a **scene handle**: index plus generation. A stale handle misses. Reloading a file keeps handles.
3. Exactly two links, as components: **`Parent`** (local and world transforms, updated depth by depth, then by handle; cycles rejected) and **`PrefabSource`** (`.gprefab` data files; instances copy, then override). No general relation type. Other components (a wall, a light, an outfit) are ordinary columns, not links.
4. Columns store a write tick. Each tick emits **change records in handle order**; lighting (ADR 0013) consumes them instead of full compares. Menu/settings edits emit none. A replication pass can read the same tick and take one component's bytes.
5. Three ids, three owners: scene handle (save, undo, MCP, Rhai, parent), `BodyId` (physics), `NetId` (snapshot, server-assigned; equals the file handle for objects from the shared file). Snapshots apply by `NetId`, never by local vec order.
6. Save/load and the net snapshot share one codec over the tables, in `NetId` order.

## Amendment (10 Nov 2026)

The design text said the tables live inside `genos-scene` and that the migration adds no crate. The tables are implemented in `genos-ecs` (`crates/ecs`).

`genos-scene` still owns the placed objects, the script, and the handles the renderer sees. Animation and a later replication layer depend on `genos-ecs` without taking Rhai, the camera, or the lighting settings. The storage rules above are unchanged: archetype columns, an index plus a generation, a write tick per cell, and handle order for the bytes a caller publishes.

This is a departure from "no new crate" in [`docs/systems/ecs-scene.md`](../systems/ecs-scene.md) §10. It is recorded here so the ADR stays the decision.

## Determinism

The hierarchy pass is one ADR 0031 stage per depth, each node writing its own slot, so parallel equals serial. Change records and in-place edits are serial stages. Rhai `get`/`set` run in the Commands stage only and never inside a physics or rollback replay.

## CPU floor

On a 4-core 1060-era desktop and on the Steam Deck (4c/8t), Low: a 4096-node hierarchy update ≤ 0.30 ms wall inside the Must scope, and a one-field edit costs no full scene compare (`scene.full_compares == 0`). Measured on the owner's PC with Low caps, not lavapipe.

## Observability and debug binary

Per-handle dumps, archetype and column counts, change-record counts per frame, `scene.full_compares`, hierarchy depth and time per label, snapshot and restore, all on the command catalogue. `debug/scene` on the shared scaffolding (scene picker, reset, replay, `--check`, mutations such as reordered records and stale-handle writes).

The store itself is checked by `debug/ecs` (`genos-ecs-debug`): `--check` spawns an object, edits one component, rejects a stale handle, and writes `./genos-debug/ecs/metrics.json`. `--out` changes that directory. `--mutate stale_accept` and `--mutate skip_tick` are planted bugs the check must fail on.

## Migration

| Stage | Change | Test |
|---|---|---|
| E1 | Handles on `Wall`, `Solid`, `Light`; reload keeps them | `reload_keeps_scene_handles` + MCP live-session test |
| E2 | Undo ring of patches; stale handle writes nothing | `undo_restores_one_field`, `stale_handle_writes_nothing` |
| E3 | Archetype tables behind the vecs, signatures unchanged | camera room tests, `scene_script.rs` |
| E4 | `Parent` and world transforms, cycle reject | `child_follows_parent`, `a_cycle_is_rejected` |
| E5 | Change records in handle order | `a_moved_solid_emits_one_record`, `a_menu_edit_emits_none` |
| E6 | `.gscene` / `.gprefab` round-trip | `prefab_override_does_not_write_the_file` |
| E7 | Rhai `get` / `set` / `parent` | `set_handle_emits_a_change_record`, `replay_does_not_run_rhai` |
| E8 | `NetId` and snapshot apply; `debug/scene` | `a_removal_does_not_retarget_the_survivor` |

E1–E3 need no job system. E4's parallel depth pass waits for ADR 0031 J3; serial first. The store crate (handles, tables, queries, ticks, component bytes) lands before E3 and is what E3 moves the vecs onto.

## Consequences

- Animation, networking and save/load get stable ids, a parent transform and an ordered snapshot from one place.
- We own the storage; no query language beyond what systems need.
- Renderer-side vec indices remain until a later change; change records carry handles so a stale index is rebuilt, never silently retargeted.

## Resolution (10 Nov 2026)

1. Hand-rolled archetypes. The cost of owning the storage is accepted. `bevy_ecs`, hecs, legion, and a flecs graph stay out.
2. `Parent` and `PrefabSource` are the only links. A later skeleton socket is a component, not a relation.
3. `.gscene` is the saved room. `.rhai` stays the script that builds a scene.
4. `NetId` equals the file's scene handle for objects that came from the shared file. A runtime spawn gets a server `NetId` that is not a client-local index. This change does not assign `NetId`; the replication ADR sits on the handle and the per-component tick.

## References

The papers and the departures are in [`docs/systems/ecs-scene.md`](../systems/ecs-scene.md). Frykholm 2014 and the physics plan's `BodyId` are the index-plus-generation shape. Bevy's per-component tick is the change record. There is no general relation type.
