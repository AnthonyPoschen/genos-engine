# 0032: The scene becomes an archetype ECS inside genos-scene, with one Parent link

Status: **Proposed** (31 Oct 2026). Anthony reviews before any code. Design: [`docs/systems/ecs-scene.md`](../systems/ecs-scene.md). Depends on [ADR 0031](0031-job-system.md); feeds [ADR 0013](0013-lighting-invalidation-is-local.md).

## Context

`genos-scene` holds `Scene` as plain vecs of walls, solids and lights, built from `.rhai`. `render/world.rs` turns that into the GPU layout by vec index (`World::from_scene`, `FixedPart::Wall(usize)`). There is no stable id across reloads, no parent/child transform, and change detection compares whole scenes. Animation needs bone and socket parents; networking needs stable ids and a replication order; save/load needs a snapshot that round-trips; lighting needs per-object change records.

## Options

| Option | For | Against |
|---|---|---|
| bevy_ecs | Full-featured, change ticks, hierarchy | Owns the world and brings its own schedule over ADR 0031; large dependency |
| hecs | Small, archetype tables | Has no ticks, parent, persistent handle or prefab; we would build those outside the crate that owns the bytes |
| legion | Archetypes, schedules | Unmaintained; own scheduler |
| flecs-style relations | General graph | Query order over a relation graph is hard to pin for a server hash; we need two links |
| **Hand-rolled archetypes in `genos-scene`** | Fits the existing `Scene`, MCP and renderer; order is a sort; we choose the ticks | We write and maintain the storage |

## Decision

1. Keep the scene in `genos-scene`. Store objects in archetype tables with contiguous columns. `walls()`, `solids()`, `lights()`, `World::from_scene` and `attach_scene` keep their signatures through the migration.
2. Every object has a **scene handle**: index plus generation. A stale handle misses. Reloading a file keeps handles.
3. Exactly two links, as components: **`Parent`** (local and world transforms, updated depth by depth, then by handle; cycles rejected) and **`PrefabSource`** (`.gprefab` data files; instances copy, then override). No general relation type.
4. Columns store a write tick. Each tick emits **change records in handle order**; lighting (ADR 0013) consumes them instead of full compares. Menu/settings edits emit none.
5. Three ids, three owners: scene handle (save, undo, MCP, Rhai, parent), `BodyId` (physics), `NetId` (snapshot, server-assigned; equals the file handle for objects from the shared file). Snapshots apply by `NetId`, never by local vec order.
6. Save/load and the net snapshot share one codec over the tables, in `NetId` order.

## Determinism

The hierarchy pass is one ADR 0031 stage per depth, each node writing its own slot, so parallel equals serial. Change records and in-place edits are serial stages. Rhai `get`/`set` run in the Commands stage only and never inside a physics or rollback replay.

## CPU floor

On a 4-core 1060-era desktop and on the Steam Deck (4c/8t), Low: a 4096-node hierarchy update ≤ 0.30 ms wall inside the Must scope, and a one-field edit costs no full scene compare (`scene.full_compares == 0`). Measured on the owner's PC with Low caps, not lavapipe.

## Observability and debug binary

Per-handle dumps, archetype and column counts, change-record counts per frame, `scene.full_compares`, hierarchy depth and time per label, snapshot and restore, all on the command catalogue. `debug/scene` on the shared scaffolding (scene picker, reset, replay, `--check`, mutations such as reordered records and stale-handle writes).

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

E1–E3 need no job system. E4's parallel depth pass waits for ADR 0031 J3; serial first.

## Consequences

- Animation, networking and save/load get stable ids, a parent transform and an ordered snapshot from one place.
- We own the storage; no query language beyond what systems need.
- Renderer-side vec indices remain until a later change; change records carry handles so a stale index is rebuilt, never silently retargeted.

## Open questions

1. Hand-rolled archetypes over hecs: accept the cost of owning storage?
2. Only `Parent` and `PrefabSource` for now: OK that skeleton sockets later arrive as a component, not a relation?
3. Should `.gscene` (data) replace `.rhai` as the saved room format, with `.rhai` kept for scripted scenes?
4. `docs/systems/core-networking/plan.md` (PR #14) is not on master; is `NetId` = file handle the rule you want when it unparks?
