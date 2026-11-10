# 0043: Pathfinding is a navigation mesh baked in this repository

Status: **Proposed** (10 Nov 2026). Anthony reviews before any navigation code. This file does not accept itself. `step` is unchanged.

The crowd member is a **navigator**. That word is in `CONTEXT.md`. The agent endpoint stays the loopback MCP session. The budgets below are for 200 full-rate navigators and for 1000 registered navigators.

This binds to [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md), [ADR 0016](0016-physics-ccd-destruction.md), [ADR 0019](0019-agent-operable.md), [ADR 0033](0033-physics-scale-broadphase-and-sleep.md), and [ADR 0035](0035-cooked-package-runtime-load.md). The job system and the scene store are ADR 0031 and ADR 0032, approved in [PR #131](https://github.com/AnthonyPoschen/genos-engine/pull/131) and written up in [`docs/systems/job-system.md`](../systems/job-system.md) and [`docs/systems/ecs-scene.md`](../systems/ecs-scene.md). Vehicles are ADR 0040, proposed in [PR #176](https://github.com/AnthonyPoschen/genos-engine/pull/176). Replication is ADR 0042, proposed in [PR #214](https://github.com/AnthonyPoschen/genos-engine/pull/214). Clips are ADR 0038, proposed in [PR #163](https://github.com/AnthonyPoschen/genos-engine/pull/163). ADR 0031, 0032, 0038, 0040, and 0042 are not on this branch. The system note is [`docs/systems/navigation.md`](../systems/navigation.md).

## Context

Master has no pathfinding. The view body is a capsule of radius 0.3 m and half-height 0.55 m (height 1.70 m) with `motor` set. The physics plan's step-up is 0.3 m. A mesh collider is at most six boxes or one 12-vertex hull. `Body::static_triangles` is still a plan step. Gameplay networking waits in [architecture](../architecture.md). The authority rules that do exist are ADR 0042: one writer, dedicated or listen-server, serial tick, guests extrapolate and do not resimulate remote objects.

A listen-server is a player process on the Low CPU: a 4-core desktop of the GTX 1060 era, or a Steam Deck (4 cores, 8 threads). ADR 0031 caps that Must scope at 6.0 ms wall. Navigation is CPU work. The GTX 1060 is the GPU floor and does not run the bake.

Walkers need a surface that follows collision, including a wall that breaks and a door that shuts. Ground vehicles need a wider surface. Aircraft and spacecraft do not walk. Sending every corner of every path every tick would spend the bandwidth ADR 0042 is trying to keep small.

## Options

| Option | What it covers | Why it loses |
|---|---|---|
| recastnavigation (C++) | Bake, tiles, tile cache, A*, funnel, crowd | A binding would own the mesh and the query. The region walk is that library's stack order. ADR 0007 keeps this kind of helper in-repo. Jolt was rejected on the same rule |
| [rerecast](https://github.com/janhohenheim/rerecast) 0.5.0 (26 Oct 2026) | Rust polygon bake and a detail mesh. The readme still has tile generation open | No tiles, no query, no crowd. The `libm` feature is the cross-platform float path, and the physics plan refuses `libm`. The Bevy plugin is a second scheduler |
| [recast-rs](https://recast-rs.readthedocs.io/en/latest/) (`landmark`, `waymark`, `waymark-crowd`, `waymark-tilecache`) | A Rust port of the five C++ modules, plus crowd and a tile cache | The docs say it has not been used outside WoW emulation and the API may change. It uses `glam`. It would own the mesh, the query, and the crowd, so the golden hash would follow that crate |
| **`genos-nav` in this repository** | The bake, the tiles, the query, the funnel, and the crowd, on `genos-math` | We write and test the pipeline. One code path on Linux, Windows, and macOS |

## Decision

1. New crate `crates/nav`, package `genos-nav`, `[lints] workspace = true`. No C++ binding, no `glam`, no `libm`. The debug binary is `debug/navigation`, package `genos-navigation`, bin `genos-navigation`. N0 adds both to the workspace and to [Architecture](../architecture.md).
2. A **navigation mesh** is baked per **navigation profile** from static collision. The pipeline is the Recast order: voxel heightfield, walkable filter, radius erosion, distance field, regions, contours, convex polygons of at most 6 vertices. Vertex Y is kept. There is no detail mesh.
3. The mesh is **tiled**. The authority rebuilds **one dirty tile per tick**, from kept spans when the tile is allowed to change, otherwise from the static triangles. A door flips a flag. It does not rebuild a tile.
4. **Off-mesh links** are authored jumps, ladders, and doors. Automatic jumps stay off.
5. A path query is **A\*** over polygons with integer millimetre costs, then an integer **funnel** that pulls the string. The query advances **512 expansions per tick** and resumes from a stored open set.
6. The authority follows the corridor. Full-rate navigators then solve **ORCA** in the XZ plane. Far navigators follow the string and do not solve ORCA.
7. The slice sizes are counts, in scene-handle order. Wall time never chooses who moved or which tile baked. The millisecond figures are ceilings for those counts, marked **est** until the owner's PC measures them. If a count takes longer than its ceiling, the count still completes and the counter increments.
8. The authority is the only writer. A guest receives a **72-byte path window** when it changes, and extrapolates along it. The guest does not bake, search, or steer. Navigators are not rolled back.
9. A ground vehicle uses the ground profile and stores the window on the chassis. This ADR does not write throttle, steer, or wheel forces. Aircraft and spacecraft use a **volume graph**, not a navigation mesh, and this ADR does not apply lift or thrust.
10. `NavSettings` implements `EngineSettings` and lives on `Scene`.

## Profiles

Up is +Y. Cell height `ch` is 0.10 m for every profile. A tile is 12 m on a side. Erosion is `ceil(radius / cs)` cells, which is the Recast rule and can be up to one cell past the radius.

| Profile | Who | Radius | Height | Step | Slope | `cs` | Erosion |
|---|---|---|---|---|---|---|---|
| `walker` | Kinematic capsule | 0.30 m | 1.70 m | 0.30 m | 45° | 0.20 m | 2 cells = 0.40 m |
| `ground` | ADR 0040 wheeled chassis | 1.20 m | 1.50 m | 0.20 m | 25° | 0.40 m | 3 cells = 1.20 m |

The walker numbers match the view capsule (radius 0.3 m, half-height 0.55 m) and the plan's 0.3 m step. The ground radius is the G1 chassis half-extent on X (`1.2` m). The cylinder is wider than the chassis is on Z, so a gap the box could thread stays closed on this mesh. Climb is `ceil(step / ch)` cells: 3 for a walker, 2 for ground. Walkable height is `ceil(height / ch)` cells: 17 and 15.

Aircraft and spacecraft have no row. They use the volume graph.

## Bake

Input is the static object layer only.

- A plane fills the solid half-space. The walkable face is the upper side of a floor.
- A box and a convex hull rasterize conservatively: a cell whose square meets the shape is solid across the overlapping Y spans.
- A capsule rasterizes as a sphere swept along its segment, with an integer overlap test.
- A triangle mesh uses the pieces the physics step already collides with. When `static_triangles` exists, the bake uses that list. Until then, a hole the six-box reducer closed is closed on the mesh too.
- An intact breakable contributes its intact bounds. A bond break marks the overlapping tiles dirty. Spawned chunks are obstacles on those tiles, not new static triangles.
- Dynamics, debris, the view capsule, and navigators are not bake input. A body with `MotionQuality::Swept` is not a navigation mover. A moving platform is not a walkable surface.

Spans are filtered by slope and by clearance. Regions grow in `(x, z, span)` order, and each span pushes its neighbours in that same order. Contour error and max edge length are integer millicells. Polygon vertices are quantized to tile-local `u16` millimetres. The FNV-1a hash covers those integers, in tile `(x, z)` order, then profile id. Two runs in one process match. A cross-machine golden file is not a gate.

Tiles that contain a breakable, or that an author marks dynamic, keep their compact spans. Other tiles drop the spans after the bake. A rebuild of a kept tile rasterizes those spans plus the current obstacles and then runs regions onward. The hash of that tile equals a from-scratch bake of the same triangles plus the same obstacles.

The authority bakes the tile with the lowest `(x, z, profile id)` among the dirty set, one tile per tick, and then stitches its seams to the neighbours. A quiet tick bakes nothing. The bake does not stop halfway through a tile.

A cooked `.gnav` is those same quantized bytes. ADR 0035's cook can store them later. N1–N8 hash memory. They do not add a loader.

## Off-mesh links

A link is a component on a scene handle: start polygon, end polygon, kind, and a bidirectional bit.

| Kind | Traversal |
|---|---|
| Jump | An arc with an authored height, for a fixed tick count |
| Ladder | A vertical climb at an authored speed, for a fixed tick count |
| Door | A flag. The doorway has no seam. The link is the only crossing. Closed, the link is unwalkable. Open, it is that crossing. No tile rebuild |

Traversal is a state on the navigator for that tick count. The wire event is the link id and the start tick. ADR 0038 plays the clip. Joint transforms stay local, which is ADR 0042's rule. Automatic jump generation stays off.

## Queries and the funnel

Polygon id is `(tile x, tile z, poly index)`, compared lexicographically.

The start and the goal snap to the nearest polygon in XZ within one profile height. A\* then runs:

- Cost and heuristic are the same integer hypot, the floor square root `isqrt(dx² + dz²)` on millimetres, with `i64` intermediates. The edge cost is the hypot from the entry point (the start position, or the seam midpoint) to the next entry. The heuristic is the hypot from that entry to the goal. The floor square root can sit 1 mm under the true length. The oracle meshes have a single polygon sequence, so that millimetre does not change the test.
- The heap orders by `(f, polygon id, g)`.
- An off-mesh link is an edge with an extra integer cost.
- A tie becomes the lower polygon id, so one pair of endpoints has one polygon sequence.

One waiting query advances **512 expansions** per authority tick. The open set and the closed bits are stored on the query. The oldest query goes first, and equal age breaks by scene handle. A finished query then runs the funnel. A path of 2048 expansions takes four ticks.

The funnel walks the seam edges in XZ. The left/right test is the `i64` cross product `(b − a) × (c − a)` on millimetre coordinates. Corners are vertex ids. The authority keeps the full string. The replicated window is the next 8 corners.

If the navigator's polygon is no longer on the corridor, the wish becomes zero and a new query is queued. `nav.left_corridor` increments. The navigator does not steer through a wall while the new query is open.

## Corridor, crowd, bodies

A full-rate navigator is a kinematic capsule. `motor` is set only on a tick whose wish is non-zero, so a resting navigator can sleep under the existing rule. The wish is written before `step`, in scene-handle order, the same way the view body writes a wish. The character controller is still the input map. A navigator is not that map.

The full-rate set is the navigators inside `full_rate_radius` (Low default 32 m) of a player, at most `full_rate_cap` (Low 200). Past 200, the nearest 200 win, distance first and scene handle second, and `nav.crowd_capped` increments. Everyone else is far. Far navigators have **no body** and do not collide. Their position is a component. Each tick the authority advances **160** of them, round-robin by a stored cursor, along the string at their speed. Entering the full-rate set inserts a capsule at that position. The frame of the insert can start overlapped, and `step` separates it. Leaving copies the body position back and removes the capsule. Both happen in handle order. `nav.body_swaps` counts the inserts and the removals.

Half-planes use the positions at the start of the tick. `step` then moves the capsules. The steering is one tick behind the pose. That lag is the same on every run.

ORCA runs on the full-rate set only, in XZ:

- Neighbours are the nearest `K = 10`, sorted by `(distance², scene handle)`. The spatial hash cell is twice the largest walker radius.
- Time horizon is 1500 ms. A navigator-navigator half-plane uses responsibility 1/2. A corridor edge uses responsibility 1. A ground-vehicle chassis is a circle of radius 1.20 m, also responsibility 1, taken from the vehicle list rather than the walker hash.
- The 2D linear program is the ORCA program, in that sorted neighbour order. An infeasible set uses the paper's least-penetrating relaxation.
- The output is the wish, in centimetres per second.

A ground chassis is not in the ORCA set. Walkers treat it as the circle above. The chassis already exists under ADR 0040. This decision does not add a second body for it, and it does not write throttle or steer.

## Volume graph

A flying or space navigator searches a volume graph. A node is an authored sphere. An edge exists when the segment between two nodes, expanded by the navigator's radius, misses static boxes. The test is a slab test. A\* uses the same integer hypot and the same heap order. There is no funnel and no ORCA. The result is a desired velocity toward the next node, stored on the component.

This decision does not build a 4 m lattice over a kilometre, and it does not apply aircraft lift or spacecraft thrust. The graph stage is capped at 256 nodes. A denser volume is a later ADR.

## Jobs

The authority tick is serial, which is the job-system oracle and ADR 0042's rule for the writer. Each stage writes slots and reduces in index order so a later parallel run can hash-equal serial. Labels are `nav_steer`, `nav_query`, and `nav_bake`.

On the authority all three are Must. They always run the counts below, including when the 6.0 ms ceiling is already spent. `jobs.over_budget` counts the overrun. They are not Slack: Slack is skipped when the budget is met and while `seat_enter` is active, and a skipped tile would make a replay depend on wall time.

| Stage | Count, every authority tick | Ceiling on the Low CPU (**est**) |
|---|---|---|
| `nav_steer` | 200 full-rate (follow + ORCA), 160 far follows | ≤ 0.80 ms |
| `nav_query` | 512 expansions, then the funnel if the query finished | ≤ 0.40 ms |
| `nav_bake` | one dirty tile, or nothing | ≤ 4.0 ms, and 0 ms when nothing is dirty |

Navigation inside the Must scope, bake excluded, is **≤ 1.2 ms est**. ADR 0031 already gives animation ≤ 1.0 ms and a 4096-node hierarchy ≤ 0.30 ms. Those three leave about 3.5 ms of the 6.0 ms ceiling for physics before the counter increments. A dirty tick adds one tile on top and may cross 6.0 ms. That is counted. It is one tile, not the kilometre.

A guest runs none of these stages. Extrapolation is a visual step after the snapshot apply. It is not hashed.

A pure guest's `nav_bake` stage is empty. A listen-server is the authority, so it bakes.

## Replication

ADR 0042 already sends the pose, as a packed residual, when it leaves the deadband. Navigation does not send a second pose.

The path window is 72 bytes:

```text
generation: u32     speed_cm_s: u16     count: u8     flags: u8
corners: [x_mm: i32, z_mm: i32; 8]      // 64 bytes
```

`count` is 0..=8. Unused corners are zero. Bit 0 of `flags` means the string continues past the window. Bit 1 means the next corner is an off-mesh link, and the guest holds pose interpolation across that span instead of walking the XZ segment. Y is not in the window. `speed_cm_s` tops out at 655 m/s. A faster body keeps moving by the pose residual. The first flyer fixtures sit under that cap. Wide-orbit speeds are ADR 0040's S2 and are outside this window.

The component change tick bumps when `generation` changes. A connection receives the window only for a navigator whose pose is already eligible, and only on that change. Thirty-two windows every two seconds are `32 × 72 / 2 = 1152` bytes, ceiling **1.5 KiB/s**. One hundred twenty-eight are `4608` bytes, ceiling **6 KiB/s**. The 1000 registered navigators are not all eligible. A navigator that replans every tick shows up in `nav.replan_hz`. Poses are ADR 0042's budget and are not added here.

The guest advances a visual XZ position along the window at `speed_cm_s`, between snapshots, then holds at the last corner. A correction outside the deadband interpolates to the authority pose. The guest does not resume an open set. `nav.guest_searches` stays 0.

## Memory for 1 km²

A 1000 m edge at 12 m tiles is 84 tiles on a side, **7056** tiles. The resident set is the quantized polygon mesh, not the voxels.

Bytes per polygon, **est**: a 32-byte polygon record, three 12-byte seam links, and 12 bytes of shared vertex. That is **80 bytes**. A dense town is 60% of the tiles occupied and 40 polygons on an occupied tile: `0.60 × 7056 × 40 = 169,344` polygons, `169,344 × 80 = 13,547,520` bytes, **14 MB est**. The walker ceiling is **64 MB**. The ground profile is coarser, **6 MB est**, ceiling **32 MB**. Both profiles together ceiling **96 MB**. A sparse kilometre (10% occupied, 8 polygons on a tile) is under 1 MB est, ceiling **8 MB** for both profiles. Off-mesh links at 32 bytes × 8192 sit inside those ceilings.

The voxel scratch for one tile is capped at **4 MB**. The authority holds one tile. An offline cook on three workers may hold 12 MB of scratch and is not a frame.

A bake that would pass 64 MB for the walker mesh fails the cook. The contour error is too tight. The fix is that error, not a bigger ceiling.

## Time ceilings

All **est**, Low CPU, one core. The fast tests assert the counts. The owner's PC reports the milliseconds with:

```sh
cargo test -p genos-nav --release --test crowd_bench -- --nocapture
```

Cloud-agent times are not this machine. Lavapipe times are not either.

| Work | Ceiling |
|---|---|
| One walker tile, one profile | ≤ 4.0 ms |
| 512 expansions | ≤ 0.40 ms |
| 200 full-rate, follow + ORCA | ≤ 0.80 ms |
| 160 far follows | included in the 0.80 ms |
| Navigation Must, quiet tick | ≤ 1.2 ms |
| One dirty tile on top of that | ≤ 4.0 ms extra |
| 7056 tiles, one core, offline | ≤ 30 s |

An uncapped 1000-navigator ORCA is not the Low tick. The bench prints it. The tick steers 200.

## Settings

`NavSettings` on `Scene`, text round-trip, `set_key`.

| Key | Low | Meaning |
|---|---|---|
| `full_rate_radius_m` | 32 | Who can be full-rate |
| `full_rate_cap` | 200 | ORCA and capsule cap |
| `registered_cap` | 1000 | Navigators that may exist |
| `far_per_tick` | 160 | Far follows per tick |
| `query_expansions` | 512 | A\* expansions per tick |
| `bake_tiles_per_tick` | 1 | Dirty tiles baked per tick |
| `avoidance_neighbours` | 10 | ORCA K |
| `horizon_ms` | 1500 | ORCA time horizon |
| `path_window` | 8 | Corners in the 72-byte record |

`--nav-full-rate-cap`, `--nav-query-expansions`, and `--nav-bake-tiles` win. `GENOS_NAV_FULL_RATE_CAP`, `GENOS_NAV_QUERY_EXPANSIONS`, and `GENOS_NAV_BAKE_TILES` apply only when that argument was omitted. The implementing commit registers those mirrors. This decision does not add a Rust string, so it does not edit `lint/env_mirrors.toml`.

High may raise the caps. A Low preset on a larger CPU still uses the Low caps, so the owner's machine can run the Deck ceiling.

## Stages

Each stage is one later commit, after this ADR is accepted. This proposal implements none of them. The crate stays warning-free. `dt` is `1/60` s.

| Stage | Change | Test |
|---|---|---|
| N0 | Crate, settings round-trip, empty mesh. Architecture map | `nav_settings_round_trip` |
| N1 | Heightfield, slope and clearance filters, erosion, one tile | `voxel_spans_match_the_golden_hash` |
| N2 | Regions, contours, polygons, vertex heights | `polygon_mesh_matches_the_golden_hash` |
| N3 | Tile seams, walker and ground profiles | `seams_match_across_two_tiles`, `a_wider_profile_drops_the_narrow_gap` |
| N4 | A\* on a hand-built mesh | `astar_picks_the_oracle_polygon_sequence` |
| N5 | Funnel | `funnel_corners_match_the_oracle_string` |
| N6 | Corridor follow, walker wish, ground window with no wheel write | `the_walker_reaches_the_goal_inside_the_corridor`, `a_ground_window_does_not_change_wheel_rays` |
| N7 | Jump, ladder, door | `a_closed_door_blocks_and_an_open_door_crosses` |
| N8 | One-tile rebuild equals a scratch bake of that tile | `a_dirty_tile_matches_a_scratch_bake` |
| N9 | ORCA | `two_navigators_exchange_the_oracle_velocities`, `crowd_op_count_stays_under_the_cap` |
| N10 | Authority stages, counts, serial hash | `a_sliced_query_resumes_the_same_open_set`, `serial_nav_matches_the_direct_hash` |
| N11 | 72-byte window, guest extrapolation | `a_path_window_is_72_bytes`, `path_bytes_for_32_every_two_seconds_are_1152`, `a_guest_extrapolates_and_does_not_search` |
| N12 | `debug/navigation` and the command catalogue | `nav_stats_match_the_fixture` |
| N13 | Volume graph, 256 nodes, no thrust | `a_graph_path_avoids_the_solid` |

N13 does not block N1–N12. The crowd bench is the release command above. It is not a CI gate. CI gates the operation count: 200 navigators insert at most `200 × 10` neighbour half-planes. Corridor edges are outside that count.

The golden hash is a literal in the test, FNV-1a over the quantized bytes. A bake change that moves the hash updates the literal in that commit and says why. The A\* and funnel oracles compare id sequences on a mesh the test builds by hand, so a bake change does not move them.

## Debug binary and probes

`debug/navigation` uses the shared scaffolding: scene picker and hotkey, checkpoints, reset and loop, replay of the serial mesh hash and the corner string, sliders for `NavSettings`, a guided-review panel, and a headless `--check`. Output is `./genos-debug/navigation/metrics.json` plus a top-down `plot.png`. `--out` overrides that directory. The process does not write under `$HOME`.

The plot is a software raster of the XZ mesh and the corner string. `genos-nav` does not call Vulkan. Headless `--check` does not open a window.

MCP, `POST /cmd`, and Rhai share one catalogue: `get_nav_settings`, `set_nav_settings`, `get_nav_stats`, `nav_path`, `nav_tile`, `nav_dump`.

`get_nav_stats` returns `nav.steer_ms`, `nav.query_ms`, `nav.bake_ms`, `nav.tiles_dirty`, `nav.tiles_baked`, `nav.queries_waiting`, `nav.expansions`, `nav.navigators`, `nav.full_rate`, `nav.bodies`, `nav.body_swaps`, `nav.left_corridor`, `nav.crowd_capped`, `nav.replan_hz`, `nav.path_bytes`, `nav.guest_searches`, `nav.mesh_bytes`, `nav.bake_over_budget`.

| Failure | Tunable | Probe |
|---|---|---|
| Erosion dropped | walker radius, `cs` | `voxel_spans_match_the_golden_hash` |
| Region order depends on a hash map | span order `(x, z)` | the polygon golden hash, two runs |
| A seam crack after a rebuild | one tile per tick | `a_dirty_tile_matches_a_scratch_bake` |
| The funnel crosses a corner | integer cross product | `funnel_corners_match_the_oracle_string` |
| A\* depends on heap ties | polygon id tie-break | `astar_picks_the_oracle_polygon_sequence` |
| A query runs to completion in one tick | `query_expansions` 512 | `a_sliced_query_resumes_the_same_open_set` |
| ORCA neighbour order depends on insertion | K and the handle sort | `two_navigators_exchange_the_oracle_velocities` |
| 1000 navigators all insert capsules | `full_rate_cap` 200 | `nav.bodies` ≤ 200 on `crowd_200` |
| A guest searches | empty guest stages | `nav.guest_searches == 0` |
| The window grows past 72 bytes | `path_window` 8 | `a_path_window_is_72_bytes` |
| A door rebuilds a tile | link flag | `nav.tiles_baked` stays 0 across the toggle |
| A ground path spins a wheel | no throttle write | `a_ground_window_does_not_change_wheel_rays` |
| Bake uses wall-clock skip | one tile, always | `serial_nav_matches_the_direct_hash` |

Planted bugs, `--mutate <name>`. The fixture fails while the mutation is on.

| Mutation | What it breaks |
|---|---|
| `skip_erosion` | spans ignore the radius |
| `unordered_spans` | region ids follow hash-map order |
| `funnel_cuts_a_corner` | the string takes the wrong side of a vertex |
| `rebake_all_tiles` | one obstacle rebuilds every tile |
| `guest_runs_astar` | the guest advances a query |
| `unstable_neighbours` | ORCA skips the handle sort |
| `wall_clock_slice` | the expansion cap follows the timer |

Scenes for the picker: `corridor`, `stairs`, `door`, `crowd_200`, `tile_dirty`, `vehicle_gap`, `flyer_nodes`.

## Papers and departures

- Mikko Mononen, Recast Navigation, <https://github.com/recastnavigation/recastnavigation>. The heightfield, the walkable filter, the erosion, the watershed regions, the contours, the 6-vertex polygons, the tiles, and the tile-cache spans are that pipeline's structure. Departures: neighbour pushes are sorted `(x, z)` so the region id is a function of the spans; contour error is integer millicells; there is no detail mesh; vertex Y is stored on the polygon; static tiles drop their spans; the hash is the quantized mesh.
- Hart, Nilsson, and Raphael, "A Formal Basis for the Heuristic Determination of Minimum Cost Paths", IEEE Transactions on Systems Science and Cybernetics, 1968. A\* with a consistent heuristic. Departure: `f`, `g`, and `h` are integer millimetres, and the heap tie-break is the polygon id. The slice stops at 512 expansions and resumes.
- Hershberger and Snoeyink, "Computing Minimum Length Paths of a Given Homotopy Class", Computational Geometry: Theory and Applications, 1994. The funnel. Departure: the left/right test is an `i64` cross product on millimetre coordinates. Detour's `findStraightPath` uses floats. We do not.
- van den Berg, Guy, Lin, and Manocha, "Reciprocal n-Body Collision Avoidance", Robotics Research (ISRR), 2009. The half-plane linear program, responsibility 1/2 for a pair and 1 for a static obstacle, and the infeasible relaxation. Departure: `K = 10` neighbours, not every navigator, and the pair order is `(distance², scene handle)`.
- van den Berg, Lin, and Manocha, "Reciprocal Velocity Obstacles for Real-Time Multi-Agent Navigation", ICRA 2008. DetourCrowd samples these velocity obstacles. This decision uses the ORCA linear program, because a sample set depends on sample order.

## Rejected

- Linking recastnavigation, or depending on rerecast or recast-rs. The option table says why.
- A detail mesh. Vertex heights on the 6-vertex polygon carry the step. A second triangle mesh would be more bytes and another float sample.
- Rebuilding every tile when one obstacle moves, and rebuilding a tile because a door opened.
- Treating a swept ship or a moving platform as a walkable surface.
- Generating jumps from the drop between polygons. Authored links only, until a later ADR.
- Float A\* and a float funnel. The oracle would move when `f32` addition order moves.
- Slicing work by wall time. ADR 0031 already keeps wall time out of the replay hash.
- Running A\* or ORCA on a guest. ADR 0042's guest extrapolates. It does not resimulate the crowd.
- Rolling navigators back. Owned rollback stays the player's body.
- Writing throttle, steer, lift, or thrust from this crate. ADR 0040 owns those forces. ADR 0042 already says a guest does not run the tyre solver.
- A 4 m voxel lattice over 1 km² for flyers.
- Putting the bake on Slack on the authority.
- Marking this proposal accepted in the commit that adds it.

## Five lines

- Replicated state: the 72-byte path window on a navigator whose pose is already eligible for that connection, sent when the window generation changes. The pose stays the transform column from ADR 0042. The polygon corridor, the open set, the tiles, the spans, and the crowd half-planes stay on the authority. A link traversal adds a link id and a start tick. Joint transforms stay local.
- Authority: the dedicated or listen-server runtime bakes, searches, steers, and moves navigators. Guests do not bake, search, or steer.
- Prediction / rollback: a guest extrapolates along the window between snapshots, then holds. A pose correction outside ADR 0042's deadband interpolates to the authority pose. Navigators are not rolled back. The player's body stays the owned rollback from ADR 0042.
- Determinism: the authority tick is serial. Costs, the funnel test, and the neighbour order are integers. Each tick advances 512 expansions, 200 full-rate navigators, 160 far navigators, and one dirty tile. Two runs in one process match the FNV-1a of the mesh and the corner string. The wire compares the window bytes and the quantized pose. Cross-machine `f32` identity is not required.
- Headless server: the dedicated process runs the same tick with no window, no Vulkan, and no audio. It stores the mesh and the windows. It does not draw the plot. A listen-server is the player process and is not this headless build.

## Consequences

`docs/systems/navigation.md` records that no navigation code exists. The commit that implements a stage updates the game use and the limits, and it copies these five lines if they changed. N0 adds `genos-nav` to the architecture crate map.

A quiet authority tick does no bake and no query. A dirty tick bakes one tile and can cross the 6.0 ms Must ceiling. The counter is the signal, and the tile still finishes.

Acceptance of this ADR is Anthony's. Until then the stage table is a design.

## Open questions

1. One whole tile per dirty tick is the grain. If a measured tile is past 4 ms on a Deck, the fallback is stored phases inside the tile. Is the whole tile the grain you want until that measurement exists?
2. Jumps, ladders, and doors are authored. Automatic jumps stay off. Is that the first cut?
3. A ground chassis stores a path window and does not gain throttle here. A flyer stores a desired velocity and does not gain thrust. Is that the cut against ADR 0040?
4. Flyers search an authored graph of at most 256 spheres. A local voxel bubble around one flyer would be a later ADR. Is the authored graph enough for the first space scene?
5. Guests extrapolate on the window and do not search. Navigators are never rolled back. The player's body remains the only owned rollback. Is that the split you want with ADR 0042?
