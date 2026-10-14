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
| Direct | The pixel direct dispatch. Cache-ray and kept-hit lighting still run, and compose reads emission from the G-buffer instead of the unwritten glow half | `direct_lights` (0 = every lamp), `direct_refresh` (every N frames), `direct_scale` (1 = every pixel). `lamp_pick` (`off`, `hits`, `pixels`, `both`, `reuse (experimental)`) samples point lamps instead of tracing every one. `lamp_pick_samples` (alias `lamp_pick_rays`) is the shadow-ray count, 1..4. `lamp_pick_reuse` turns the temporal reservoir on or off. `lamp_pick_candidates`, `lamp_pick_radius` (0 = no spatial merge), `lamp_pick_history`, and `lamp_pick_unbiased` are the rest. `high`, `medium`, `low`, and `gi-1bounce` use `hits`. Potato and day-cycle stay `off`. `reuse (experimental)` is not a preset. `direct_lights` stays a hard cap and does not share a field with the sample count |
| Shadows | The shadow ray and the shadow-cell cache inside the direct dispatch. GPU time for that bucket is 0 | `shadow_filter` `hard` or `cell`, `shadow_cell` metres, `shadow_refresh` |
| GI | Every GI pass and the GI buffers (the million-slot cache and the probe work) | `gi_refresh` (cache round every N frames), `budget_shift` (divide ray budgets by 2^shift), `world_mode` |
| Bounces | `0` is direct light plus flat ambient: no GI dispatch is recorded and the GI buffers are not allocated. `1` traces screen probes and shades their hits, and does not record the surface-cache, cache-ray, or a-trous dispatches. `2` allows cache-to-cache and records those dispatches. `unlimited` is the tuned path. A lower count omits the dispatches; it does not run them and zero the colour | `bounces` |
| Probes | Place, trace, gather, filter | `probe_rays` (4..64), `probe_tile` (0 = the picture-height rule), `probe_refresh`, `probe_stale` |
| World cache | Surface cache, world mode 3. Mode 0 is probe GI. The a-trous filter runs only in mode 3. A still probe-only scene settles once the light has held: probes, cache rays, and hit lighting stop, and only pixel direct and compose run, so off does not cost more than on. A moving frame keeps the probe passes. Before that settle, a still pixel still reuses direct light | `world_cell`, `world_refresh`, `world_stale` |
| Denoiser | The a-trous dispatches. Every preset runs the hash-slot wavelet. `denoise_launch=screen` opts into a picture-sized launch of that kernel. With the denoiser off, compose shows each texel's own mean. The wavelet filters bounce light only; pixel direct is composed afterwards and stays sharp | `denoise_iters` (1, 3, or 5), `denoise_dirty` (hash wavelet), `denoise_launch` (`hash` or `screen`; `denoise_screen` on/off is the alias), `denoise_adaptive` (experimental, off on every preset; the hash wavelet only), `denoise_variance` |
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
| `low` | Probe GI, 16 rays, 32 px tile, refresh every 2 frames, one bounce, budget shift 2 (budgets / 4), shadow cell 0.25 m, max 4 lights, 1 full-resolution a-trous iteration stored but the filter does not run (no surface cache), adaptive off, prefill off, day ambient at 0.35. Gather bins `probe_rays`, not a fixed 64 | Probe GI. One bounce shades probe hits and does not record the surface-cache, cache-ray, or a-trous dispatches. No prefill |
| `medium` | Surface cache, 32 rays, two bounces, budget shift 1 (budgets / 2), max 8 lights, 3 hash-slot a-trous iterations, screen wavelet off, adaptive off, prefill, day ambient | Surface cache and two bounces, half the ray budget. The wavelet is the hash-slot kernel, three iterations |
| `high` | Every feature on, unlimited bounces, 64 rays, tile from the picture height, refresh every frame, shadow cell 0.125 m, max lights unlimited, 5 hash-slot a-trous iterations, compose reuse, sky, emissive, prefill. Day ambient is the fallback for when GI is turned off. Adaptive passes stay off. `denoise_launch=screen` opts into the picture-sized wavelet | The tuned path. The à-trous launch is one group per 64 cache slots, the same as gi-v2 |

