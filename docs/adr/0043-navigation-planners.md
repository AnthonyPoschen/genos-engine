# 0043: Several planners share one navigation mesh, and A* with a funnel ships first

Status: **Proposed** (10 Nov 2026). Anthony Poschen reviews this before any navigation code. This file does not accept itself. No crate, no bake, and no search lands in this change.

The living note is [`docs/systems/navigation.md`](../systems/navigation.md). Words are in `CONTEXT.md`.

## Context

The runtime has no navigation. A character moves by a physics wish (`docs/systems/physics.md`). ADR 0040 (proposed, [PR #176](https://github.com/AnthonyPoschen/genos-engine/pull/176)) puts ground, aircraft, and space forces on one chassis body and leaves sea undecided. Nothing in that proposal chooses a path.

The schedule and the storage this decision uses are the [job system](../systems/job-system.md) and the [scene ECS](../systems/ecs-scene.md) ([PR #131](https://github.com/AnthonyPoschen/genos-engine/pull/131), ADR 0031 and ADR 0032): a fiber-free pool whose serial mode is the oracle, archetype columns, and a generational scene handle. The authority runtime is ADR 0042 (proposed, [PR #214](https://github.com/AnthonyPoschen/genos-engine/pull/214)): one writer, a 60 Hz tick, 20 Hz snapshot opportunities, and 192 bytes of packed fields per snapshot per connection. Those three proposals are not on `master` yet. This ADR binds to them as written there.

[ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md) keeps a helper in this repository when the standard library has no equivalent. The Low floor is the GTX 1060 / RX 580 class CPU (4 cores) and the Steam Deck (4 cores, 8 threads), the same floor as [ADR 0023](0023-graphics-settings-presets.md) and the job system. Lavapipe timings are not a budget.

Anthony asked for a search recommendation grounded in the literature, and for more than one planner behind a shared representation: an individual search, a long-range search, a shared-goal field, an incremental replanner, and a learned heuristic only where the research supports shipping one. Each archetype, or each query, picks the planner. Results stay deterministic on the authority.

## Decision

1. Navigation is a new crate `genos-nav` when the first stage lands. It does not call Vulkan and it does not own `step`. The bake reads collision geometry. The search writes a corridor and a wish. Physics consumes the wish.
2. Ground agents share a tiled navigation mesh, one mesh per agent profile. Flying and space agents share a sparse volume graph. Both are a `NavGraph`. Planners implement one trait over that graph.
3. The first planner is A* on the polygon graph with the funnel pull inside the corridor it found. Polyanya, a hierarchical search, a flow field, and D* Lite / LPA* are later implementors of the same trait. A learned heuristic has an id and stays refused until the five conditions in the research section all hold.
4. Selection is data on the archetype. The default id is `AStarFunnel`. `Auto` is an explicit opt-in and is a pure function of the query shape. It never reads a clock.
5. The authority runtime is the only process that runs a planner. A player runtime follows the replicated navigation intent and does not search for a remote agent.
6. Time slicing counts expanded nodes, in scene-handle order, from one shared budget. Wall-clock milliseconds are ceilings for the owner's PC. They are not the slice.

## Is A* still the right search?

A* (Hart, Nilsson, and Raphael, 1968, <https://doi.org/10.1109/TSSC.1968.300136>) is optimal on a graph when the heuristic is consistent, and a fixed tie-break makes the node sequence a function of the graph. On a navigation mesh the graph is the polygons and the off-mesh links. That graph is small next to a game grid: a 32 m tile capped at 256 polygons is a few hundred nodes, and a long query walks tiles rather than cells. Dijkstra (Dijkstra, 1959, <https://doi.org/10.1007/BF01386390>) is the same search with a zero heuristic and is the oracle below. A* is the right default for that graph. It is not the fastest algorithm on every representation, and it is not the Euclidean shortest path unless the edge costs are the Euclidean metric. The rest of this section is the comparison Anthony asked for. The recommendation is at the end.

### Grids, where newer classical search does beat A*

These results are real, and they are about uniform grids. A navigation mesh is an irregular polygon graph. The pruning rules do not move across.

| Method | What it guarantees | Query | Memory | Determinism | When the world changes | Low hardware |
|---|---|---|---|---|---|---|
| A* | Optimal if `h` is consistent | Expands every node the heuristic cannot prune | The open set | Yes, with a fixed tie-break and a fixed neighbor order | A fresh search sees the new edges | Fine on a few hundred polygons. Expensive on a large open grid |
| JPS, JPS+ | Optimal on a uniform-cost 8-connected grid | Harabor and Grastien (AAAI 2011, <https://ojs.aaai.org/index.php/AAAI/article/view/7994>) report an order of magnitude and more against A*, with no preprocess. JPS+ (Harabor and Grastien, ICAPS 2014, <https://ojs.aaai.org/index.php/ICAPS/article/view/13633>) adds a jump-distance table and is faster still | JPS itself is none. JPS+ stores distances | Yes | An edited cell clears the jump assumptions. JPS+ rebuilds the table around the edit | Strong on grids. The prune is false on unequal polygon edges, so it is the wrong search on this mesh |
| HPA* | Near-optimal. The paper is empirical, not a tight bound for every map | Botea, Müller, and Schaeffer, Journal of Game Development 1(1), 2004 (<https://www.movingai.com/benchmarks/papers.html>). Cluster crossings are cached, so a long path is a short graph | One distance table per cluster | Yes if cluster ids and entrance order are fixed | A local edit rebuilds the clusters it touches. That is the property this design keeps for the long-range planner | The right shape for a path that crosses many tiles |
| Contraction hierarchies | Optimal | Geisberger, Sanders, Schultes, and Delling, WEA 2008 (<https://doi.org/10.1007/978-3-540-68552-4_24>). Sturtevant's Dragon Age comparison (<https://www.cs.du.edu/~sturtevant/papers/highlevelpathfinding.pdf>) finds them comparable to a game abstraction on a static high-level graph | Shortcuts, often a small factor over the graph | Yes | A topology edit recomputes the contraction order. Customizable contraction hierarchies (Dibbelt, Strasser, and Wagner, JEA 2016, <https://doi.org/10.1145/2886843>) retune weights and still assume the topology | Preprocess is a cook. A destructible wall is a topology edit, so this is a poor fit for dirty tiles |
| Compressed path databases | Optimal, no search at runtime | Botea, AIIDE 2011 (<https://ojs.aaai.org/index.php/AIIDE/article/view/12443>). On Baldur's Gate maps up to 320×320, speedups against A* reach 700× on that set. The first move is a rectangle lookup | A 9,836-tile map was about 3.6 MB in the paper's unoptimized records. The database is all-pairs first moves, compressed. Memory grows with the map, and the paper's maps are static | Yes | The paper aims at static maps. One new wall invalidates the first-move rectangles that crossed it. Rebuilding is a Dijkstra from every node | Excellent query time, and the wrong memory and update cost for a 1 km² world that rebakes tiles |

### Any-angle on a mesh

A* on polygon centres, followed by a string pull, returns the taut path inside one corridor. A shorter path can live in a different corridor. That gap is the compromise Polyanya removes.

- The funnel (Hershberger and Snoeyink, Computational Geometry 4, 1994, <https://doi.org/10.1016/0925-7721(94)90010-8>; the channel form in games is Demyen and Buro, AAAI 2006) is linear in the corridor. It is exact for that corridor and silent about the others.
- Theta* (Nash, Daniel, Koenig, and Felner, AAAI 2007) is any-angle and not always optimal. It is a grid algorithm.
- Anya (Harabor, Grastien, Öz, and Aksakalli, JAIR 2016, <https://www.jair.org/index.php/jair/article/view/11004>) is optimal any-angle on grids, online, with no extra memory, and up to an order of magnitude faster than grid A* in that paper. It requires a grid.
- Polyanya (Cui, Harabor, and Grastien, IJCAI 2017, <https://doi.org/10.24963/ijcai.2017/70>, <https://www.ijcai.org/proceedings/2017/0070.pdf>) generalises Anya to convex polygons. It is online and Euclidean-optimal. In the paper, preprocessing methods such as TRA* can be up to two orders of magnitude faster and then cannot take a local edit; Polyanya's best mesh variant beat Anya by up to an order of magnitude in search time on the grids they converted. The successor is an interval on a polygon edge, not a polygon centre.

Polyanya is the individual planner that beats A* plus a funnel on geometric length, and it stays online when a tile is swapped. It is more code than A*. It ships second, behind the same trait.

### Crowds and replanning

- Flow fields (Emerson, "Crowd Pathfinding and Steering Using Flow Field Tiles", Game AI Pro, 2013, <https://www.gameaipro.com/GameAIPro/GameAIPro_Chapter23_Crowd_Pathfinding_and_Steering_Using_Flow_Field_Tiles.pdf>) integrate one cost field per shared goal and then look up a direction per agent. Cost follows the number of goals, not the number of agents. The field is optimal on its own grid and coarser than the polygon mesh. Emerson's sectors are 10 m. This design uses the same idea on the tiles a group actually occupies.
- Local avoidance is a separate layer. Reciprocal velocity obstacles (van den Berg, Lin, and Manocha, ICRA 2008, <https://gamma.cs.unc.edu/RVO/>) and ORCA (van den Berg, Guy, Lin, and Manocha, ISRR 2011, <https://doi.org/10.1007/978-3-642-19457-3_1>, <https://gamma.cs.unc.edu/RVO2/publications/>) solve a half-plane per neighbor. DetourCrowd is the game reference for that split: a corridor, then a velocity obstacle. ORCA does not replace the planner.
- D* Lite (Koenig and Likhachev, AAAI 2002, <https://www.aaai.org/Papers/AAAI/2002/AAAI02-088.pdf>) and LPA* (Koenig, Likhachev, and Furcy, Artificial Intelligence 155, 2004, <https://doi.org/10.1016/j.artint.2003.12.001>) reuse the previous search tree when edge costs change. The first search is a backwards A*. A small edit then touches a small set of inconsistent nodes. A tile swap that rewrites the region invalidates the tree, and the planner restarts. They earn their state when the same agent replans on a mesh that only changed a few edges.

Hierarchical navigation on meshes is HNA* (Pelechano and Fuentes, Computers & Graphics 59, 2016, pages 68–78, <https://doi.org/10.1016/j.cag.2016.05.023>). On their scenes it was up to 7.7 times faster than A* on the same mesh. It is the mesh form of HPA*: entrances between clusters, a short global search, then a local search. Inserting the start and the goal into that hierarchy can dominate the cost (Motion in Games 2017, <https://dl.acm.org/doi/10.1145/3136457.3136465>). The global tier is what `Hierarchical` implements. The ε below is a contract on the benchmark scenes.

### Learned search

These methods are the ones a reviewer will name. None of them is a drop-in replacement for A* on this mesh.

| Method | Source | What it optimises | Optimality | Where it was measured | Determinism and updates |
|---|---|---|---|---|---|
| Value iteration networks | Tamar, Wu, Thomas, Levine, and Abbeel, NeurIPS 2016, <https://arxiv.org/abs/1602.02867> | A differentiable planner embedded in a policy | No shortest-path guarantee | Small grid tasks | A forward pass follows the training stack. A new wall needs a new forward pass over the grid |
| Learned heuristic search (SAIL) | Bhardwaj, Choudhury, and Scherer, CoRL 2017, <https://arxiv.org/abs/1707.03034> | Imitates a clairvoyant expansion policy | The heuristic is not admissible, so A* with it is not optimal | Planning benchmarks, not a live navmesh | The cost model is part of the state. A dynamic tile changes the input the net never trained on |
| Neural A* | Yonetani, Taniai, Barekatain, Nishimura, and Kanezaki, ICML 2021, <https://proceedings.mlr.press/v139/yonetani21a.html> | A guidance map, then a differentiable A*, trained to match an expert path | The loss is agreement with the expert, not admissibility. A* on a learned cost map can prefer a longer path | Image and grid maps | The encoder is a convolution over a grid. Two platforms do not promise the same reductions. A rebaked tile invalidates the map |
| TransPath | Kirilenko, Andreychuk, Panov, and Yakovlev, AAAI 2023, <https://doi.org/10.1609/aaai.v37i10.26465> | A correction factor and a path-probability map, used with A* or Focal Search | Focal Search keeps a chosen suboptimality bound. Their A* variant averaged under 0.3% extra cost and up to 4× fewer expansions, on grids | Grid datasets extended from Neural A* | A transformer over a grid. The 4× is against grid A* in obstacle-rich maps, which is the regime JPS and a CPD already serve without a network |
| Diffusion planners | Janner, Du, Tenenbaum, and Levine, ICML 2022, <https://arxiv.org/abs/2205.09991>; motion-planning diffusion, Carvalho et al., IROS 2023, <https://arxiv.org/abs/2308.01557> | Sample a trajectory from a learned prior | No shortest-path guarantee. Sampling is the point | Robot manipulation and short-horizon motion, usually on a GPU | A sampler is not a pure function of the mesh unless the noise and the step count are fixed, and a fixed sampler is still a long iterative loop |

A learned heuristic would be worth enabling only when all five of these hold:

1. The heuristic is admissible, or the planner publishes an ε and the oracle test checks `cost ≤ (1+ε) × Dijkstra` on the benchmark scenes.
2. The forward pass is a pure function of the integer tile bytes and the query, with one reduction order.
3. Two serial runs in one process write the same corridor bytes.
4. On the Low CPU, a batch that fills the 0.50 ms query slice answers more queries than Polyanya and the hierarchical planner on the same fixture. The command is `debug/navigation --bench planners` on the owner's PC. A lavapipe or cloud-CPU time is not this test.
5. A tile rebake drops the cached tensors for that tile only, and the next query still meets (1)–(3).

No cited method meets (1), (3), and (4) together on a tiled navigation mesh with off-mesh links. Neural A* and TransPath are grid encoders. Diffusion planners spend their time in sampling, which is the wrong shape for a 0.50 ms slice on a Steam Deck CPU. A network that is only faster than naive grid A* has not beaten Polyanya, JPS, or a CPD on the representation those papers actually use.

**Recommendation.** Ship A* with the funnel as the default. It is optimal in the polygon-graph metric, the tie-break is a sort, and the code is the one the Dijkstra oracle can check node for node. Add Polyanya for archetypes that need the Euclidean path, the hierarchical planner past 64 m, a flow field when a group shares a goal, and D* Lite when the same agent replans on a mesh whose edges mostly survived. Leave contraction hierarchies and compressed path databases out: their speed is real, and their preprocess does not survive the dirty-tile rule. Leave learned search off. The id exists so an archetype can name it. The implementation returns unsupported until the five conditions hold, and `Auto` never selects it.

What a learned model is allowed to become later, short of those five conditions, is an offline area cost: an author, or a tool, paints a polygon weight, the bake stores that weight as an integer, and A* pays it. That is data. It is not a second search.

## Representation

The bake follows Recast's pipeline (Mononen, recastnavigation, <https://github.com/recastnavigation/recastnavigation>): rasterise the collision triangles into a heightfield, drop spans the agent cannot stand on, partition the walkable spans into regions, trace contours, and emit convex polygons. Detour's tiled mesh is the runtime shape: a polygon mesh, a detail mesh for height, and off-mesh links.

Departures from Recast, fixed so the golden hash is a function of the input:

- Region partition is the monotone scan (Recast's monotone option), in `+X` then `+Z`. Recast's default watershed is seed-ordered. The scan gives each region the id of its first span.
- Contour vertices are millimetres. The simplification error is one cell. A tie keeps the vertex with the lower `(x, z)`.
- The published mesh stores integer millimetres. Edge cost is `ceil(hypot)` in millimetres, computed with an integer square root. The heuristic is `floor(hypot)` in millimetres. `floor(hypot)` never exceeds a path of `ceil(hypot)` edges, so A* stays admissible in that metric. The visibility-graph oracle uses the same `edge_cost_mm`. An `f32` Euclidean optimum can differ by a millimetre. That difference is accepted.
- Polygon order inside a tile is `(min vertex x, min vertex z, region id)`.

A tile is 32 m on X and Z. The cell is 0.25 m. The cell height is 0.20 m. A 1 km² world is a 32×32 block of tiles (1024 m on a side, so the square kilometre sits inside it). A tile holds at most 256 polygons. A bake that would emit more fails the tile, sets `nav.poly_cap`, and leaves the previous tile in place.

The contact shape in `genos-physics` is a capsule, a box, or a mesh of at most six boxes or one 12-vertex hull (`docs/systems/physics.md`). That reduced hull is a legal bake input, and it can close a hole the source triangles kept. The bake prefers the cook's triangle soup when the cook has one. `nav.source_was_simplified` counts tiles whose only input was the reduced hull. Character bodies are not obstacles. A body marked as a blocker (a crate, a fallen chunk above the debris volume, a closed door's volume) is an obstacle.

### Agent profiles

One mesh per profile, because the radius is an erosion of the voxel. The profiles are a fixed set:

| Id | Radius | Height | Step | Max slope | Links |
|---|---|---|---|---|---|
| `walker_human` | 0.30 m | 1.80 m | 0.40 m | 45° | jump, ladder, door |
| `walker_large` | 0.60 m | 2.20 m | 0.50 m | 40° | door, and a jump only when the link names this profile |
| `ground_vehicle` | the chassis half-width plus 0.20 m | the chassis height | 0.20 m | 25° | door, when the width fits. No jump and no ladder |

Low keeps `walker_human` and `ground_vehicle` resident. `walker_large` is cooked per tile and loaded with the tiles the authority has queried. A game adds a profile by data. It does not bake per character.

### Memory for 1 km²

The ceiling is the layout, not a measurement.

A polygon record is 32 bytes (six `u16` vertex indices, six `u16` neighbours, flags, area, first link). Amortised vertices and the detail height are budgeted at 48 more bytes. 80 bytes × 256 polygons × 1024 tiles = 20,971,520 bytes, which is 20 MiB, per resident profile at the cap. Two resident profiles are 40 MiB. Runtime scratch (open sets, at most eight flow-field windows, avoidance lines) is capped at 8 MiB. The Low ceiling for a 1 km² world is **48 MiB**. An open map with 64 polygons per tile is 5 MiB per profile. `nav.resident_bytes` is the probe. Crossing 48 MiB fails the check.

A flow-field window is the group's tiles expanded by one tile, at 0.5 m cells, 8 bytes per cell. A 64 m window is 128×128×8 = 128 KB. Eight live fields are 1 MB, inside the scratch cap.

### Dynamic tiles

A blocker, a hot-swap from [ADR 0016](0016-physics-ccd-destruction.md), or a moved static dirties every tile its bounds overlap. The rebake runs on the Slack class of ADR 0031, one tile at a time, per profile that is resident. Queries keep the published tile until the new tile finishes every voxel row. The swap is one tick, in tile-id order. A query that started on the old generation is discarded and restarted, and `nav.generation_restart` counts it.

A temporary box or capsule blocker can be rasterised onto a cached heightfield of the static triangles, which is Recast's tile-cache idea. A change to the triangles themselves rebuilds that heightfield. The door flag below is neither: it does not dirty a tile.

The bake quantum is voxel rows, not milliseconds. The owner's PC ceiling, for the fixture of one 32 m tile with 40 boxes and one profile, is **3.0 ms** on one Low-class core. `debug/navigation --bench bake` is that measurement. The fake-clock test checks that a slice publishes only after the row budget is spent, and that a second bake of the same input matches the golden FNV-1a 64 of the little-endian millimetre bytes.

## Off-mesh links

A link is an edge between two polygons: a jump, a ladder, or a door. The author places it. This decision does not invent jumps from a drop.

A door is a flag on the link or on the polygon. Closed stores a cost the search treats as absent. Opened restores the baked cost. The tile bytes do not change, so the golden hash does not change. A jump or a ladder names the profiles that may use it. `ground_vehicle` ignores a jump and a ladder. The corridor reports the link. The character motor and the vehicle wish play it. This ADR does not define that motor.

## The planner trait

```rust
pub enum NavGraph<'a> {
    Ground(&'a NavMesh),
    Volume(&'a VolumeGraph),
}

pub enum SearchStep {
    Running,
    Found(Corridor),
    None,
    Unsupported,
}

pub trait Planner {
    fn id(&self) -> PlannerId;
    fn search(
        &mut self,
        graph: NavGraph<'_>,
        query: &Query,
        budget: &mut Budget,
    ) -> SearchStep;
}
```

`Budget` holds `expansions_left` for the frame. Every planner decrements it by the nodes it pops. A flow-field integration decrements it by the cells it settles. `Running` keeps the open set, keyed by `(scene handle, path generation)`. `Unsupported` means this planner has no successor function on this graph. On a navigation mesh the selector then uses `AStarFunnel`. On a volume graph it uses `GraphAstar` once that id is registered, which is stage N14. Until then a volume query stays `Unsupported` and does not run the ground funnel. `nav.planner_unsupported` increments either way.

`PlannerId` is `AStarFunnel`, `Polyanya`, `Hierarchical`, `FlowField`, `DLite`, `LpaStar`, `GraphAstar`, `Learned`.

| Id | Graph | Optimal in | When it is the one to run |
|---|---|---|---|
| `AStarFunnel` | Ground | The polygon graph, under `edge_cost_mm`. The funnel is taut inside that corridor | Default for a walker and a ground vehicle |
| `Polyanya` | Ground | `edge_cost_mm` against the vertex visibility graph | An individual whose archetype asks for the any-angle path |
| `Hierarchical` | Ground or volume | Within `hier_epsilon` (default `0.10`) of Dijkstra on the benchmark scenes | A query whose straight-line distance is at least 64 m |
| `FlowField` | Ground | Dijkstra on the 0.5 m field, which is coarser than the mesh | Eight or more agents in the same profile share a goal cell |
| `DLite` | Either | Dijkstra, after each edge edit, while the generation is unchanged | The same agent replans and most edges survived |
| `LpaStar` | Either | Dijkstra, fixed start, edited edges | An archetype names it. `Auto` does not |
| `GraphAstar` | Volume, and the ground graph if a ground planner returns `Unsupported` | The volume graph under the same integer cost | Flying and space |
| `Learned` | None until the five conditions hold | The published ε, or it does not ship | Never, until those conditions hold |

Tie-break, for every id that searches: lower `f`, then lower `g`, then lower node id. The open set is a binary heap with that key. Neighbor lists are sorted by id. No hash-map walk.

`Hierarchical` publishes `hier_epsilon`. On the benchmark scenes the test is integer: `cost_mm * 10 <= dijkstra_mm * 11` at the default. A game query that exceeds ε increments `nav.hier_over_epsilon` and the agent reruns `AStarFunnel` on the next slice. HPA*'s paper does not promise 10% on every map. The promise here is the benchmark scenes, plus the probe.

D* Lite and LPA* drop their tree when the tile generation changes. `nav.dlite_reset` counts that. A tree is capped at 4096 nodes; past the cap the agent restarts as `AStarFunnel` for that query.

## Selection

The archetype stores `PlannerSelect::{Fixed(PlannerId), Auto}`. The default is `Fixed(AStarFunnel)`. `Auto` is opt-in so a test that names a planner is not surprised by a heuristic.

`Auto` reads the domain, the straight-line distance, the number of agents in the same profile whose goal cell is the same, the replan count over the last 30 ticks, and the number of edges the last edit changed. It does not read wall time, a random source, or iteration order. Thresholds are settings: `hier_distance_m` default 64, `flow_group_min` default 8, `replan_min` default 4, `dirty_edge_max` default 32.

The order is:

1. Flying or space: `GraphAstar`. Polyanya and the flow field have no successor there.
2. `flow_group_min` agents share a goal cell: `FlowField`.
3. The agent has replanned at least `replan_min` times in 30 ticks and the edit touched at most `dirty_edge_max` edges: `DLite`.
4. Distance is at least `hier_distance_m`: `Hierarchical`.
5. The archetype sets `any_angle`: `Polyanya`.
6. `AStarFunnel`.

`Learned` is ignored by this order. A `Fixed(Learned)` while `learned_heuristic` is off falls back to step 6 and increments `nav.learned_refused`.

Agents are considered in scene-handle order. A shared goal cell is one `nav_field` task. Members look the field up only after that task publishes.

## Corridor, follow, avoidance

The corridor is the polygon sequence plus the taut points from the funnel (or the Polyanya path, which is already taut). Follow advances the agent to the next corner. A corner the agent has reached is dropped. A blocker on the current tile, or a generation change, marks the corridor stale and queues a search. Until the search finishes, the agent slows using the last corner. It does not invent a side step from a partial open set.

Avoidance is ORCA on the ground plane, after the corridor wish and before the physics step. Neighbors are the nearest `neighbor_cap` agents (default 8, floor 4), sorted by scene handle, so the linear program sees one order. The tick is `1/60` s. A flying agent does not run ORCA in the first avoidance stage. The volume graph keeps clearance, and a later stage can add a 3D half-plane. That stage is outside this decision.

ORCA can deadlock a doorway. The probe is `nav.avoid_stuck` (an agent whose wish is near zero while the corridor is non-empty for 60 ticks). The remedy in this decision is the neighbor cap and the door link, not a joint multi-agent solver. Conflict-based search and the increasing-cost tree are optimal for the joint plan and too much work for 200 agents at 60 Hz. They are not a planner id.

## Vehicles, flying, space

ADR 0040 is the force model. This ADR is the path. Neither waits on the other.

- A ground vehicle uses the `ground_vehicle` profile and `AStarFunnel` unless its archetype says otherwise. Navigation writes a throttle and a steer toward the next corner. The ground payload applies force when that payload exists. Until it exists, the wish is stored and `step` is unchanged. Jump and ladder links are refused for this profile.
- An aircraft uses the volume graph. Nodes are authored, plus a later sampler that only places nodes near those corridors. The cap is 4096 nodes per km². A dense voxel of the sky is not the graph. `GraphAstar` is the default. The wish is a desired acceleration the aircraft payload may consume. Lift and drag stay in ADR 0040.
- Space uses the same sparse graph, with a larger clearance and no slope. The wide orbit in ADR 0040's S2 is a force problem on a large position. It is not a path on this graph.
- Sea is not a domain in ADR 0040. This ADR adds no water mesh.

`nav.volume_cap` fires when a bake or an authoring pass would pass 4096 nodes.

## Jobs, the slice, and the budgets

ADR 0031's serial mode is the oracle. Each task writes `slot[index]` and nothing else the hash covers. The frame thread reduces in index order. Until `genos-jobs` exists, the caller runs that same list on one thread.

| Stage | Class | Index | Label |
|---|---|---|---|
| Field integration | Soon | field id | `nav_field` |
| Queries | Soon | scene handle | `nav_query` |
| Avoidance and the wish | Must | scene handle | `nav_avoid` |
| Tile rebake | Slack | tile id | `nav_bake` |

`nav_field` finishes, or stays unpublished, before `nav_query` reads it. A half-built field is invisible. That is the same publish rule as a finished lighting cache.

The shared query budget is `expansions_per_frame`, default 4096, spent in scene-handle order. A resumed search runs before a new one. At zero, later agents keep their corridor and `nav.deferred_queries` increments. The quantum is the expansion count. A wall-clock timeout would expand a different set on a slow core and would change the corridor. ADR 0031 already keeps wall time out of the replay hash. This slice does too.

Ceilings for the owner's PC, Low preset, one 4-core 1060-class CPU or the Steam Deck. The fake-clock tests enforce the expansion count, the neighbor cap, and the resident-byte cap. They do not assert these milliseconds. `debug/navigation --bench` prints them.

| Ceiling | 200 agents | 1000 agents |
|---|---|---|
| Query slice, Soon (`nav_field` and `nav_query` together) | 0.50 ms | 0.50 ms |
| Avoidance, Must, neighbor cap 8 | 0.40 ms | 1.20 ms |
| One tile bake, one profile, the 40-box fixture, off the tick | 3.0 ms | 3.0 ms |
| Resident navigation bytes, 1 km², two profiles at the polygon cap | 48 MiB | 48 MiB |

A steady crowd searches only when a goal, a corridor, or a tile generation changes. The 1000-agent avoidance ceiling is a large share of ADR 0031's 6.0 ms Must scope. Must still runs when the sum exceeds 6.0 ms, and `jobs.over_budget` is that probe. The neighbor cap is the knob if `nav.avoid_ms` misses 1.20 ms. Searches stay on Soon so they defer instead of stretching the physics step.

## Multiplayer

ADR 0042's five rules stay. Navigation adds one component and no second channel.

- **Replicated state:** `NavIntent` on the agent: the goal in millimetres, a `u16` generation, and the next corner in millimetres. The mesh, the open set, the flow field, the avoidance velocity, and the planner id stay on the authority. A late joiner's baseline includes the current intent while the agent has a goal, for the same reason an `AnimEvent` that is still playing is in the baseline.
- **Authority:** the authority runtime runs every planner and writes the intent. A listen-server is the authority, so it runs them. Clients do not accept one another's corridors.
- **Prediction / rollback:** a remote agent, which is every AI agent on a player runtime, keeps the last intent and moves from the replicated pose toward the corner at the last horizontal speed. That extrapolation is at most 3 ticks, then the picture holds, which is ADR 0042's remote rule. The player runtime does not call `Planner::search` for that agent. An owned pawn is not an AI agent. A click-to-move goal is a command the authority searches; the client's preview is the last intent, corrected by the pose.
- **Determinism:** serial tick, expansion quanta, integer millimetres, handle order. Two runs in one process match corridor bytes. The wire compares the quantized intent. Cross-machine `f32` identity is not required, and the mesh has no `f32` vertex.
- **Headless server:** the same bake and the same planners, no window and no Vulkan device. `debug/navigation` is that path.

`NavIntent` is omitted when its change tick is clear, so a steady agent adds zero intent bytes. A corner pass or a goal change competes inside the 192-byte snapshot budget through ADR 0042's priority accumulator. It is not a second allowance. An agent sends at most one intent per 10 ticks unless the goal changed, and a goal change still waits its turn in the accumulator. A dropped intent leaves the client on the previous corner; the pose residual corrects the picture. `nav.intent_drops` counts the drops. The pack test asserts a steady crowd adds zero intent bytes, and that one corner event is 26 bytes (three `i32` for the goal, one `u16` generation, three `i32` for the corner). That record must fit inside a snapshot that already holds the pose residuals, or it is the record that loses to a higher priority.

## Settings, debug, probes

`NavSettings` implements `EngineSettings` and lives on `Scene`. The text form round-trips. MCP commands, registered in `crates/debug/src/commands.rs` when the debug stage lands: `get_nav_settings`, `set_nav_settings`, `get_nav_stats`, `get_agent_nav`, `bake_tile`, `query_path`. The debug menu edits the same struct through `set_key`.

Arguments on `debug/navigation` win. The matching `GENOS_NAV_*` variable applies only when the argument was omitted. `--out <dir>` sets the output directory. The default is `./genos-debug/navigation/` in the working directory. The process does not write under `$HOME`.

| Key | Default | Role |
|---|---|---|
| `tile_m` | 32 | Tile edge |
| `cell_m` | 0.25 | Voxel size |
| `cell_height_m` | 0.20 | Voxel height |
| `expansions_per_frame` | 4096 | Shared slice |
| `neighbor_cap` | 8 | ORCA neighbors, floor 4 |
| `flow_group_min` | 8 | `Auto` field threshold |
| `hier_distance_m` | 64 | `Auto` hierarchical threshold |
| `hier_epsilon` | 0.10 | Benchmark bound |
| `replan_min` | 4 | Replans per 30 ticks before `DLite` |
| `dirty_edge_max` | 32 | Above this, `Auto` does not pick `DLite` |
| `learned_heuristic` | off | The flag in front of `Learned` |
| `out` | `genos-debug/navigation` | Debug output |

`debug/navigation` is the binary on the shared scaffolding in `docs/systems/debugging.md`. Headless first, the same shape as `debug/physics`: scenes in code, one JSON line, `--check`, `--replay`, `--out`. A window, when one exists, draws the mesh, the entrances, the corridor, the funnel points, the avoidance segments, the dirty tiles, and the flow arrows. Screenshots and `metrics.json` land under `--out`. The guided-review panel lists the checks in the stage table. Planted mutations the check must catch: `swapped_tie_break`, `float_vertices`, `wall_clock_slice`, `learned_on`, `client_ran_planner`, `watershed_regions`.

| Failure | Tunable | Probe |
|---|---|---|
| The bake drifted | monotone scan, millimetre vertices | `a_tile_hash_matches_the_golden`, `nav.tile_hash` |
| The source hull closed a hole | cook triangles when present | `nav.source_was_simplified` |
| A tile exceeded 256 polygons | the cap | `nav.poly_cap` |
| A search disagreed with Dijkstra | tie-break, `edge_cost_mm` | `every_planner_matches_its_oracle` |
| A slice followed the clock | `expansions_per_frame` | `a_sliced_query_resumes_and_matches_one_shot` |
| Avoidance order followed a hash map | `neighbor_cap` | `avoidance_is_handle_order` |
| A field was read half-built | publish on completion | `nav.field_published` |
| A hierarchical path left the bound | `hier_epsilon` | `nav.hier_over_epsilon` |
| A tile swap reused a D* tree | generation | `nav.dlite_reset` |
| Learned search ran | `learned_heuristic` | `nav.learned_refused` |
| A client searched | the authority flag | `client_ran_planner` |
| Intent bytes blew the snapshot | the 10-tick cap, the 192-byte budget | `nav.intent_drops`, `a_steady_intent_sends_zero_bytes` |
| Resident memory passed 48 MiB | the polygon cap, two profiles | `nav.resident_bytes` |
| The volume graph densified | the 4096-node cap | `nav.volume_cap` |
| A doorway stuck | `neighbor_cap` | `nav.avoid_stuck` |

## Stages

Each stage is one later commit, after this ADR is accepted. This change implements none of them. The harness `every_planner_matches_its_oracle` starts in N2 with the planners that exist, on one set of scenes: an open square, a concave room, a narrow gap, a door, a jump, a shared goal, and one edge whose cost changes. Adding a planner adds a row. It does not add a private oracle.

Dijkstra on the same graph and the same `edge_cost_mm` is the oracle for `AStarFunnel` (graph cost), `GraphAstar`, `DLite`, and `LpaStar`. The vertex visibility graph is the oracle for `Polyanya`. The funnel test checks tautness inside the corridor A* already chose, which is a weaker statement than Euclidean optimality. The flow field's oracle is Dijkstra on the field grid. `Hierarchical` uses the ε test. `Learned` has no oracle row while it is refused.

| Stage | Change | Test |
|---|---|---|
| N1 | One tile from boxes. Millimetre vertices. Monotone regions. Golden hash | `a_tile_hash_matches_the_golden`, `a_second_bake_matches_bits` |
| N2 | The trait. `AStarFunnel` only | `astar_cost_matches_dijkstra`, `funnel_is_taut_in_the_corridor`, `every_planner_matches_its_oracle` |
| N3 | Tiles, dirty rebuild, swap on completion | `a_dirty_tile_keeps_the_old_mesh_until_the_swap` |
| N4 | Jump, ladder, door flag | `a_closed_door_is_blocked_without_a_rebake`, `a_jump_link_is_a_corridor_edge` |
| N5 | The three profiles | `a_large_profile_drops_a_narrow_gap`, `a_ground_vehicle_refuses_a_jump_link` |
| N6 | Corner follow | `an_agent_advances_to_the_next_corner` |
| N7 | The expansion slice and the serial hash | `a_sliced_query_resumes_and_matches_one_shot`, `parallel_corridors_match_serial` |
| N8 | ORCA | `avoidance_is_handle_order`, `agents_stay_outside_the_radius` |
| N9 | `NavIntent` packing against the 192-byte rule. No socket | `a_steady_intent_sends_zero_bytes`, `a_corner_event_competes_inside_the_budget` |
| N10 | `Polyanya` | `polyanya_length_matches_the_visibility_graph` |
| N11 | `Hierarchical` | `hierarchical_cost_stays_within_epsilon` |
| N12 | `FlowField` | `a_shared_goal_field_matches_grid_dijkstra` |
| N13 | `DLite` and `LpaStar` | `dlite_matches_dijkstra_after_an_edge_change`, `lpastar_matches_dijkstra_after_an_edge_change` |
| N14 | The volume graph and the ground-vehicle wish | `a_flying_query_uses_the_sparse_graph`, `a_space_graph_rejects_a_dense_lattice` |
| N15 | `Learned` stays refused | `learned_is_refused_while_the_flag_is_off` |
| N16 | `debug/navigation`, MCP, the probe table | `get_nav_stats_names_each_probe` |

Crowd timing is a command for the owner's PC, not a CI gate: `debug/navigation --bench crowd --agents 200` and `--agents 1000`. CI runs the overlap test and the expansion budget. ADR 0031's rule holds: lavapipe time means nothing.

N7 runs the list on the caller until ADR 0031 has a pool, and the hash is still the serial hash. N9 packs bytes. ADR 0042's codec, when it exists, is the consumer. This ADR does not open a socket and does not edit that codec.

## Papers the implementing commits follow

The algorithm and the structure come from the paper. A departure not listed here needs a new line in the same commit.

- Hart, Nilsson, and Raphael, 1968, for A*. Departure: costs and the heuristic are integer millimetres, `ceil` on an edge and `floor` on the heuristic. The tie-break is node id.
- Hershberger and Snoeyink, 1994, for the funnel. Departure: predicates are `i64` cross products on millimetre vertices.
- Cui, Harabor, and Grastien, 2017, for Polyanya. Departure: the same integer cost. Interval endpoints are mesh vertices. Optimality is against the integer visibility graph.
- Botea, Müller, and Schaeffer, 2004, and Pelechano and Fuentes, 2016, for the hierarchy. Departure: ε is 0.10 on the benchmark scenes, and a miss falls back to A*. The paper does not use that fallback.
- Emerson, 2013, for the flow field. Departure: the window is the group's tiles plus one tile, at 0.5 m, and an unfinished field is not readable.
- Koenig and Likhachev, 2002, and Koenig, Likhachev, and Furcy, 2004, for D* Lite and LPA*. Departure: a tile-generation change drops the tree. The node cap is 4096.
- van den Berg, Guy, Lin, and Manocha, 2011, for ORCA. Departure: at most eight neighbors, sorted by scene handle, ground plane only, tick `1/60` s.
- Recast's voxel, filter, region, contour, and polygon steps. Departure: monotone regions, integer contours, and the simplified-hull probe.

Neural A*, TransPath, the diffusion planners, JPS, contraction hierarchies, and compressed path databases are cited in the research section so the rejection is sourced. They are not the algorithm of a stage.

## Rejected

- One planner for every agent. A shared goal and a 64 m path are different queries. The trait is the way that stays one system.
- Polyanya as the first ship. It is the better geometric search, and A* plus the funnel is the one the oracle can check before the interval search exists.
- JPS or JPS+ on the navigation mesh. The prune assumes a uniform grid.
- A contraction hierarchy or a compressed path database as the dynamic representation. The speed in those papers is on a static map. A dirty tile is a topology change.
- Recast and Detour as a linked library, and the Rust ports that wrap or rewrite them today. `rerecast` 0.5 (<https://github.com/janhohenheim/rerecast>) already builds a polygon mesh and a detail mesh, and a `libm` feature pins its `f32` ops. Tiled generation and monotone partition are still open on that list, and the vertices stay floats. `polyanya` 0.18 (<https://crates.io/crates/polyanya>) searches `glam` floats. Either crate would own the bake or the query. The integer millimetre mesh, the monotone scan, and the job-system slot hash would sit outside this repository. They are references. [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md) is the rule. Bevy's navigation plugins pull that stack in. The scene ECS design already keeps the scene out of Bevy.
- A learned planner turned on. The five conditions are unmet. A flag that defaults on would make the authority hash depend on a reduction order the papers do not fix.
- `Auto` as the archetype default. A fixed id keeps a test on the planner it named.
- Slicing on wall-clock time. The corridor would follow the core.
- Replicating the mesh, a waypoint strip, or a flow field. ADR 0042's 192 bytes do not hold that, and a remote picture can follow one corner.
- A client-side planner that "corrects" the authority. The authority is the only writer.
- Baking from the 6-box hull when the cook still has the triangles, without a probe. The hull is allowed only as a fallback, and `nav.source_was_simplified` counts it.
- A dense voxel for the sky, or a water mesh while sea is undecided.
- Joint multi-agent search (conflict-based search, increasing-cost trees) as the crowd layer.
- Marking this proposal accepted in the commit that adds it.

## Consequences

`docs/systems/navigation.md` records the goal and the limits. The commit that implements a stage updates the code section, the game use, and the five networking lines if a line changed. `genos-nav` joins the workspace and `docs/architecture.md` in N1, with `[lints] workspace = true`.

`step` stays as it is. ADR 0040's empty vehicle list stays empty. ADR 0042's snapshot budget stays 192 bytes. A navigation intent that does not fit is dropped, not a reason to raise the budget.

Acceptance of this ADR is Anthony's. Until then the stage table is a design, and no crate carries a planner.
