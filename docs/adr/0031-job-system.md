# 0031: A fiber-free job pool with a serial mode that defines the result

Status: **Proposed** (31 Oct 2026). Anthony reviews before any code. Design: [`docs/systems/job-system.md`](../systems/job-system.md). Binds to [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md), [ADR 0019](0019-agent-operable.md), [the agent rubric](../agent-rubric.md).

## Context

There is no CPU scheduler. Each call site starts its own threads: `std::thread::scope` over rows in `crates/bake` and the path tracer in `crates/debug`, one-off `thread::spawn` for scene load, teardown, the seat pump, MCP and the Rhai script thread. Four planned layers need wide CPU work: animation sampling, asset cook, physics at scale (P22–P26), and the networking server tick. The server tick and replays also need the same bytes whichever worker finishes first. Vulkan submit stays on the frame thread.

The CPU floor is a 4-core GTX 1060-era desktop and the Steam Deck (4 Zen 2 cores, 8 threads, ADR 0023 Low).

## Options

| Option | For | Against |
|---|---|---|
| Rayon | Mature, `par_iter` is easy | Owns the pool; a nested wait blocks a worker; results come in steal order; no priority or frame budget |
| bevy_tasks | Compute/async/IO pools | Brings an async executor (ADR 0007 already rejects tokio); poll order, no frame budget |
| Fibers (Naughty Dog) | A waiting job frees the core | Needs a native context-switch surface; resume order is not a replay key; no current code waits inside a job |
| **Custom fiber-free pool** | Small; serial mode and index-order reduce built in; priority classes; owns its stats | We write and test the deque and the pool |

## Decision

1. New crate `genos-jobs`: long-lived workers `genos-job-N`, one Chase-Lev deque each, work stealing. No fibers, no async.
2. A frame is a list of **stages**. A task is `(stage, index, class, label, work)` and writes only `slot[index]`. The frame thread reduces each stage in index order before the next one starts. A wait inside a stage is rejected; the stage edge is the only join.
3. **Serial mode runs the same list on the frame thread in `(stage, index)` order. Serial is the oracle.** The headless server tick and replays always run serial. A client's parallel run must hash-equal serial. Wall time and steal order are never inputs.
4. Classes: Must (joined before `Renderer::draw`, runs even over budget and counts it), Soon (defers to next frame), Slack (after submit, suspended during `seat_enter`), Pumped (the existing `run_pumped`).
5. No task calls Vulkan or takes the MCP command queue. MCP, seat, scene-load, teardown, the Rhai script thread and the audio callback stay dedicated threads.
6. Bake and debug row loops keep `std::thread::scope` until a later commit moves them to Slack. Physics keeps its P23 scope until P23 lands.

## CPU floor

| Machine | `workers` (Low) | Must ceiling | Overhead |
|---|---|---|---|
| 4-core 1060-era desktop | 3 | 6.0 ms wall | ≤ 0.20 ms |
| Steam Deck, 4c/8t | 3 (one per physical core besides the frame thread; SMT siblings not counted) | 6.0 ms wall | ≤ 0.20 ms |
| High, any | `available_parallelism - 1` | 8.0 ms | ≤ 0.20 ms |

Inside the Must scope on Low: animation ≤ 1.0 ms, 4096-node hierarchy ≤ 0.30 ms. These are ceilings enforced with an injected clock, then measured on the owner's PC with the Low caps. Lavapipe numbers do not count.

## Observability and debug binary

Every task carries a system label. `get_job_stats` (MCP, `POST /cmd`, Rhai through `crates/debug/src/commands.rs`) returns per-label ms, Must/Soon/Slack wall, overhead, steals, deferred, over-budget, rejected, panics, the longest task and `jobs.hash`. `JobSettings` lives on `Scene` with the other settings, editable live. `debug/jobs` on the shared scaffolding runs named workloads headless with `--check`, writes `./genos-debug/jobs/metrics.json`, and has planted-bug mutations (`unordered_slots`, `server_runs_parallel`, `slack_in_fence`, `nested_wait`, …) that the checks must catch.

## Migration

One commit per stage, workspace green, one test each (names from the design):

| Stage | Change | Test |
|---|---|---|
| J1 | Crate, serial scope only | `serial_order_is_stage_then_index` |
| J2 | `JobSettings` round-trip | `job_settings_round_trip` |
| J3 | Pool; parallel equals serial | `parallel_slots_match_serial_hash`, `serial_replay_matches_two_runs` |
| J4 | Classes and budget (fake clock) | `soon_defers_when_must_meets_the_budget`, `must_still_runs_and_counts_over_budget` |
| J5 | Camera frame Must scope before draw | `draw_starts_after_the_must_scope`, `a_worker_cannot_take_the_command_queue` |
| J6 | Seat suspends Slack | `slack_does_not_run_inside_seat_enter` |
| J7 | Stats on MCP, `debug/jobs` | `label_sums_match_the_task_clock` |

## Consequences

- One scheduler and one determinism rule for physics, animation, cook and the server tick.
- We own a lock-free deque; its tests and the mutation harness are the cost.
- A job that genuinely must wait on another job inside one stage is not expressible. If that is measured, fibers come back as a new ADR.
- Cross-machine float identity is not promised here; the networking snapshot is quantised (physics plan).

## Open questions

1. Accept the Must-runs-over-budget rule (never drop Must work, only count it)?
2. Deck: 3 workers on physical cores, or use the SMT threads (up to 6)? Needs a Deck or a 4c/8t measurement.
3. Is the frame-thread-only Vulkan rule fine for now, given multi-threaded command recording would be a later render-graph ADR?
