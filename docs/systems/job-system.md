# Job system (design)

Status: proposed, 27 Oct 2026. Owner: Engine. No code in this change.

This is the CPU scheduler the rest of the building blocks wait on. [Animation](animation.md) samples many skeletons on the CPU. The [physics plan](physics/plan.md) (P23, P24, P26) runs a wide solve that must hash-match the one-thread step. Save, load, and the net snapshot are ordered by the [scene ECS](ecs-scene.md), and that order is only useful if the parallel pass reduces in the same order every time. The scheduler is the piece that makes those four wide without making the server or a replay depend on which worker finished first.

Binds to [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md), [ADR 0019](../adr/0019-agent-operable.md), [the agent rubric](../agent-rubric.md), and [the settings principles](../engine/settings-principles.md).

## 1. Goal

Run CPU work for one frame on the cores that are free, join it before the frame thread submits Vulkan, and produce the same outputs in serial mode as in parallel mode. The headless server and a replay use serial mode. A 4-core, GTX 1060-class machine stays inside a 6 ms wall-time ceiling for the work that must finish before the draw.

## 2. Where it sits today

There is no pool. Each call site starts the threads it needs and joins them before it returns.

| Piece | What it does now | Code |
|---|---|---|
| Frame thread | Pumps the window, reads input, steps the scene, draws, presents | `examples/camera/src/main.rs` loop; `Renderer::draw` in `crates/render/src/gpu.rs` |
| Vulkan submit | Recorded and submitted on the frame thread | `vkQueueSubmit` via `queue_submit` in `crates/render/src/gpu.rs`, `lighting_gpu.rs`, `crates/render/src/gpu/fog.rs` |
| Seat during a device wait | A side thread answers compositor pings while the frame thread is inside a wait, a pipeline compile, or a copy larger than 1 MB | `Renderer::set_blocker` → `genos_window::seat_enter` (`crates/window/src/stall.rs`, thread `genos-seat`). `PUMP_BUDGET` is 100 ms |
| Scene load | One worker reads the file; the frame thread keeps pumping | `run_pumped`, thread `genos-scene-load` |
| Teardown | Drop a retiring scene off the frame thread | `genos-retire`, `genos-teardown` |
| MCP | Accept loop and one thread per connection. They queue work; they do not touch the scene | `genos-mcp`, `genos-mcp-conn` in `crates/mcp/src/http.rs` |
| Command queue | A client blocks. The frame thread `take`s the queue at one point in the frame and `reply`s | `crates/mcp/src/command.rs` |
| Debug script | Rhai on its own thread. Commands go through that same queue. The engine is `Rc<RefCell<_>>` | `crates/debug/src/script.rs` `spawn` (the thread is unnamed) |
| Physics step | One thread. The plan's parallel step is `std::thread::scope` per color, joined before the next color, and it must hash-equal `step` | `crates/physics/src/lib.rs` `step`; [physics plan](physics/plan.md) P22–P24 |
| Bake and the path tracer | `std::thread::scope` over rows | `crates/bake/src/sdf.rs`, `crates/debug/src/reference.rs` |
| Frame-time plot | A UI graph of past frame times. It is not a pass graph | `FrameGraph` in `examples/stress/src/main.rs` |

The camera loop's order is the one the scheduler has to keep: pump, input, scene step, draw, present. Debug commands run at the fixed point `Tools::before_draw` already uses (`crates/debug/src/tools.rs`).

## 3. Prior art

