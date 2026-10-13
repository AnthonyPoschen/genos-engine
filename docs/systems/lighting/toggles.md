# Lighting settings

`LightingSettings` (`crates/scene/src/lighting.rs`) is the reference [engine feature setting](../../engine/settings-principles.md). It implements `EngineSettings`. A game, a scene file, Rhai, MCP, and the debug menu read and write that one struct. The next draw applies the change. A knob that changes a buffer size rebuilds the buffers after the frames in flight finish.

The default is preset `high`: the tuned GI path. Pictures taken with today's defaults stay on that path.

## Networking

- Replicated state: the settings struct. GPU caches, probes, and timestamps stay on the client that draws.
- Authority: the client that owns the view. A server does not light the picture.
- Prediction / rollback: none. The settings are presentation data.
- Determinism: the same settings select the same passes. The GPU passes are not cross-device deterministic.
- Headless server: it does not run the lighting passes. It can store the struct and forward it.

## Features

Each feature can be turned off. Off means the pass is not recorded and, where the feature owned a buffer, that buffer is not allocated. A shader gate still has to match, so a disabled feature cannot sample a buffer that is not there.

| Feature | What off skips | Quality knobs |
|---|---|---|
| Direct | The pixel direct dispatch. Cache-ray and kept-hit lighting still run, and compose reads emission from the G-buffer instead of the unwritten glow half | `direct_lights` (0 = every lamp), `direct_refresh` (every N frames), `direct_scale` (1 = every pixel) |
| Shadows | The shadow ray and the shadow-cell cache inside the direct dispatch. GPU time for that bucket is 0 | `shadow_filter` `hard` or `cell`, `shadow_cell` metres, `shadow_refresh` |
| GI | Every GI pass and the GI buffers (the million-slot cache and the probe work) | `gi_refresh` (cache round every N frames), `budget_shift` (divide ray budgets by 2^shift), `world_mode` |
| Bounces | `0` is direct light plus flat ambient: no GI dispatch is recorded and the GI buffers are not allocated. `1` traces screen probes and shades their hits, and does not record the surface-cache, cache-ray, or a-trous dispatches. `2` allows cache-to-cache and records those dispatches. `unlimited` is the tuned path. A lower count omits the dispatches; it does not run them and zero the colour | `bounces` |
| Probes | Place, trace, gather, filter | `probe_rays` (4..64), `probe_tile` (0 = the picture-height rule), `probe_refresh`, `probe_stale`, `wake` (`legacy` or `rings`), `wake_tol`, `wake_budget`, `wake_radius` |
| World cache | Surface cache, world mode 3. Mode 0 is probe GI. The a-trous filter runs only in mode 3. A still probe-only scene settles once the light has held: probes, cache rays, and hit lighting stop, and only pixel direct and compose run, so off does not cost more than on. A moving frame keeps the probe passes. Before that settle, a still pixel still reuses direct light | `world_cell`, `world_refresh`, `world_stale` |
| Denoiser | The a-trous dispatches. Compose then shows each texel's own mean, so it does not walk the parent field the push would have filled | `denoise_iters` (1, 3, or 5), `denoise_dirty` |
| Compose reuse | The indirect block list. The compose pass still runs, over the whole image, and does not read the key table | `compose_reuse` |
| Sky | Sky radiance on rays that miss. The scene sky colour stays; this bit stops the lighting from reading it | `sky` |
| Emissive | Emitted light added in the direct pass and in compose | `emissive` |
| Prefill | WRC scout rays | `prefill` |
| Ambient | Flat fill when bounce is 0 or GI is off. `off`, `constant`, `hemisphere`, `day`, or `curve` | `ambient_color`, `ambient_intensity`, `ambient_ground`, `night_color`, `night_intensity`, `day_color`, `day_intensity`, `dawn_color`, `dawn_intensity`, `noon_color`, `noon_intensity`, `dusk_color`, `dusk_intensity`, `time_of_day` |

`day` lerps night and day colour and intensity by the sun's height. Rays travel along a light's direction; the sun is up when that direction points down. Hemisphere mixes ground colour away from the normal with the sky colour toward it. `curve` samples night, dawn, noon, and dusk by `time_of_day` (hours, `0` midnight, `6` dawn, `12` noon, `18` dusk) and also scales the sky colour by that sample relative to the noon key. Noon white at intensity 1 leaves the sky unchanged. `day` does not scale the sky. Bounce 0 adds the flat fill on top of direct light. One bounce or more does not add it.

