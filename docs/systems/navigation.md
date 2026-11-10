# Navigation (design)

Status: proposed, 10 Nov 2026. Owner: Engine. No code in this change.

The decision is [ADR 0043](../adr/0043-navigation-planners.md). This file is the system note. The bake, the planner trait, the research, the budgets, and the stages live in the ADR. When a stage changes a goal, an intent, a game use, or a limit, that stage edits this file.

Binds to [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md), the [job system](job-system.md) and the [scene ECS](ecs-scene.md) (ADR 0031 and ADR 0032, [PR #131](https://github.com/AnthonyPoschen/genos-engine/pull/131)), ADR 0040 ([PR #176](https://github.com/AnthonyPoschen/genos-engine/pull/176)), ADR 0042 ([PR #214](https://github.com/AnthonyPoschen/genos-engine/pull/214)), [the agent rubric](../agent-rubric.md), and [the settings principles](../engine/settings-principles.md). ADR 0031, ADR 0032, ADR 0040, and ADR 0042 are proposed and are not on `master` yet.

## Goal

Compute a path for an agent on a navigation mesh or on a sparse volume graph, follow it, and steer around neighbors, inside a fixed expansion budget. The authority runtime is the only writer of that path.

## Intent

- One representation, several planners behind one trait. The default planner is A* with a funnel. Further planners arrive as later stages of ADR 0043.
- The baked mesh is integer millimetres. Two bakes of the same geometry and the same agent profile match bytes.
- The slice counts expanded nodes, in scene-handle order. A clock does not choose the corridor.
- A player runtime follows the replicated navigation intent. It does not run a planner for a remote agent.
- A learned heuristic stays refused until ADR 0043's five conditions all hold.

## Code

There is no crate. N1 of ADR 0043 adds `genos-nav` and lists it in [Architecture](../architecture.md). The crate sets `[lints] workspace = true`. It does not call Vulkan.

## Game use

No calls yet. The calls a later stage exposes are: bake a tile for an agent profile, ask a planner for a corridor, read the wish the avoidance step wrote, and read `NavIntent` for the replicator. A game sets `PlannerSelect` on the archetype. The default is `AStarFunnel`. A game does not link Recast, and it does not run its own search on the mesh.

## Limits

No bake, no search, no avoidance, no debug binary, and no replicated field. ADR 0040 still has no vehicle in `step`. A ground vehicle's wish has nowhere to land until that payload exists. Sea has no mesh. The learned planner id is specified and refused. A flying query has no planner until stage N14.

The Low ceilings, as layout and as owner's-PC targets, are in ADR 0043: 3.0 ms for one fixture tile, 0.50 ms for the query slice, 0.40 ms avoidance at 200 agents, 1.20 ms avoidance at 1000 agents, and 48 MiB resident for a 1 km² world with two profiles at the polygon cap.

## Networking

The implementing commit copies these lines if it changes them. ADR 0042 owns the socket and the snapshot codec. N9 only packs the bytes.

- Replicated state: `NavIntent` (goal in millimetres, generation, next corner in millimetres), sent when it changes, and omitted when the change tick is clear. The mesh and the planner stay on the authority.
- Authority: the authority runtime, including a listen-server, runs the planners. Clients do not accept one another's corridors.
- Prediction / rollback: a remote agent extrapolates toward the last corner for at most 3 ticks, then holds. It does not search. The pose residual corrects it.
- Determinism: serial tick, expansion quanta, integer millimetres, scene-handle order. Two runs in one process match corridor bytes. The wire compares the quantized intent.
- Headless server: the same bake and the same planners. No window and no Vulkan device.

## Decisions

- [ADR 0043](../adr/0043-navigation-planners.md). Proposed. The search recommendation, the trait, and the stages.
