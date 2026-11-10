# Navigation (plan)

Status: proposed, 10 Nov 2026. Owner: Engine. No code in this change.

The decision is [ADR 0043](../adr/0043-navmesh-pathfinding.md). That file is the specification. This note is the system record the implementing commit extends. Where they disagree, the ADR wins and this note changes in that same commit.

Binds to [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md), [ADR 0016](../adr/0016-physics-ccd-destruction.md), [ADR 0019](../adr/0019-agent-operable.md), ADR 0031 and ADR 0032 as approved in [PR #131](https://github.com/AnthonyPoschen/genos-engine/pull/131), ADR 0040 (proposed, [PR #176](https://github.com/AnthonyPoschen/genos-engine/pull/176)), ADR 0042 (proposed, [PR #214](https://github.com/AnthonyPoschen/genos-engine/pull/214)), [the job system](job-system.md), [the scene ECS](ecs-scene.md), and [the settings principles](../engine/settings-principles.md). ADR 0031, 0032, 0040, and 0042 are not on this branch. The links above are the pull requests that hold them.

## Goal

A navigator can walk a collision scene, take a jump, a ladder, or a door, and steer around other navigators, on the authority runtime, inside a CPU budget a Steam Deck listen-server can hold.

## Intent

The bake, the query, and the crowd live in this repository. A walker and a ground vehicle use navigation meshes with different profiles. A flying or space navigator uses a volume graph. The authority tick does a fixed amount of work per tick, in scene-handle order, so a replay does not depend on wall time. A guest extrapolates along a replicated path window. It does not bake, search, or steer.

## Code

None. The implementing crate is `crates/nav`, package `genos-nav`. The debug binary is `debug/navigation`, package `genos-navigation`, bin `genos-navigation`. N0 adds both to the workspace and to [Architecture](../architecture.md).

## Game use

None yet. The calls land with the stage that adds them. A game asks for a path by writing a goal on a scene handle. It does not voxelize a tile and it does not run the crowd solver.

## Limits

The mesh is not baked. There is no query, no crowd, no off-mesh link, and no path on the wire. Aircraft and spacecraft are not steered by this plan. A ground vehicle stores a path and does not gain throttle here. Uniform gravity, up is +Y. A sphere world waits.

## Decisions

[ADR 0043](../adr/0043-navmesh-pathfinding.md).

## Networking

- Replicated state: the 72-byte path window on a navigator whose pose is already eligible for that connection, sent when the window generation changes. The pose stays the transform column from ADR 0042. The polygon corridor, the open set, the tiles, the spans, and the crowd half-planes stay on the authority. A link traversal adds a link id and a start tick. Joint transforms stay local.
- Authority: the dedicated or listen-server runtime bakes, searches, steers, and moves navigators. Guests do not bake, search, or steer.
- Prediction / rollback: a guest extrapolates along the window between snapshots, then holds. A pose correction outside ADR 0042's deadband interpolates to the authority pose. Navigators are not rolled back. The player's body stays the owned rollback from ADR 0042.
- Determinism: the authority tick is serial. Costs, the funnel test, and the neighbour order are integers. Each tick advances 512 expansions, 200 full-rate navigators, 160 far navigators, and one dirty tile. Two runs in one process match the FNV-1a of the mesh and the corner string. The wire compares the window bytes and the quantized pose. Cross-machine `f32` identity is not required.
- Headless server: the dedicated process runs the same tick with no window, no Vulkan, and no audio. It stores the mesh and the windows. It does not draw the plot. A listen-server is the player process and is not this headless build.
