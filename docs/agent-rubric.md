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
- Debug and proof: `docs/systems/debugging.md`. Gameplay networking waits (`docs/architecture.md`). Fill in the networking row anyway.

## Checklist

1. **Networking.** A new component or system states, in its system record, these five lines. Local-only work still answers each one. Gameplay networking still waits; the lines are the contract for a later server. The loopback agent endpoint stays the live-scene path.

   - Replicated state:
   - Authority:
   - Prediction / rollback:
   - Determinism:
   - Headless server:

2. **Papers.** The implementation follows the cited paper's algorithm and structure. Every departure is explicit in the pull request: the paper, the section or equation, and what the code does. Lighting departures also land in the deviations list in `docs/systems/lighting/README.md`, in the same change. An ADR conflict is named.

3. **Commits.** One change per commit. Push with `git push` so the remote fast-forwards. A force push, `--force-with-lease`, or a rewrite of a pushed branch fails this row.

4. **Fast test.** Each change has a fast deterministic test. GPU work runs under lavapipe (software Vulkan) on the headless path in `docs/systems/debugging.md`. Real performance and visual judgment happen on the owner's PC (zanven-pc with the RTX 4070, and the MacBook). When the result is visual, ship a runnable script or binary that writes screenshots and metrics (`genos-stress` scripts, `gi-check`). The session adds that command; the owner's PC runs it.

5. **Inventory.** Read the crates, the system record, and, for lighting, `docs/systems/lighting/experiments/` before adding code. Build on the piece that already works. A new crate, shader, or path says why the existing one cannot carry the change.

6. **Performance.** Work is gated by need. A budget is a ceiling (E19 in the lighting experiments). A cheap check verifies the frame; "nothing changed" is a result of that check (`docs/systems/lighting/README.md`, S74A). The frame, the desktop, and a live cache stay free of a hard freeze. A steady scene's frame time approaches that scene's non-GI floor.

7. **GI quality.** Refinement is even inside a region. That region shows no pop and no patchwork. Quality may change at a logical boundary (a mesh, a room, a layer). A fill that finishes in the same frame is the allowance already in the lighting record, and the region is still even.

8. **Report.** The pull request or the experiment note carries numbers: what was measured, where, and the before and after. A visual change carries the screenshots the script wrote, and they are looked at. A rollback or a false pass has five lines:

   1. Claim:
   2. Check:
   3. Fault:
   4. First seen:
   5. Left reverted:

9. **Small tasks.** One concern, small enough to review. A long build, a full `gi-check`, or a path trace is a command left for the owner's PC. Run the fast test from row 4, then continue. The Rust gate in `AGENTS.md` still applies to the crates you touched.

10. **Debuggability.** Hook the change into the debug binaries (`genos-debug`, `genos-stress`, the MCP command queue in `docs/systems/debugging.md`) and the shared scenes (the stress building, `examples/stress/scenes/room.rhai`, `examples/camera/scene.rhai` through `crates/scene`). Expose a preset (an existing knob or stress preset: scale, lamps, dynamic, sun, time of day) and a reset that returns the clock and the movers to the start.

11. **Scored checklist.** The pull request includes the table below, filled in. This row is `pass` only when every other row has a score and evidence.

## Pull request table

| # | Item | Score | Evidence |
|---|---|---|---|
| 1 | Networking: replicated state, authority, prediction or rollback, determinism, headless server | | |
| 2 | Papers followed; every departure named | | |
| 3 | One change per commit; normal push | | |
| 4 | Fast deterministic test; lavapipe for GPU; visual script for the owner's PC | | |
| 5 | Existing code inventoried and reused | | |
| 6 | Perf: gated work, budget as ceiling, cheap check, no hard freeze, steady scene near non-GI floor | | |
| 7 | GI: even refinement in a region; no pop; no patchwork | | |
| 8 | Numbers, screenshots when visual, five-line postmortem on rollback or false pass | | |
| 9 | Small task; long builds and benchmarks left as commands | | |
| 10 | Debug binary, shared scene, preset, reset | | |
| 11 | This table is in the pull request | | |
