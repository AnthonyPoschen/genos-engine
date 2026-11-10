# genos-ecs

Headless check for the scene store in `genos-ecs`. No window and no Vulkan.

```sh
cargo run -p genos-ecs-debug -- --check
cargo run -p genos-ecs-debug -- --check --mutate stale_accept
```

`--check` spawns an object, edits one component, and rejects a write to the despawned handle. It writes `./genos-debug/ecs/metrics.json`. `--out <dir>` changes that directory. The process does not write under `$HOME`.

`--mutate stale_accept` expects the despawned handle to accept a write. `--mutate skip_tick` expects the edit to leave the component tick unchanged. The store does neither, so the process exits non-zero while the mutation is on.
