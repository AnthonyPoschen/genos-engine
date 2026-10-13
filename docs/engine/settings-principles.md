# Engine feature settings

Every feature of the runtime that has a setting follows the three rules below. Lighting is the reference. The type is `LightingSettings` in `crates/scene/src/lighting.rs`. Its knobs, presets, and commands are in [the lighting toggles](../systems/lighting/toggles.md).

## The three surfaces

1. **A public settings value.** The setting is a public, serializable type on the engine API. Any later system that can see the type can read it and write it: a game, a scene, a script, the renderer, the debug tools. The shared contract is `EngineSettings` in `crates/scene/src/settings.rs`: `to_text`, `parse`, and `set_key`. `to_text` and `parse` round-trip the whole value. `set_key` overlays one knob. `LightingSettings` implements the trait. The live lighting value is `Scene.lighting`.

2. **Agents, over MCP.** The same value is on the command catalogue (`GENOS_MCP_PORT`). Lighting's commands are `get_lighting_settings`, `set_lighting_settings` (a partial update through `set_key`), `list_presets`, and `apply_preset`. A read returns the text form and the measurements that show what the change did. For lighting, those measurements are the GPU pass times and dispatch counts.

3. **People, in the debug menu or UI.** The same value is editable from the debug menu. The stress knobs in `examples/stress/src/knobs.rs` write `Scene.lighting` through `set_key`. The menu has the preset, the feature switches, and the quality knobs: refresh, rays per probe, resolution scale, max lights, shadow filter, denoiser iterations, staleness, budget shift, probe wake (mode, tolerance, verification budget, max ring radius), and ambient mode.

A scene file, a Rhai call, and a `GENOS_LIGHTING_*` variable are further editors of that same struct. A variable that is set at launch wins every frame for the key it names.

## Games

A game may map its own player-facing settings onto this config, or it may expose the raw engine config.

```rust
scene.lighting = LightingSettings::preset(LightingPreset::Low);

scene.lighting.set_key("probe_rays", "32")?;
```

The game owns the words a player sees. The engine owns the struct those words write.

## A later feature

A new feature adds a public struct that implements `EngineSettings`, stores it where other systems can already reach it, registers MCP commands that read and write that struct, and adds debug-menu controls that call `set_key`. Copy lighting.

`EngineSettings` lives in `genos-scene` because games, the renderer, and the debug tools already share that crate. A feature crate that `genos-scene` itself depends on keeps the same three methods on its own struct until a second implementor needs the trait in a crate both can see.

## Output directories

A command that writes files follows the Output rule in `AGENTS.md`. The directory is a field of a public `EngineSettings` value. Debug output is `DebugSettings` on `Scene`, key `out`. The default is `genos-debug/unfilled` under the working directory, so an unfilled event lands in `./genos-debug/unfilled/<timestamp>/`. `--out <dir>` overrides that default for the process. `GENOS_UNFILLED_DIR` is the environment mirror of the same field: when it is set, and `--out` was not passed, it wins every frame. The process does not choose `$HOME`. The unfilled count, tint, dump threshold, and caps are arguments too (`--unfilled`, `--unfilled-tint`, `--unfilled-dump`, `--unfilled-max-age-secs`, `--unfilled-max-folders`, `--unfilled-max-bytes`, `--unfilled-session-cap`). Each matching `GENOS_*` variable applies only when that argument was omitted. `genos-stress --wake legacy|rings` sets the lighting `wake` key and removes `GENOS_LIGHTING_WAKE` for the process. That variable applies only when `--wake` was omitted. `--gpu-times N` and `--lighting-debug` are read once when the renderer opens. `GENOS_GPU_TIMES` and `GENOS_LIGHTING_DEBUG` apply only when the matching argument was omitted. `--wake-log <run>` writes the stress process log to `./genos-debug/wake/<run>/run.txt` (a value with a slash, or `.` or `..`, is the directory). `GENOS_WAKE_LOG` applies only when `--wake-log` was omitted. The log is attached once, after argument parsing and before the first frame, so it is a launch argument in the same class as `--gpu-times` rather than a live `DebugSettings` field. `--out` remains the unfilled directory.
