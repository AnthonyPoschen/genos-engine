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
| `resting_pile` | 16 turned hull cubes, friction 0.6, dropped onto a static hull slab. | settle and sink checks, including sleep off |
| `static_grid_10k` | 10 000 static boxes on a 100 x 100 grid, inserted as one batch, 8 spheres falling onto it. | broadphase cost; statics stay out of the dynamic tree |
| `resting_1025` | 1 024 spheres on a plane. | scale bench |
| `layer_filter` | Two overlapping debris spheres. | `pair_tests` stays 0 |
| `sleep_island` | Two separated stacks. | both islands sleep; a displaced sphere wakes one |
| `chassis_rest` | 1 200 kg chassis, four wheel rays, static plane, awake for 180 steps. | `a_chassis_rests_on_four_springs` |
| `thruster` | 1 000 kg cube, one offset thruster, gravity zero, 60 steps. | `a_thruster_in_zero_gravity_matches_force_and_torque` |
| `orbit` | Circular orbit at 1e7 m, gravity zero, a second body with no orbit, 200 steps of 1/60 s. | `a_circular_orbit_at_1e7_metres_keeps_its_radius` |

Scenes are built in code (`src/scenes.rs`). The shared `debug/scenes/*.scene`
files describe lighting geometry and have no bodies, masses or velocities.

## Per-step stats

`step`, `step_us`, `bodies`, `ccd_applied`, `toi_failed`, `toi_deferred`,
`pair_slices`, `dynamic_pair_impulses`, `linear_energy`, `rotational_energy`,
`max_speed`, `min_y`, `hash`, `contact_points`, `sleeping_bodies`,
`pair_tests`, `broadphase_builds`, `tree_nodes`, `islands`, `awake_bodies`,
`solver_bodies`, `wheel_rays`, `vehicle_count`, and `orbit_radius_m`. `hash`
is FNV-1a over every body's pose and velocities, then each stored wheel length,
bit for bit. An orbit also mixes its wide position, wide velocity, and
gravitational parameter. The hash then mixes `World.origin`. Equal hashes at a
step mean equal state. A world with no vehicle hashes the bodies and the origin.
`orbit_radius_m` is 0 when the scene has no orbit.

## MCP and `POST /cmd/<name>`

`physics_scenes`, `physics_load {scene}`, `physics_reset`,
`physics_step {steps}`, `physics_stats`, `physics_body {index}`,
`physics_replay {steps}`, `physics_quit`. The port is `--mcp-port N`, or
8767 with `--mcp` (lighting uses 8766).

## Not yet

A picker hotkey, presets, a loop and screenshots all need a window. They come
with a debug draw.

`chassis_rest` is the G1 trial, `thruster` is the S1 trial, and `orbit` is
the S2 trial from [ADR 0040](../../docs/adr/0040-vehicle-domains.md). The
thruster scene starts awake. A chassis that begins that scene asleep, with the
thruster already open, wakes on the first step. The orbit scene steps at 1/60 s.
The 200 steps of 1 s radius gate is the physics test, not this harness. The ADR
stays proposed. A1 would add `aero_trim` on this same headless path. That scene
is not here. Sea has no scene.