- Blumofe and Leiserson, "Scheduling Multithreaded Computations by Work Stealing", Journal of the ACM 46(5), 1999. <https://doi.org/10.1145/324133.324234>. A worker pops its own deque from one end and steals from the other end of a victim's deque.
- Chase and Lev, "Dynamic Circular Work-Stealing Deque", SPAA 2005. <https://doi.org/10.1145/1073970.1073974>. The bounded circular buffer the workers use.
- Matsakis, Rayon. <https://github.com/rayon-rs/rayon>, and "Rayon: data parallelism in Rust" (2015, <https://smallcultfollowing.com/babysteps/blog/2015/12/18/rayon-data-parallelism-in-rust/>). Fork-join on a global pool. The caller writes `join` and `par_iter`. Completion order is the steal order.
- Bevy `bevy_tasks`. <https://docs.rs/bevy_tasks>. A work-stealing pool on `async-task` (<https://github.com/smol-rs/async-task>), split into a compute pool, an async pool, and an IO pool. A waiting task is a future the pool can poll on another worker. Poll order is the steal order.
- Gyrling, "Parallelizing the Naughty Dog Engine Using Fibers", GDC 2015. <https://www.gdcvault.com/play/1022186/Parallelizing-the-Naughty-Dog-Engine>. Each worker holds many stacks. A job that waits on a counter switches fibers, so the OS thread stays busy and a high-priority job can run on that core.
- Reinalter, "Job System 2.0: Lock-Free Work Stealing", molecular matters, 2015. <https://blog.molecular-matters.com/2015/08/24/job-system-2-0-lock-free-work-stealing-part-1-basics/>. Explicit jobs, a deque per worker, a counter for the join. Part 1 does not switch stacks.
- O'Donnell, "FrameGraph: Extensible Rendering Architecture in Frostbite", GDC 2017. <https://www.gdcvault.com/play/1024612/FrameGraph-Extensible-Rendering-Architecture>. A GPU pass graph with declared reads and writes. Cited here so the CPU stage list is not mistaken for that graph.

## 4. Options

| | Rayon | bevy_tasks | Custom, fiber-free | Naughty Dog fibers |
|---|---|---|---|---|
| Where the code lives | The rayon crate owns the pool | `bevy_tasks` and `async-task` own it | `std::thread` and a deque in this repository | An OS context switch this repository would have to own (Windows fibers, or a POSIX equivalent) |
| Wait inside a job | Blocks that worker. A stage that waits on itself deadlocks when every worker is inside the wait | The worker polls another future | Rejected at submit. The wait is the stage join | Switches stacks. The worker stays busy |
| Priority | None in Rayon itself | Three pools, no frame budget | Must, Soon, Slack, and the pumped load that already exists | A high-priority job preempts by switching fibers |
| Order | Steal order | Poll order | Each task writes `slot[index]`. The frame thread reduces in index order. Serial mode runs `(stage, index)` on the frame thread | Resume order |
| Frame thread and Vulkan | The caller can size a pool and still has to keep `vkQueueSubmit` on one thread | Same | The frame thread joins the Must scope, then calls `Renderer::draw` | A yield on the frame thread gives up the core the seat pump needs |
| Server and replay | The caller sorts afterwards | The caller sorts afterwards | Serial mode is the server tick and the replay | Fiber resume is not a replay key |
| ADR 0007 | A third-party crate would own the scheduler | A third-party crate and an async runtime | The helper lives here, which is the rule when `std` is not enough | A native context-switch surface, for a wait the stage list already expresses |

## 5. Recommendation

Ship a fiber-free pool in a new crate `genos-jobs`.

Workers are long-lived `std::thread`s named `genos-job-N`, started once. Each has a Chase-Lev deque. A worker pops its own newest task and steals the oldest task from a victim, as in Chase and Lev 2005. The frame is a list of stages. A task is `(stage, index, class, label, work)`. It writes into a slot the stage allocated. The frame thread reduces that stage in index order before the next stage starts. Serial mode runs the same list on the frame thread in `(stage, index)` order and starts no worker.

The physics plan already requires this shape. P22 sorts edges before coloring so the colors are a function of the graph. P23 joins one color before the next. P24 fills pair-indexed slots and sorts by body id so the parallel manifold list matches the serial list. `step_parallel` must hash-equal `step`, and the server tick calls `step`. The pool is where that parallel step runs once both exist. Until then P23 keeps `std::thread::scope`, as that plan wrote it. This design does not edit that plan.

**Why this one.** ADR 0007 says a helper lives in this repository when the standard library has no equivalent. Rayon and `bevy_tasks` would own the scheduler. `bevy_tasks` also brings an async executor, and ADR 0007 already rejects `tokio`. Fibers solve a nested wait. The code we have does not wait inside a parallel task: a color join, a row scope, and `run_pumped` are all "run this, then continue". A stage edge is that wait. Gyrling 2015 is the design to reopen if a measured Must task blocks on another Must task and a stage cannot express it. That reopen adds a native context switch, so it is an ADR, not a silent change.

**Departures.**