`high` on a due frame builds the group list the frame recorded before settings existed, including the hash-slot à-trous (one group per 64 cache slots). `denoise_launch=screen` replaces those dispatches with one 8×8 group per picture block (gather, variance, five iterations, push). Shader gates load the control block and fall through when the bits match that path. The default shadow cell is 0.125 m and the shader divides by the `LIGHTING_SC_TEXEL` constant, so a still frame stays on the old quantisation. A different `shadow_cell` is the variable division.

## Scene file and Rhai

`lighting_preset("low")` replaces the struct. `lighting("direct_lights=3; ambient_intensity=0.4")` overlays lines (`key=value`, split on newlines and `;`). A bad line fails the script. `preset=` inside the text replaces, then later keys overlay. After a manual edit the text prints `preset=custom` unless the knobs are exactly a named preset again. `preset=custom` does not replace the struct; the other lines are the value.

The text form is `LightingSettings::to_text` / `parse`. Floats print with their debug form so they round-trip.

## Runtime

`Renderer::set_lighting_settings` holds an override until `clear_lighting_override`. A game that owns the scene writes `scene.lighting` and leaves the override clear. The draw copies the override or the scene, then applies launch locks. `--preset`, `--bounces`, and `--ambient` win every frame. `GENOS_LIGHTING_PRESET`, `GENOS_LIGHTING_BOUNCES`, and `GENOS_LIGHTING_AMBIENT` apply only when that argument was omitted, and then they win every frame. `--preset` or a set `GENOS_LIGHTING_PRESET` replaces the struct, then the other locks overlay one key each. `--time-of-day H` or `--time-of-day HH:MM` sets the stress start clock (0 midnight, 6 dawn, 12 noon, 18 dusk) and keeps `GENOS_LIGHTING_TIME` from replacing it. When that argument was omitted, a set `GENOS_LIGHTING_TIME` holds that hour. The scene clock survives a preset replace unless the time mirror is the lock. Any other `GENOS_LIGHTING_*` variable that is set still wins every frame for the key it names. `GENOS_LIGHTING_COMPOSE_ALL` forces compose reuse off. `GENOS_LIGHTING_FULL_EFFORT` (any value except `0`, `false`, `off`, `no`) is applied last. It writes the high-preset maximum: budget shift 0, 64 probe rays, unlimited bounces, refresh every frame, five dense a-trous iterations (`denoise_dirty` 0), and compose reuse off so every pixel is rewritten. The text form stores those knobs. `preset` prints `custom`. The same action is the key `full_effort` for MCP `set_lighting_settings` and the stress knob `lighting_full_effort`. Off restores preset `high`.

The stress debug menu edits `world.scene.lighting` through `set_key`. The knobs are the preset, each feature switch, and the quality knobs (max lights, refresh, resolution scale, lamp pick, samples per point, reservoir reuse, candidates, radius, history, unbiased, shadow filter, GI refresh, budget shift, probe rays, probe and world staleness, denoiser iterations, adaptive passes, variance threshold, denoise launch, ambient mode). The lighting debug binary draws an ordered grade as a slider, an unordered choice as a dropdown, and each on/off switch as a checkbox. Both write `set_key`. The next draw picks that up. `--lamp-pick`, `--lamp-pick-samples`, `--lamp-pick-rays`, `--lamp-pick-reuse`, `--lamp-pick-candidates`, `--lamp-pick-radius`, `--lamp-pick-history`, and `--lamp-pick-unbiased` win over the matching `GENOS_LIGHTING_LAMP_PICK*` variable. An omitted flag leaves that variable alone. Every preset keeps the hash-slot launch unless `denoise_launch` is `screen`.

## MCP

On the existing command catalogue (`GENOS_MCP_PORT`):