Shadows share the direct dispatch. Their GPU time is a separate bucket: when shadows are on, the direct and hit-light milliseconds are also charged to `shadows`. When shadows are off, that bucket stays at 0 dispatches and 0 ms.

The S78S isolate toggles (`GENOS_LIGHTING_TOGGLES`: `freeze_cache`, `direct_only`, `bounce_only`, `freeze_probes`) still attribute a change to one component. They are not this struct.

## Presets

| Preset | What it spends | Cost, structural |
|---|---|---|
| `potato` | Direct, hard shadows, flat ambient `(0.42, 0.44, 0.48)` at intensity 0.65. No GI. Emissive stays on | Direct and compose only. No ~368 MB cache and no key table. Shadow rays stay inside the direct pass. The hangar look: direct, shadows, ambient fill |
| `day-cycle` | Direct, cell shadows, sky, and the time-of-day ambient curve. No GI | Direct and compose only. The curve scales the sky from its noon key. No GI buffers |
| `gi-1bounce` | Screen probes, 64 rays, one bounce. Surface cache, denoiser, and prefill off | Probe passes only. No surface-cache, cache-ray, or a-trous dispatches |
| `low` | Probe GI, 16 rays, 32 px tile, refresh every 2 frames, one bounce, budget shift 2 (budgets / 4), shadow cell 0.25 m, max 4 lights, 1 a-trous iteration stored but the filter does not run (no surface cache), prefill off, day ambient at 0.35. Gather bins `probe_rays`, not a fixed 64 | Probe GI. One bounce shades probe hits and does not record the surface-cache, cache-ray, or a-trous dispatches. No prefill |
| `medium` | Surface cache, 32 rays, two bounces, budget shift 1 (budgets / 2), max 8 lights, 3 a-trous iterations, prefill, day ambient. The push reads the last iteration that ran, copied into buffer A | Surface cache and two bounces, half the ray budget |
| `high` | Every feature on, unlimited bounces, 64 rays, tile from the picture height, refresh every frame, shadow cell 0.125 m, max lights unlimited, 5 a-trous iterations, compose reuse, sky, emissive, prefill. Day ambient is the fallback for when GI is turned off | The tuned path. Same dispatches as before this struct existed |

`high` on a due frame builds the same group list the frame recorded before settings existed. Shader gates load the control block and fall through when the bits match that path. The default shadow cell is 0.125 m and the shader divides by the `LIGHTING_SC_TEXEL` constant, so a still frame stays on the old quantisation. A different `shadow_cell` is the variable division.

## Scene file and Rhai

`lighting_preset("low")` replaces the struct. `lighting("direct_lights=3; ambient_intensity=0.4")` overlays lines (`key=value`, split on newlines and `;`). A bad line fails the script. `preset=` inside the text replaces, then later keys overlay. After a manual edit the text prints `preset=custom` unless the knobs are exactly a named preset again. `preset=custom` does not replace the struct; the other lines are the value.

The text form is `LightingSettings::to_text` / `parse`. Floats print with their debug form so they round-trip.

## Runtime

`Renderer::set_lighting_settings` holds an override until `clear_lighting_override`. A game that owns the scene writes `scene.lighting` and leaves the override clear. The draw copies the override or the scene, then applies launch locks. `--preset`, `--bounces`, and `--ambient` win every frame. `GENOS_LIGHTING_PRESET`, `GENOS_LIGHTING_BOUNCES`, and `GENOS_LIGHTING_AMBIENT` apply only when that argument was omitted, and then they win every frame. `--preset` or a set `GENOS_LIGHTING_PRESET` replaces the struct, then the other locks overlay one key each. `--time-of-day H` or `--time-of-day HH:MM` sets the stress start clock (0 midnight, 6 dawn, 12 noon, 18 dusk) and keeps `GENOS_LIGHTING_TIME` from replacing it. When that argument was omitted, a set `GENOS_LIGHTING_TIME` holds that hour. The scene clock survives a preset replace unless the time mirror is the lock. Any other `GENOS_LIGHTING_*` variable that is set still wins every frame for the key it names. `GENOS_LIGHTING_COMPOSE_ALL` forces compose reuse off. `GENOS_LIGHTING_WAKE` (`legacy` or `rings`), `GENOS_LIGHTING_WAKE_TOL`, `GENOS_LIGHTING_WAKE_BUDGET` and `GENOS_LIGHTING_WAKE_RADIUS` overlay `wake`, `wake_tol`, `wake_budget` and `wake_radius`. `--wake` removes `GENOS_LIGHTING_WAKE` for the process, so the argument wins.