- From Blumofe and Leiserson 1999 and from Chase and Lev 2005: the deque and the steal direction are theirs. The index-order reduce is ours. Their theorem is about a schedule finishing the computation, not about two schedules writing the same bytes. The hash requires the reduce.
- From Gyrling 2015: no stack switch. The stage list is the counter.
- From O'Donnell 2017: this is not a GPU frame graph. Passes stay recorded in `Renderer::draw`. The stress `FrameGraph` stays the frame-time plot.
- From Rayon: the frame thread helps with Must work, and it does not help with Slack while `genos-seat` is running.

`crates/bake` and `crates/debug` keep `std::thread::scope` until a later commit moves those row loops onto Slack. They are correct as they are.

## 6. Frame graph and the render thread

The stage list for the camera frame follows `examples/camera/src/main.rs`. Names in parentheses are the labels that show up in the stats.

1. **Pump** (frame thread). `Window::pump`. Not a job.
2. **Input** (frame thread). The action map. Not a job.
3. **Commands** (frame thread). `command::take`, run, `reply`. This is the point a script thread and an MCP client already wait on. Not a job.
4. **Must scope.** Codimation wishes, the physics step, animation sampling, then the transform hierarchy from the [scene ECS](ecs-scene.md). The frame thread joins this scope, helping run tasks, before it continues.
5. **Change records** (serial). One pass, in scene-handle order. The ECS design writes the records. The job is only the stage.
6. **Draw** (frame thread). `Renderer::draw`, then `vkQueueSubmit`. Not a job. No task calls Vulkan.
7. **Slack**, after submit and before the next pump. Suspended for the whole time `seat_enter` is active.

`Renderer::draw` stays on the thread that opened the device (`Renderer::open`). A task that needs a GPU buffer fills a `Vec` the draw copies, the same way the physics plan's `PoseBuffer` is a `Vec` the renderer can copy later.

A pumped load stays `run_pumped` in `crates/window/src/stall.rs`. The frame thread pumps inside `PUMP_BUDGET` (100 ms) while `genos-scene-load` reads the file. Moving that read onto a pool worker is a later step. The pump loop stays in the window crate either way, because a join inside `Renderer::draw` cannot answer a ping.

## 7. Script thread and MCP threads

These threads stay out of the pool.

| Thread | Why it stays dedicated |
|---|---|
| `genos-mcp`, `genos-mcp-conn` | Blocking socket IO. `crates/mcp/src/http.rs` |
| `genos-seat` | Answers `xdg_wm_base.ping` during a device wait. `crates/window/src/stall.rs` |
| `genos-scene-load` | The frame thread is in the pump loop until the load finishes |
| `genos-retire`, `genos-teardown` | Drop work that must not run on the frame thread. [Window](window.md) |
| Script thread | `crates/debug/src/script.rs` holds the Rhai engine in `Rc<RefCell<_>>`. So does `load_str` in `crates/scene/src/script.rs`. That engine cannot move to a stolen job. The implementing step names this thread `genos-script` |
| Audio callback | The device calls it. [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md). The pool does not run it |

`command::take` and `command::reply` run on the frame thread only. A `genos-job-*` thread that calls `take` gets an error. A job that calls `command::call` would deadlock: the frame thread is inside the Must scope and is not taking the queue. Jobs do not call it. The script thread and the MCP threads do, as they do now, and the Commands stage drains them.

Rhai gameplay hooks, when they exist, run in the Commands stage or in a serial Must stage. [ADR 0009](../adr/0009-rhai-is-the-scripting-language.md) is unchanged: a script calls registered functions and does not call the operating system or Vulkan. The [physics plan](physics/plan.md) already says Rhai does not run inside a rollback replay. The scheduler does not add a second place it could run.

## 8. Deterministic order for the server and replays

Two modes, one task list.

- **Serial.** The frame thread runs `(stage, index)` order. Workers are idle. This is the headless server tick, matching the physics plan's rule that the server calls `step` and not `step_parallel`. It is also the replay.
- **Parallel.** Workers run the stage. Each task writes `slot[index]` and nothing else that the hash covers. The frame thread reduces in index order, then starts the next stage. The reduce hash equals the serial hash. Completion order is not an input to the hash. Timestamps are not either.