- `get_lighting_settings` — the settings text and each bucket's last collected milliseconds and dispatch count. One frame behind the draw that just finished.
- `set_lighting_settings` — partial update. Keys match the text form. A `text` field overlays `key=value` lines. Writes the scene and clears a renderer override.
- `list_presets` — `potato`, `day-cycle`, `gi-1bounce`, `low`, `medium`, `high`, each with its text.
- `apply_preset` — `name` replaces the scene lighting and clears the override.

GPU times need `--gpu-times N` (or `GENOS_GPU_TIMES` when that argument was omitted) before the renderer opens. The names are `compact`, `place`, `trace`, `direct`, `gather`, `filter`, `cache`, `compose`, `atrous`, `cache_rays`, `hit_light`, `shadows`.

## Owner commands

Run these on the owner machine (RTX 4070, 1920×1080, the moving stress scene). They are not the lavapipe check. A flag wins over the matching `GENOS_LIGHTING_*` variable. An omitted flag leaves that variable in effect. `make stress` is `cargo run --release -p genos-stress`. The `atrous` column is the one that should move. Lavapipe does not time the GPU. High is the full-resolution scalar wavelet.

```
cargo run --release -p genos-stress -- --preset potato --gpu-times 30
cargo run --release -p genos-stress -- --preset day-cycle --gpu-times 30
cargo run --release -p genos-stress -- --preset gi-1bounce --gpu-times 30
cargo run --release -p genos-stress -- --preset low --gpu-times 30
cargo run --release -p genos-stress -- --preset medium --gpu-times 30
cargo run --release -p genos-stress -- --preset high --gpu-times 30
```

Adaptive passes are experimental and off on every preset. They drop later wavelet iterations on texels whose sample count is high or whose relative variance `sqrt(var) / mean` is under `denoise_variance` (stored at 0.02). The last stage publishes the level that ran, so a clean skip does not leave the push on an empty buffer. The 4070 retest missed the picture bar, so medium stays the 3-iteration scalar wavelet. A threshold of 0.1 forces one iteration. Half-res and shared-memory tiles were removed after they lost the 4070 A/B (S79).

```
cargo run --release -p genos-stress -- --size 1920x1080 --gpu-times 30 --preset high --denoise-adaptive on --denoise-variance 0.02
cargo run --release -p genos-stress -- --size 1920x1080 --gpu-times 30 --preset high --denoise-adaptive off
```

High's denoiser is the hash-slot wavelet, the same launch as medium and as gi-v2. `--denoise-launch screen` is the opt-in picture wavelet; the `atrous` column is the one that should move. A still frame is `--still-boxes --freeze-sun --dynamic 0`. The moving default is dynamic 50 with the sun running. Lavapipe does not time the GPU. Sponza is `genos-lighting`. `--checkpoint` jumps a named pose. `--eye x,y,z` moves the camera and `--look x,y,z` aims it, so a small move off that pose does not need a new scene line. Yaw 0 looks down -Z.

```
cargo run --release -p genos-stress -- --size 1920x1080 --gpu-times 30 --preset high
cargo run --release -p genos-stress -- --size 1920x1080 --gpu-times 30 --preset high --denoise-launch screen
cargo run --release -p genos-stress -- --size 1920x1080 --gpu-times 30 --preset high --still-boxes --freeze-sun --dynamic 0
GENOS_GPU_DISPATCH=1 GENOS_LIGHTING_DEBUG=1 cargo run --release -p genos-lighting -- --scene sponza --checkpoint atrium --size 1920x1080 --frames 180 --preset high --gpu-times 30
GENOS_GPU_DISPATCH=1 GENOS_LIGHTING_DEBUG=1 cargo run --release -p genos-lighting -- --scene sponza --checkpoint colonnade --size 1920x1080 --frames 180 --preset high --gpu-times 30
GENOS_GPU_DISPATCH=1 GENOS_LIGHTING_DEBUG=1 cargo run --release -p genos-lighting -- --scene sponza --checkpoint curtains --size 1920x1080 --frames 180 --preset high --gpu-times 30
GENOS_GPU_DISPATCH=1 GENOS_LIGHTING_DEBUG=1 cargo run --release -p genos-lighting -- --scene sponza --checkpoint dark-corner --size 1920x1080 --frames 180 --preset high --gpu-times 30
GENOS_LIGHTING_DENOISE_LAUNCH=screen cargo run --release -p genos-lighting -- --scene sponza --checkpoint curtains --size 1920x1080 --frames 180 --preset high --gpu-times 30
```

