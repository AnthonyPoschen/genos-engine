# Quality settings and graphics presets

This page explains how quality knobs and graphics presets fit together. The general rule for any engine setting is in [the settings principles](settings-principles.md). The preset design is in [ADR 0023](../adr/0023-graphics-settings-presets.md) and [settings-presets.md](../systems/rendering/settings-presets.md).

## The rule

Every quality knob is a field in one data-driven settings struct. Lighting is the reference: `LightingSettings` in `crates/scene/src/lighting.rs`, stored live as `Scene.lighting`. Reflections have `ReflectionPreset` in `crates/math/src/optics.rs`, and HDR output has its own settings in `crates/scene/src/hdr.rs`.

- The struct implements `EngineSettings` (`crates/scene/src/settings.rs`). `to_text` and `parse` round-trip the whole value, and `set_key` changes one knob.
- Each knob can be set from a launch argument, from MCP (`get_lighting_settings`, `set_lighting_settings`, `list_presets` and `apply_preset` in `crates/debug/src/commands.rs` and `crates/debug/src/tools.rs`), from the debug menu, and from scene files.
- A change applies at runtime with no restart. The next draw reads the new value. If a knob changes a buffer size, the renderer rebuilds that buffer after the frames in flight finish.

A preset is a named table of knob values and nothing more. `LightingSettings::preset(LightingPreset)` starts from the High defaults and overrides only what that tier changes. Any knob can still be changed by hand after a preset is applied. Presets never add separate code paths, so one renderer and one test harness cover every tier.

## Tiers and hardware targets

Master currently has these `LightingPreset` values: `Potato`, `DayCycle`, `Gi1Bounce`, `Low`, `Medium` and `High` (the default). `ReflectionPreset` has `Potato`, `Low`, `Medium` and `High`. Potato runs direct light, traced shadows and a flat ambient, with no GI buffers. It is the fallback, not a tuned player tier. `DayCycle` and `Gi1Bounce` are test steps between Potato and Low. Ultra is planned in ADR 0023 but not in code yet.

| Tier | Target hardware | Frame budget |
|---|---|---|
| Low | GTX 1060 6 GB / RX 580 at 1080p; Steam Deck at 800p | 16.6 ms; on the Deck, 40 fps in 40 Hz mode with a 30 fps floor at the worst moment |
| Medium | RTX 2060 / RX 6600 class, 1440p | 16.6 ms |
| High | RTX 4070 class, 1440p | 8.3 ms |

- The minimum spec is Vulkan 1.3 and 4 GB of VRAM. Ray queries are optional on every tier.
- Lighting has a soft VRAM budget of about 200 MB at Low.
- Low upscales with FSR, which is planned in ADR 0023 and not yet implemented.
- A knob value that misses its tier's frame-time or VRAM gate moves down a tier.

Pending: PR #135 (unmerged) renames `Potato` to `Fallback` (name `fallback`, with `potato` kept as a deprecated alias) in `LightingPreset`, `ReflectionPreset` and `PromotePreset`.

## Adding a setting

1. **Field.** Add the field to the feature's settings struct, for example a sub-struct of `LightingSettings`. Give it a `set_key` key and include it in `to_text` and `parse`.
2. **Per-preset defaults.** Set the High value in the defaults, then override it in `LightingSettings::preset` for each tier that differs. Pick the values from a measured cost on the tier's target hardware, not from a guess.
3. **CLI.** If a tool needs it at launch, add an argument that goes through `set_key`. The `--preset` handling in `debug/lighting/src/main.rs` is the pattern. If you also add an environment variable, it must mirror that argument, and it must be registered in `lint/env_mirrors.toml` as `mirror`.
4. **MCP.** Usually you don't need to do anything, because `get_lighting_settings` and `set_lighting_settings` already carry every key. A new feature struct gets its own `get_*` and `set_*` pair in `crates/debug/src/commands.rs`, and the read should return measurements that show the effect.
5. **Runtime apply.** The renderer reads the value every frame. A size change rebuilds its buffers, and a plain value change takes effect on the next draw. Never cache the value at startup.
6. **Test.** Add a round-trip and `set_key` test next to `crates/scene/tests/lighting_settings.rs`. If the knob changes the picture, add a check over MCP, following `crates/debug/tests/lighting_mcp.rs`.

## What not to do

- **No env-var-only quality knobs.** An environment variable mirrors a struct field and a launch argument. It is never the only way to set a value. `scripts/lint-env-mirrors.py` enforces this, and `kind = "todo"` entries are the backlog. Do not add new ones.
- **No hard-coded per-GPU branches.** The renderer must not branch on vendor or device ID to change quality. Startup may pick a preset from the device (VRAM, ray-query support), but the pick is only a preset value that the player and tools can change.