A replay record is the serial reduce: for each stage, the index order and the hash of the slots. A second run with the same inputs, serial or parallel, matches that record. Cross-machine `f32` identity is the physics plan's rule, not a new one: a Windows server and a Linux client compare the quantized snapshot, and the job hash is an in-process check.

Stages that edit the world in place (Commands, change records) are serial even when the mode is parallel. A parallel stage is one whose slots commute under the index reduce: the physics color, the narrowphase pairs, one depth of the transform hierarchy, one animation sample range.

The fake clock used by the budget tests is not part of the hash. Wall time is not a replay input. That matches the physics plan's test rule: fixed `dt`, fixed call order, no check of thread scheduling or wall-clock time.

## 9. Priority and latency classes

Four classes. A lower class does not start while a higher class still has queued work in the same scope.

| Class | When it runs | Past the budget |
|---|---|---|
| Must | Inside the scope the frame thread joins before `Renderer::draw` | It still runs. `jobs.over_budget` increments. Dropping it would desync the server tick |
| Soon | In that same scope, after the Must tasks of the stage | It moves to the next frame and `jobs.deferred` increments |
| Slack | After `vkQueueSubmit`, before the next pump | It does not start when the Must wall time has already met the budget. `seat_enter` suspends it for the whole nesting depth |
| Pumped | The pattern `run_pumped` already implements | The frame thread keeps pumping. This class is not stolen work inside the draw |

Must work for the first cut is the codimation wish, the physics step, the animation sample the [animation design](animation.md) marks as gameplay bones, and the hierarchy update. Soon work is the far-rate animation sample (`update_rate_far_hz` in that design). Slack work is a bake row, a reference tile, and a streaming decode. Pumped work is the scene-file read.

The 100 ms `PUMP_BUDGET` in `crates/window/src/stall.rs` stays the seat rule. The job budget is a smaller ceiling inside one frame. It does not replace the stall meter.

## 10. Data and settings

`JobSettings` implements `EngineSettings` and lives on `Scene`, next to the other settings structs (`crates/scene/src/settings.rs`). The text form round-trips.

| Key | Low (1060-class CPU) | High | Meaning |
|---|---|---|---|
| `workers` | `min(3, available_parallelism - 1)` | `available_parallelism - 1` | `0` forces serial. The frame thread is the core that is not in this count |
| `order` | `parallel` on a client, `serial` on the server | same | The server forces `serial` even if the file says `parallel` |
| `frame_budget_ms` | `6.0` | `8.0` | Wall time of the Must scope. A ceiling, not a floor |
| `slack` | `on` | `on` | `off` runs no Slack |

`--jobs-workers`, `--jobs-order`, and `--jobs-budget-ms` win. `GENOS_JOBS_WORKERS`, `GENOS_JOBS_ORDER`, and `GENOS_JOBS_BUDGET_MS` apply only when the matching argument was omitted. MCP `get_job_settings` and `set_job_settings` read and write the same struct. The debug menu edits it through `set_key`.

A preset of Low on a machine with more cores still uses the Low caps, so the owner's RTX 4070 machine can run the 1060-class ceiling. Lavapipe times are not this measurement. The CPU test uses the fake clock and a fixed worker count.

## 11. Per-system millisecond stats and the 1060-class budget

Each task carries a label: `codimation`, `physics`, `anim`, `hierarchy`, `change_records`, and any later system. A worker adds the task's duration to a thread-local sum and flushes that sum at the stage join, so the published number does not depend on who stole the task.

`get_job_stats` returns, for the last completed frame:

- `jobs.must_wall_ms`, `jobs.soon_wall_ms`, `jobs.slack_wall_ms`
- `jobs.<label>_ms`, the sum of task durations for that label (this can exceed wall time when the tasks ran in parallel)
- `jobs.overhead_ms`, wall time of the scope minus the longest worker's task sum
- `jobs.steal_count`, `jobs.deferred`, `jobs.over_budget`, `jobs.slack_during_fence`, `jobs.queue_violation`, `jobs.rejected`, `jobs.panics`
- `jobs.longest_task_ms` and that task's label

The GTX 1060 is the Low GPU in [the GI design](gi-design.md). The CPU paired with that card in the same era is a 4-core desktop. Low at 1080p and 60 fps leaves most of the 16.7 ms frame for the game; the lighting Low cap is 4 ms on that GPU. The scheduler's ceilings on that CPU, for the camera room plus the animation design's 32 characters, are:

