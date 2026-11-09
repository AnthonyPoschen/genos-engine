# genos-particles

Stage-1 particle debug binary. It runs the CPU mirror of the compute step and prints one `fx.*` line. It does not open a window.

```text
cargo run -p genos-particles -- --frames 8 --fx-count 64
```

`fx.sim_ms`, `fx.sort_ms`, and `fx.draw_ms` are 0 in this binary. `fx.sort_ms` and `fx.overdraw_px` stay 0 in stage 1. The GPU agreement test, the budget cap, the seed replay, and the skip-indirect picture are `cargo test -p genos-render --test particles_gpu` on lavapipe. Those timestamps are lavapipe, not a desktop-GPU score.

MCP and the lighting CLI share the catalogue: `get_particle_settings`, `set_particle_settings`, `get_particle_stats`, and the design names `fx.stats`, `fx.set`, `fx.list`, `fx.spawn`. Launch flags are `--fx-preset`, `--fx-max`, `--fx-count`, `--fx-freeze`, and `--fx-spawn`. The matching `GENOS_FX_*` variables apply only when that flag was omitted. `--fx-spawn` has no mirror.

The scene default stays off, with count 0. A plain `genos-particles` run fills count 64 and turns the emitter on so the printed line is not all zeros. Passing `--fx-count` also turns it on. That switch is this binary only. The engine stays off until a setting says otherwise. The 64 is this binary's omitted count, not the engine budget. The engine budget is the preset: Low 65536, Medium 262144, High 1048576.
