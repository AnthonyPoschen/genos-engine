# Genos Engine

Genos Engine is a Rust game runtime and a Linux editor. The runtime runs on Linux, Windows, and macOS. The editor runs on Linux. Omarchy is the first desktop the editor must fit. Engine code makes each native call to the operating system or to Vulkan.

The hosting product stays in the `genos` repository.

## Run

`make run` opens the lit scene as a normal window. The window shows the frame rate and the stage times. Click it, then use `WASD` or `IJKL` to walk and the mouse to look. `Escape` releases the pointer. Close the window to quit. Press `P` to pause that graph. Drag across the graph to inspect a time interval. Press `R` to follow live frames again. Pass `--profile detailed` to record GPU time. Pass `--profile off` to hide the profiler.

`make stress` opens the lighting stress building: a 25 m section with windows, skylights, wide doorways, still and moving lamps, a sun on a two-minute day, and sliding, spinning boxes. The panel switches the lamp count, the moving share, the scale and the sun live. `make stress-big` opens 16 sections with 100 lamps. `make stress-bench` sweeps lamp count × moving share × scale and prints tables. `genos-stress --help` lists every option. See `examples/stress/README.md`.

Checks use `genos-camera --proof`. That window stays floating and does not take focus.

## Plan

- `CONTEXT.md` holds the vocabulary.
- `docs/architecture.md` holds the crate map and the first proof.
- `docs/goals/mvp-scene.md` is the execution goal for that proof.
- `docs/adr/` holds the decisions.
- `AGENTS.md` tells an agent which of those files to open.
- `docs/references/breaking-point.md` names the old engine to read for math and camera ideas.

The input behavior comes from [`input-zig`](https://github.com/AnthonyPoschen/input-zig). This repository will port that library to Rust.