| Ceiling | Value |
|---|---|
| Scheduler overhead | ≤ 0.20 ms wall |
| Must scope, wall | ≤ 6.0 ms |
| Animation sample inside that scope | ≤ 1.0 ms, from the [animation design](animation.md) |
| Hierarchy, 4096 nodes, inside that scope | ≤ 0.30 ms wall |
| Soon | ≤ 1.0 ms, or deferred |
| Slack while Must wall ≥ 6.0 ms | 0 |
| Workers | 3 |

The physics plan's 4096 active spheres are a scale gate on the physics step, not a claim that they fit in 6 ms. If that step exceeds the budget it still finishes, and `jobs.over_budget` is the probe. High raises the Must ceiling to 8.0 ms and uses every core except the frame thread. These are ceilings to enforce with the fake clock and, later, to measure on the owner's PC. They are not lavapipe numbers.

## 12. Migration

Each step is one commit, keeps `cargo build --workspace` green, and adds the test named on that row. The crate sets `[lints] workspace = true` and is added to the workspace and to [Architecture](../architecture.md) in the first commit.

| Step | Change | Test |
|---|---|---|
| J1 | `genos-jobs`, serial scope only. `spawn(stage, index, label, class, task)` and `wait`. No thread starts | `serial_order_is_stage_then_index` |
| J2 | `JobSettings` on `Scene`, `to_text` / `parse` / `set_key` | `job_settings_round_trip` |
| J3 | The pool. Parallel slots hash-equal serial. A replay record of the serial reduce matches a second serial run and a parallel run | `parallel_slots_match_serial_hash`, `serial_replay_matches_two_runs` |
| J4 | Classes and `frame_budget_ms`, driven by an injected clock | `soon_defers_when_must_meets_the_budget`, `must_still_runs_and_counts_over_budget` |
| J5 | Camera frame: the Must scope returns before `Renderer::draw`. `command::take` errors on `genos-job-*`. The script thread is named `genos-script` | `draw_starts_after_the_must_scope`, `a_worker_cannot_take_the_command_queue` |
| J6 | `seat_enter` suspends Slack for the nesting depth in `crates/window/src/stall.rs` | `slack_does_not_run_inside_seat_enter` |
| J7 | `get_job_stats` / `set_job_settings` on the command catalogue. `debug/jobs` writes `./genos-debug/jobs/metrics.json`. `--out` overrides that directory | `label_sums_match_the_task_clock` |

J5 does not move the physics step off `step` until the physics plan's P23 lands. The camera calls the same `update` it calls today. The scope around it is empty of physics tasks until that commit, and the draw barrier is still tested with a stand-in Must task.

## 13. Verification harness and planted-bug mutation tests

The fast harness is `crates/jobs/tests/schedule.rs`. It uses a fixed worker count and the injected clock. It does not open a window and it does not start Vulkan.

`debug/jobs` is the binary on the shared scaffolding from [Debugging](debugging.md). The picker selects a workload (`empty`, `room`, `wide`) instead of a lit scene. Checkpoints are named task graphs. Reset reruns the graph from the first stage. Replay writes and reads the serial record. The metrics file is the artifact; a picture is not the proof. Sliders edit `JobSettings`. The guided-review panel lists the checks below. `--check` runs them headless. Output goes to `./genos-debug/jobs/` unless `--out` is set, and not under `$HOME`.

The oracle is the serial reduce. Parallel mode, a second process, and a replay all match that hash.

Planted bugs, each selected with `--mutate <name>`. The harness must fail while the mutation is on, and pass when it is off.

| Mutation | What it breaks | The check that fails |
|---|---|---|
| `unordered_slots` | The reduce follows completion order | `parallel_slots_match_serial_hash` |
| `server_runs_parallel` | Serial mode is ignored on the server path | `serial_replay_matches_two_runs` |
| `slack_before_must` | A Slack task can finish while a Must task is queued | the class ordering check |
| `slack_in_fence` | Slack keeps running across `seat_enter` / `seat_leave` | `slack_does_not_run_inside_seat_enter` (`jobs.slack_during_fence` must stay 0) |
| `mislabeled` | A physics task is summed under `anim` | `label_sums_match_the_task_clock` |
| `worker_takes_queue` | The `genos-job-*` guard on `command::take` is gone | `a_worker_cannot_take_the_command_queue` |
| `nested_wait` | A task waits on another task in the same stage | submit returns `rejected`, and `jobs.rejected` increments. The run ends inside the test timeout instead of hanging |

