# genos-particles

Stage-1 particle debug binary. A plain run steps the CPU mirror and prints one `fx.*` line. `--list`, `--scene`, `--replay`, and `--checkpoint` match the physics debug binary. `--screenshot` opens a window and writes `./genos-debug/particles/<scene>.png` (`--out` changes the root). `--mcp` serves the particle commands on port 8768.

```text
cargo run -p genos-particles -- --frames 8 --scene burst
cargo run -p genos-particles -- --list
cargo run -p genos-particles -- --scene capped --replay --checkpoint 3
```

`fx.sim_ms`, `fx.sort_ms`, and `fx.draw_ms` are 0 in the CPU run. `fx.sort_ms` and `fx.overdraw_px` stay 0 in stage 1. The GPU agreement test is `cargo test -p genos-render --test particles_gpu` on lavapipe. Those timestamps are lavapipe, not a desktop-GPU score.

MCP on this binary: `particle_scenes`, `particle_load`, `particle_reset`, `particle_step`, `particle_stats`, `particle_replay`, `particle_checkpoint`, `fx_stats`, `fx_list`, `particle_quit`. The lighting catalogue also has `fx.stats`, `fx.set`, `fx.list`, `fx.spawn` and the Rhai names `fx_stats`, `fx_set`, `fx_list`, `fx_spawn`.

Launch flags are `--fx-preset`, `--fx-max`, `--fx-count`, `--fx-freeze`, and `--fx-spawn`. The matching `GENOS_FX_*` variables apply only when that flag was omitted. `--fx-spawn` has no mirror. A spawn that leaves the live cap at 0 sets it to 64.

Scenes: `burst` (default), `capped`, `frozen`, `idle`.
