# Fracture

The fracture cook writes a `.gfract` asset. The physics step reads that asset and hot-swaps chunks. The step does not cook.

## Goal

A game can break one pre-fractured object into the chunks that were cooked for it, and both peers can name those chunks from the same file.

## Intent

The cell is a Voronoi cell: the source box clipped by perpendicular-bisector planes in increasing seed index. There is no hashmap iteration and no `atan2` on this path. Level 0 is the coarse seeds. Children sit on a fixed grid inside the parent cell, inset from 20% to 80% along the cell's longest axis. Seed order is cell order.

`health = health_per_area * area` for a face bond. A support chunk also has a bond whose other end is `WORLD` (`u32::MAX`). That bond has no face, so its health is `health_per_area * 10`. The SHA-256 of the version-1 bytes is the sim content hash. The digest is computed in this crate. There is no new cryptography dependency.

The file is little-endian, magic `GFR1`, version 1. A second cook of the same request matches the first. The step does not call `cook`.

## Code

The crate is `crates/fracture`, package `genos-fracture`.

- `cook`, `write_gfract`, `read_gfract`, and `content_hash` are in `src/lib.rs`.
- The clip and the cell volume are in `src/voronoi.rs`.
- SHA-256 is in `src/sha256.rs`.

`genos-physics` re-exports `cook`, `Asset`, and `CookRequest`. The hot-swap lives in that crate. See [physics.md](physics.md).

## Game use

Build a `CookRequest` with the box, the coarse seeds, the support flags, `children_per_chunk`, and `health_per_area`. Call `cook`. Write the bytes with `write_gfract` when the asset is a file. `read_gfract` loads them. Pass the asset to `spawn_breakable`.

## Limits

The cook is a box hull and a Voronoi pattern. It is not an impact-aligned decomposition, and it does not run at the hit. `slice_chunk` is not in this crate. Interior faces are the bond faces. Render meshes and bake keys are not in the file.

A thin cell returns `FractureError::ThinCell`. A foreign magic or version returns `BadVersion`.

| Failure | Tunable | Probe |
|---|---|---|
| The two halves are not the box | seed positions | `two_seeds_split_a_cube_into_equal_volumes` |
| Children leave the parent | `children_per_chunk` | `coarse_chunks_round_trip_and_contain_their_children` |
| The file does not round-trip | version 1 layout | the same test, plus a bad magic |
| The hash is not SHA-256 | none | `sha256_of_empty_matches_the_known_digest` |

## Networking

Gameplay networking still waits. These lines are the contract.

- Replicated state: the `.gfract` content hash when the client has the file, otherwise the seeds, child counts, interior material, support flags, and spawn pose. Then `BondBreak { breakable, bond, amount_milli, tick }` in tick order. Awake chunk poses use the same quantized body record as any dynamic body. A slice, when allowed, replicates the plane. Asleep debris and the generated vertices stay local.
- Authority: the server runs `step` and is the only side that decides a bond broke. Clients apply the server's `BondBreak` through `apply_damage`. They do not accept one another's breaks.
- Prediction / rollback: none for a breakable or a chunk. The owned view body stays the only predicted body. A client that sees a chunk pose before it has spawned that chunk applies `break_baseline` first.
- Determinism: one process, `step` on one thread, fixed `1/60` s, the same `.gfract` bytes, seed order, and bond order. `step_parallel` may run on the server only after it hash-matches `step`. The wire compares quantized poses and bond ids.
- Headless server: `step` and the sim subset. No window, no Vulkan device. The server sets `emit_change_records` false and builds no change records. The client builds them after the step. Lighting does not consume them yet.

## Decisions

- [ADR 0016](../adr/0016-physics-ccd-destruction.md) picks the pre-fractured bond graph.
- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps the cook in this repository. Blast and Chaos stay out.