The stress debug menu edits `world.scene.lighting` through `set_key`. The knobs are the preset, each feature switch, and the quality knobs (max lights, refresh, resolution scale, shadow filter, GI refresh, budget shift, probe rays, probe and world staleness, probe wake, denoiser iterations, ambient mode). Probe wake is `legacy` (one check ray, and a settled scene drops the probe passes) or `rings` (ring tests around a lamp or a moved surface, plus a verification budget). A settled rings frame uses that same idle and adds the verification stride. `wake_tol` is the relative luminance a group or a verification sample may move (default 0.08). `wake_budget` is how many probes the verification retraces each frame (default 16, 0 off). `wake_radius` is the furthest ring in metres (default 8, 0 tests no rings). The next draw picks that up. `genos-stress --wake legacy|rings` sets the same key and removes `GENOS_LIGHTING_WAKE` for the process, so the argument wins. The variable applies only when `--wake` was omitted.

## MCP

On the existing command catalogue (`GENOS_MCP_PORT`):

- `get_lighting_settings` — the settings text and each bucket's last collected milliseconds and dispatch count. One frame behind the draw that just finished.
- `set_lighting_settings` — partial update. Keys match the text form. A `text` field overlays `key=value` lines. Writes the scene and clears a renderer override.
- `list_presets` — `potato`, `day-cycle`, `gi-1bounce`, `low`, `medium`, `high`, each with its text.
- `apply_preset` — `name` replaces the scene lighting and clears the override.

GPU times need `--gpu-times N` (or `GENOS_GPU_TIMES` when that argument was omitted) before the renderer opens. The names are `compact`, `place`, `trace`, `direct`, `gather`, `filter`, `cache`, `compose`, `atrous`, `cache_rays`, `hit_light`, `shadows`. `--lighting-debug` prints the lighting counters (`GENOS_LIGHTING_DEBUG` applies only when that argument was omitted). `--wake-log <run>` writes the stress log to `./genos-debug/wake/<run>/run.txt`.

## Owner commands

Run these on the owner machine. They are not the lavapipe check.

```
cargo run --release -p genos-stress -- --preset potato --gpu-times 30
cargo run --release -p genos-stress -- --preset day-cycle --gpu-times 30
cargo run --release -p genos-stress -- --preset gi-1bounce --gpu-times 30
cargo run --release -p genos-stress -- --preset low --gpu-times 30
cargo run --release -p genos-stress -- --preset medium --gpu-times 30
cargo run --release -p genos-stress -- --preset high --gpu-times 30
```

`make stress` is `cargo run --release -p genos-stress` and does not forward these arguments, so the commands above call cargo directly. `GENOS_GPU_TIMES` and `GENOS_LIGHTING_PRESET` still apply when the matching argument was omitted. The `LIGHTING_MS` line names the passes and appends `cache_rays`, `shadows`, and `prefill`. `prefill` is 1 when the scout was due and 0 when it was not. The scout shares the cache-ray dispatch, so this is not a separate millisecond.

`hall` and `roomA` sit under the roof, so dawn, dusk, and night look alike there. `--view sky` stands in the south yard and looks up: the sky and the open ground show the curve. `--freeze-sun` holds the sun on that hour.

```
cargo run --release -p genos-stress -- --preset day-cycle --view sky --time-of-day 06:00 --freeze-sun
cargo run --release -p genos-stress -- --preset day-cycle --view sky --time-of-day 18:24 --freeze-sun
cargo run --release -p genos-stress -- --preset day-cycle --view sky --time-of-day 22:00 --freeze-sun
```

Lavapipe checks the look and the still-frame cost ratio. `presets_and_toggles_match_their_look` draws each preset and each feature off and rejects an unwritten frame, a saturated field, a flat-colour blowout, blue tiles, and white boxes. `world_cache_off_still_is_not_slower` holds a still camera until probe-only has settled, then checks that its lighting milliseconds are at most the surface cache's and that cache rays and hit lighting dispatched nothing.
