# Agent rubric

Every coding agent and every planning agent follows this file. A reviewer scores the same checklist. Copy the table at the end into the pull request and fill **Score** and **Evidence**.

Score each row `pass`, `fail`, or `n/a`. Evidence is a path, a command, a number, or a short quote. An `n/a` includes the reason. A `fail` on a row that applies means the change is not ready. The author's score is the claim; the reviewer marks the row again.

These records stay in force. This rubric points at them:

- Words and decisions: `docs/agents/domain.md`, `CONTEXT.md`, `docs/adr/`.
- Architecture, native calls, scripting: `docs/architecture.md`, `docs/adr/0007-native-calls-go-to-the-system-and-vulkan.md`, `docs/adr/0009-rhai-is-the-scripting-language.md`.
- Lighting: `docs/systems/lighting/README.md`, `docs/adr/0012-gi-v2.md`. GI v1 stays removed.
- Math and camera: `docs/references/breaking-point.md`.
- Systems: `docs/systems/README.md` and the matching record, edited in the same change when the goal, the intent, the game use, or a limit changes.
- Rust: `cargo build --workspace` and the tests for the crates you touched. Warnings are denied. A new crate sets `[lints] workspace = true`.
- Input behavior: `../input-zig`. The file list is in `docs/architecture.md`.
- First scene: `docs/goals/mvp-scene.md`.
- Debug and proof: `docs/systems/debugging.md`. Every engine system has a binary under `debug/<system>/`. Gameplay networking waits (`docs/architecture.md`). Fill in the networking row anyway.

## Checklist

1. **Debug binary.** Every engine system ships a dedicated debug binary under `debug/<system>/` (`debug/lighting`, `debug/physics`, `debug/networking`, `debug/ui`, `debug/assets`, and the same folder for any other system). A system is done when that binary covers its features. A change that alters a system's behavior extends that coverage in the same change.

   The binaries share one scaffolding, extended from `crates/debug` (`genos-debug`) and the MCP command queue in `docs/systems/debugging.md`. `genos-stress` is the lighting coverage that moves into `debug/lighting`. Scenes live once, as data files in `debug/scenes/`, loaded by every binary. The files already in `examples/stress/scenes/` and `examples/camera/scene.rhai` move into that library.

   The scaffolding provides all of these:

   - A scene picker at launch, and a hotkey that switches scenes.
   - Preset test locations and checkpoints.
   - One-key reset, and loop back to start.
   - Deterministic replay.
   - Screenshot and metrics output.
   - Live tuning sliders.
   - A guided-review panel that tells a person what to look at and what to test.
   - A headless drive for agents, over the CLI or MCP.

   Lighting is the reference standard. Other systems match the inventory in `docs/systems/lighting/debug-standard.md`.

2. **Observability.** Every part of a system that can fail, or that needs tuning, exposes its state for inspection. Agents query that state through an API, the CLI, or MCP. A person reads it from an overlay or an inspector. Add the probe to the command catalogue in `crates/debug/src/commands.rs`, so MCP, `POST /cmd`, and Rhai share one JSON form. The overlay path is `Tools::overlay`, `profile_overlay`, and `inspect_region`.

   Each system exposes:

   - Per-entity dumps and per-subsystem dumps.
   - Counters and timings.
   - Visualisations: heatmaps and debug draws.
   - Tunables editable live.
   - A state snapshot and a restore, so a failure can be reproduced.

   Every new feature lists its failure modes and its tunables, and names the probe that exposes each one. A feature with no probe fails the review. The probes live on that system's `debug/<system>` binary (row 1).

   The worked example is the lighting inventory in `docs/systems/lighting/debug-standard.md`: the GI dump, the MCP command catalogue, the stuck and pop checks, the headless `genos-stress` path, the stats counters, and the overlays.

3. **Networking.** A new component or system states, in its system record, these five lines. Local-only work still answers each one. Gameplay networking still waits; the lines are the contract for a later server. The loopback agent endpoint stays the live-scene path.

   - Replicated state:
   - Authority:
   - Prediction / rollback:
   - Determinism:
   - Headless server:

4. **Papers.** The implementation follows the cited paper's algorithm and structure. Every departure is explicit in the pull request: the paper, the section or equation, and what the code does. Lighting departures also land in the deviations list in `docs/systems/lighting/README.md`, in the same change. An ADR conflict is named.

5. **Commits.** One change per commit. Push with `git push` so the remote fast-forwards. A force push, `--force-with-lease`, or a rewrite of a pushed branch fails this row.

6. **Fast test.** Each change has a fast deterministic test. GPU work runs under lavapipe (software Vulkan) on the headless path in `docs/systems/debugging.md`. Real performance and visual judgment happen on the owner's PC (zanven-pc with the RTX 4070, and the MacBook). When the result is visual, that system's `debug/<system>` binary writes the screenshots and metrics. The session adds that command; the owner's PC runs it.

7. **Inventory.** Read the crates, the system record, and, for lighting, `docs/systems/lighting/experiments/` before adding code. Build on the piece that already works. A new crate, shader, or path says why the existing one cannot carry the change.

8. **Performance.** Work is gated by need. A budget is a ceiling (E19 in the lighting experiments). A cheap check verifies the frame; "nothing changed" is a result of that check (`docs/systems/lighting/README.md`, S74A). The frame, the desktop, and a live cache stay free of a hard freeze. A steady scene's frame time approaches that scene's non-GI floor.

9. **GI quality.** Refinement is even inside a region. That region shows no pop and no patchwork. Quality may change at a logical boundary (a mesh, a room, a layer). A fill that finishes in the same frame is the allowance already in the lighting record, and the region is still even.

10. **Report.** The pull request or the experiment note carries numbers: what was measured, where, and the before and after. A visual change carries the screenshots the script wrote, and they are looked at. A rollback or a false pass has five lines:

   1. Claim:
   2. Check:
   3. Fault:
   4. First seen:
   5. Left reverted:

11. **Small tasks.** One concern, small enough to review. A long build, a full `gi-check`, or a path trace is a command left for the owner's PC. Run the fast test from row 6, then continue. The Rust gate in `AGENTS.md` still applies to the crates you touched.

12. **Scored checklist.** The pull request includes the table below, filled in. This row is `pass` only when every other row has a score and evidence.

## Pull request table

| # | Item | Score | Evidence |
|---|---|---|---|
| 1 | Debug binary under `debug/<system>/`: shared scene library, picker and hotkey, presets and checkpoints, reset and loop, replay, screenshots and metrics, sliders, guided-review panel, headless CLI or MCP; features covered | | |
| 2 | Observability: queryable state (API, CLI, or MCP), overlay or inspector, dumps, counters and timings, heatmaps and debug draws, live tunables, snapshot and restore; each failure mode and tunable names its probe | | |
| 3 | Networking: replicated state, authority, prediction or rollback, determinism, headless server | | |
| 4 | Papers followed; every departure named | | |
| 5 | One change per commit; normal push | | |
| 6 | Fast deterministic test; lavapipe for GPU; debug binary writes screenshots and metrics for the owner's PC | | |
| 7 | Existing code inventoried and reused | | |
| 8 | Perf: gated work, budget as ceiling, cheap check, no hard freeze, steady scene near non-GI floor | | |
| 9 | GI: even refinement in a region; no pop; no patchwork | | |
| 10 | Numbers, screenshots when visual, five-line postmortem on rollback or false pass | | |
| 11 | Small task; long builds and benchmarks left as commands | | |
| 12 | This table is in the pull request | | |
