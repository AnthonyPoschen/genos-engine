# genos-ecs

Headless check for the scene store in `genos-ecs`. No window and no Vulkan.

```sh
cargo run -p genos-ecs-debug -- --check
cargo run -p genos-ecs-debug -- --check --mutate stale_accept
```

`--check` spawns an object, edits one component, and rejects a write to the despawned handle. It writes `./genos-debug/ecs/metrics.json`. `--out <dir>` changes that directory. The process does not write under `$HOME`.

`--mutate stale_accept` plants a write that succeeds on the despawned handle. `--mutate skip_tick` plants an edit that emits no delta. The check is the same either way, and it exits non-zero while the mutation is on.
