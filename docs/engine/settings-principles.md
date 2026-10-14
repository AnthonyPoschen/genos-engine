# Engine feature settings

Every feature of the runtime that has a setting follows the three rules below. Lighting is the reference. The type is `LightingSettings` in `crates/scene/src/lighting.rs`. Its knobs, presets, and commands are in [the lighting toggles](../systems/lighting/toggles.md).

## The three surfaces

1. **A public settings value.** The setting is a public, serializable type on the engine API. Any later system that can see the type can read it and write it: a game, a scene, a script, the renderer, the debug tools. The shared contract is `EngineSettings` in `crates/scene/src/settings.rs`: `to_text`, `parse`, and `set_key`. `to_text` and `parse` round-trip the whole value. `set_key` overlays one knob. `LightingSettings` implements the trait. The live lighting value is `Scene.lighting`.

2. **Agents, over MCP.** The same value is on the command catalogue (`GENOS_MCP_PORT`). Lighting's commands are `get_lighting_settings`, `set_lighting_settings` (a partial update through `set_key`), `list_presets`, and `apply_preset`. A read returns the text form and the measurements that show what the change did. For lighting, those measurements are the GPU pass times and dispatch counts.

3. **People, in the debug menu or UI.** The same value is editable from the debug menu. The stress knobs in `examples/stress/src/knobs.rs` write `Scene.lighting` through `set_key`. The lighting debug panel draws an ordered grade as a slider (preset, bounces, refresh, rays per probe, resolution scale, max lights, denoiser iterations, staleness, budget shift) and an unordered choice as a dropdown (scene, checkpoint, view, shadow filter, ambient mode). An on/off feature switch is a checkbox on that panel.

A scene file, a Rhai call, a launch argument, and a `GENOS_LIGHTING_*` variable are further editors of that same struct. `--preset`, `--bounces`, `--ambient`, and `--time-of-day` win. `GENOS_LIGHTING_PRESET`, `GENOS_LIGHTING_BOUNCES`, `GENOS_LIGHTING_AMBIENT`, and `GENOS_LIGHTING_TIME` apply only when that argument was omitted, and then a variable that is set wins every frame for the key it names. The other `GENOS_LIGHTING_*` variables have no argument and still win every frame when they are set. `--time-of-day` sets the program's clock and leaves that clock in charge; `GENOS_LIGHTING_TIME` holds the hour only when the argument was omitted.

## Games

A game may map its own player-facing settings onto this config, or it may expose the raw engine config.

```rust
scene.lighting = LightingSettings::preset(LightingPreset::Low);

scene.lighting.set_key("probe_rays", "32")?;
```

The game owns the words a player sees. The engine owns the struct those words write.

## A later feature

A new feature adds a public struct that implements `EngineSettings`, stores it where other systems can already reach it, registers MCP commands that read and write that struct, and adds debug-menu controls that call `set_key`. Copy lighting.

`FpsCounterSettings` on `Scene` is the frame-rate overlay (`enabled`, `rate_hz`). It is off at 4 Hz until `--fps-counter`, the debug menu, or MCP turns it on. The debug menu draws `enabled` as a checkbox and `rate_hz` as a numeric slider. Both call `set_key`. `GENOS_FPS_COUNTER` and `GENOS_FPS_HZ` are the environment mirrors. The overlay itself is drawn by `genos-lighting`.

`MenuSettings` on `Scene` is the debug menu (`visible`, `tab`, `collapsed`, `wheel`, `keys`). It is shown until `--menu hidden`, the hide control, or MCP sets `visible` to hidden. `tab` is the page title. `collapsed` is the folded section keys. `fold` adds one key and `open` removes one. A fold stays when `tab` changes in the same write, including when that section is not on the new page. `wheel` and `keys` are each `hover`, `modal`, `click`, or `passive`. The debug menu default is `wheel=hover` and `keys=click`. The Perf page writes them with the Wheel focus and Key focus dropdowns. The lighting panel writes `visible`, `tab`, and `collapsed`. `--menu shown|hidden` wins every frame for `visible`. `GENOS_MENU` applies only when that argument was omitted. `get_menu_settings` and `set_menu_settings` are the MCP commands. `focus_wheel` and `focus_keys` are aliases of `wheel` and `keys`.

`EngineSettings` lives in `genos-scene` because games, the renderer, and the debug tools already share that crate. A feature crate that `genos-scene` itself depends on keeps the same three methods on its own struct until a second implementor needs the trait in a crate both can see.

## Output directories

A command that writes files follows the Output rule in `AGENTS.md`. The directory is a field of a public `EngineSettings` value. Debug output is `DebugSettings` on `Scene`, key `out`. `ui` on that same value defaults to false. A capture, a batch of captures, and a compare live shot leave out the debug menu, the menu tab, the frame-rate plate, and tool overlays unless that shot sets `ui` or the setting is on. `--ui` forces the setting on. `get_debug_settings` and `set_debug_settings` read and write it. The default is `genos-debug/unfilled` under the working directory, so an unfilled event lands in `./genos-debug/unfilled/<timestamp>/`. `--out <dir>` overrides that default for the process. `GENOS_UNFILLED_DIR` is the environment mirror of the same field: when it is set, and `--out` was not passed, it wins every frame. The process does not choose `$HOME`. The unfilled count, tint, dump threshold, and caps are arguments too (`--unfilled`, `--unfilled-tint`, `--unfilled-dump`, `--unfilled-max-age-secs`, `--unfilled-max-folders`, `--unfilled-max-bytes`, `--unfilled-session-cap`). Each matching `GENOS_*` variable applies only when that argument was omitted.
