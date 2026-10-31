# debug/physics

`genos-physics`, the physics debug binary. It runs **headless with stats only**:
no window and no debug draw. The render and window crates need a GPU and a
display, and drawing bodies would need a draw path the physics crate does not
have, so a person reads the per-step JSON lines and an agent drives it over the
CLI or MCP.

```
cargo run --release -p genos-physics-debug -- --list
cargo run --release -p genos-physics-debug -- --scene resting_pile            # one JSON line per step
cargo run --release -p genos-physics-debug -- --scene ship_lanes --replay     # bitwise replay check
cargo run --release -p genos-physics-debug -- --scene hull_edges --checkpoint 20  # snapshot, restore, rerun
cargo run --release -p genos-physics-debug -- --mcp                           # MCP on 127.0.0.1:8767
```

## Scenes

| Name | What it shows | Test it mirrors |
|---|---|---|
| `ship_lanes` | Four swept wedge-hull pairs head on at 30, 120, 600 and 3000 m/s from 36 m. | `crates/physics/tests/ship_impact.rs` |
| `hull_edges` | Static crossed cubes 1 cm apart and 5 cm overlapped, and a cube falling edge-first onto a static edge. | `crates/physics/tests/hull_edges.rs` |
| `resting_pile` | 16 turned hull cubes dropped onto a static hull slab. | settle and sink checks |
| `static_grid_10k` | 10 000 static boxes on a 100 x 100 grid, 8 spheres falling onto it. | broadphase cost |

Scenes are built in code (`src/scenes.rs`). The shared `debug/scenes/*.scene`
files describe lighting geometry and have no bodies, masses or velocities.

## Per-step stats

`step`, `step_us`, `bodies`, `ccd_applied`, `toi_failed`, `toi_deferred`,
`pair_slices`, `dynamic_pair_impulses`, `linear_energy`, `rotational_energy`,
`max_speed`, `min_y`, and `hash`: FNV-1a over every body's pose and velocities,
bit for bit. Equal hashes at a step mean equal state.

## MCP and `POST /cmd/<name>`

`physics_scenes`, `physics_load {scene}`, `physics_reset`,
`physics_step {steps}`, `physics_stats`, `physics_body {index}`,
`physics_replay {steps}`, `physics_quit`. The port is `--mcp-port N`, or
8767 with `--mcp` (lighting uses 8766).

## Not yet

A picker hotkey, presets, a loop and screenshots all need a window. They come
with a debug draw.