`GENOS_LIGHTING_DEBUG` writes `./genos-debug/denoise/latest.txt` (`--out` puts that under the chosen directory). It reads the counters back, so it is not an fps run. The curtains checkpoint is the central banner.

`make stress` is `cargo run --release -p genos-stress` and does not forward these arguments, so the commands above call cargo directly. `GENOS_GPU_TIMES` and `GENOS_LIGHTING_PRESET` still apply when the matching argument was omitted. The `LIGHTING_MS` line names the passes and appends `cache_rays`, `shadows`, and `prefill`. `prefill` is 1 when the scout was due and 0 when it was not. The scout shares the cache-ray dispatch, so this is not a separate millisecond.

`hall` and `roomA` sit under the roof, so dawn, dusk, and night look alike there. `--view sky` stands in the south yard and looks up: the sky and the open ground show the curve. `--freeze-sun` holds the sun on that hour.

```
cargo run --release -p genos-stress -- --preset day-cycle --view sky --time-of-day 06:00 --freeze-sun
cargo run --release -p genos-stress -- --preset day-cycle --view sky --time-of-day 18:24 --freeze-sun
cargo run --release -p genos-stress -- --preset day-cycle --view sky --time-of-day 22:00 --freeze-sun
```

Lavapipe checks the look and the still-frame cost ratio. `presets_and_toggles_match_their_look` draws each preset and each feature off and rejects an unwritten frame, a saturated field, a flat-colour blowout, blue tiles, and white boxes. `world_cache_off_still_is_not_slower` holds a still camera until probe-only has settled, then checks that its lighting milliseconds are at most the surface cache's and that cache rays and hit lighting dispatched nothing.

`high`, `medium`, `low`, and `gi-1bounce` sample bounce hits (`lamp_pick=hits`). Potato and day-cycle stay off. `reuse (experimental)` is not a preset: one pixel shadow ray keeps or drops the whole unshadowed sum, and the 4070 picture left off (p99 26–36 moving, 9–17 still). A passed flag wins. Lavapipe checks each mode against off (`lamp_pick_each_mode_against_off`) and the shadow-ray counts. `GENOS_LIGHTING_LAMP_PICK_DIAG=1` appends the last frame's ray counts to `LIGHTING_MS`.

```
GENOS_GPU_TIMES=30 cargo run --release -p genos-stress -- --proof --no-panel --size 1920x1080 --frames 180 --lights 5 --dynamic 50 --gpu-times 30 --lamp-pick off
GENOS_GPU_TIMES=30 cargo run --release -p genos-stress -- --proof --no-panel --size 1920x1080 --frames 180 --lights 5 --dynamic 50 --gpu-times 30 --lamp-pick hits
GENOS_GPU_TIMES=30 cargo run --release -p genos-stress -- --proof --no-panel --size 1920x1080 --frames 180 --lights 5 --dynamic 50 --gpu-times 30 --lamp-pick reuse
GENOS_GPU_TIMES=30 cargo run --release -p genos-stress -- --proof --no-panel --size 1920x1080 --frames 180 --lights 100 --layout first --dynamic 50 --gpu-times 30 --lamp-pick off
GENOS_GPU_TIMES=30 cargo run --release -p genos-stress -- --proof --no-panel --size 1920x1080 --frames 180 --lights 100 --layout first --dynamic 50 --gpu-times 30 --lamp-pick hits
GENOS_GPU_TIMES=30 cargo run --release -p genos-stress -- --proof --no-panel --size 1920x1080 --frames 180 --lights 100 --layout first --dynamic 50 --gpu-times 30 --lamp-pick reuse
```

Still camera, same flags, with `--dynamic 0 --freeze-sun --still-boxes`. Optional ray counts: prefix `GENOS_LIGHTING_LAMP_PICK_DIAG=1`.