A panic inside a task increments `jobs.panics`, fails that stage's hash, and still returns the frame thread to `Window::pump`. The seat thread is joined by the existing `seat_leave` path. A panic must not be the thing that leaves a mapped window without a pump.

## 14. Agent control and verification

An agent drives the scheduler without a window.

- Arguments: `--jobs-workers`, `--jobs-order`, `--jobs-budget-ms`, `--mutate`, `--check`, `--out`.
- MCP and `POST /cmd`: `get_job_settings`, `set_job_settings`, `get_job_stats`. The same names are Rhai functions, through the catalogue in `crates/debug/src/commands.rs`.
- The check that the wiring is real: `debug/jobs --check` with a Must task whose label is `physics` returns `jobs.physics_ms > 0` and `jobs.must_wall_ms > 0`. A build where the scope never runs the task returns zeros, and `--check` exits non-zero.
- Replay: `debug/jobs --check --jobs-order serial` writes the record; a second run with `--jobs-order parallel` matches it. The agent reads `jobs.hash` from `get_job_stats`.
- The seat check is `jobs.slack_during_fence == 0` after a frame that blocked inside `seat_enter`.

The debug menu on `debug/jobs` edits the same `JobSettings` keys. It is not a second configuration.

## 15. Networking

- Replicated state: none. The scheduler has no snapshot fields. The bytes it hashes are the stage outputs the calling system already replicates, or already keeps local.
- Authority: the process that runs the tick. The server process forces `order=serial`. A client may run `parallel` when the reduce matches.
- Prediction / rollback: none inside the scheduler. The physics plan rolls the view body back by calling `step` again. Those calls are serial Must work. Rhai stays out of them.
- Determinism: in-process, the parallel reduce matches the serial reduce. The replay record is that serial reduce. Wall-clock time and steal order are not inputs.
- Headless server: no window, no Vulkan device, no audio device, `workers` may be non-zero only for a stage whose reduce matches serial, and the tick itself uses serial mode. MCP for the game session is the core-networking plan's listener, not this pool. `docs/systems/core-networking/plan.md` is not in the tree; the [physics plan](physics/plan.md) names what that file owns, and a name there wins.

## 16. Failure modes and probes

| Failure | Probe |
|---|---|
| Parallel reduce diverges from serial | `jobs.hash` against the replay record |
| Must work exceeded the ceiling | `jobs.over_budget`, `jobs.must_wall_ms`, `jobs.longest_task_ms` |
| Soon work slipped | `jobs.deferred` |
| Slack ran during a seat wait | `jobs.slack_during_fence` |
| A worker took the command queue | `jobs.queue_violation` |
| A task waited inside its own stage | `jobs.rejected` |
| A task panicked | `jobs.panics` |
| A label was charged to the wrong system | `jobs.<label>_ms` against the injected clock |

## 17. Decisions for Anthony

1. Fiber-free pool in `genos-jobs`, with serial mode as the server and the replay. Rayon, `bevy_tasks`, and Naughty Dog fibers stay out until a measured nested wait needs an ADR.
2. Must work over budget still runs and is counted. Only Soon and Slack defer.
3. Low ceiling 6.0 ms wall and 3 workers on a 4-core 1060-class CPU. High ceiling 8.0 ms.

## References

- Blumofe and Leiserson, JACM 1999. <https://doi.org/10.1145/324133.324234>
- Chase and Lev, SPAA 2005. <https://doi.org/10.1145/1073970.1073974>
- Matsakis, Rayon. <https://github.com/rayon-rs/rayon>
- Bevy tasks. <https://docs.rs/bevy_tasks>
- Gyrling, GDC 2015. <https://www.gdcvault.com/play/1022186/Parallelizing-the-Naughty-Dog-Engine>
- Reinalter, molecular matters, 2015. <https://blog.molecular-matters.com/2015/08/24/job-system-2-0-lock-free-work-stealing-part-1-basics/>
- O'Donnell, GDC 2017. <https://www.gdcvault.com/play/1024612/FrameGraph-Extensible-Rendering-Architecture>
