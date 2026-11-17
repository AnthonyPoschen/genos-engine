# Graphics settings presets

Decision: [ADR 0023](../../adr/0023-graphics-settings-presets.md).

## Goal

Ship four presets (Low, Medium, High, Ultra) that each hit a frame budget and a VRAM budget on named hardware. A preset is a table of knob values, nothing more.

## Hardware targets

| Preset | Reference hardware | Resolution | Frame budget |
|---|---|---|---|
| Low | GTX 1060 6 GB, RX 580; Steam Deck | 1080p; 800p on Deck | 16.6 ms (60 fps); 25 ms on Deck (40 fps in 40 Hz mode; 33.3 ms / 30 fps hard floor) |
| Medium | RTX 2060 / RX 6600 class | 1440p | 16.6 ms |
| High | RTX 4070 class | 1440p | 8.3 ms (120 fps) |
| Ultra | RTX 4070 and above | 1440p–4K | 16.6 ms |

Minimum spec: Vulkan 1.3, 4 GB VRAM. Ray tracing (`VK_KHR_ray_query`) is optional on every preset. Without it, the software tracer does the same work (this is the macOS path through MoltenVK, ADR 0003).

## Lighting VRAM

| Preset | Current at 1440p | Target |
|---|---|---|
| Low | ~175 MB | ≤ ~200 MB (soft) |
| Medium | ~750 MB | measure, then set |
| High | ~1 GB | measure, then set |
| Ultra | — | measure, then set |

The 200 MB Low budget is soft: exceeding it logs a warning in the metrics line, not an error. All pools draw on the one `VK_EXT_memory_budget` budget (ADR 0014, ADR 0020).

## Knobs per preset

Starting values; each is confirmed by the gates below.

| Knob | Low | Medium | High | Ultra |
|---|---|---|---|---|
| Render scale | 0.67–0.75 + upscale | 0.85 | 1.0 | 1.0 |
| Direct light shadow rays per pixel | 1 | 1 | 2 | 2 |
| Direct light rate | every frame | every frame | every frame | every frame |
| Surface cache resolution | coarse | medium | fine | fine |
| Probe grid density | sparse | medium | dense | dense |
| Bounce relight rate | every frame, or spread over N (optional) | every frame | every frame | every frame |
| Bounce spread N | off by default; 2–4 when on | — | — | — |
| Ray tracing | software tracer; ray query if present | ray query if present | ray query if present | ray query if present |
| Reflections | screen space + probes | screen space + traced fallback | traced | traced, more rays |
| Shadow map size | 1024 | 2048 | 2048 | 4096 |
| Fog / volumetrics | low froxel count | medium | high | high |
| Texture mip bias / streaming pool | +1 mip, small pool | 0, medium | 0, large | 0, largest |
| Contact detail (screen space) | off | on | on | on |

### Bounce spread (Low only)

- Direct light is always computed every frame on every preset.
- When on, world-space bounce relight is split into N slices. Each frame relights one slice into a back snapshot; after N frames the snapshot is swapped in with a short blend so nothing pops.
- Screen-space terms (contact detail, specular, compose) stay per frame, so camera turns are immediate.
- It is off by default. It is for hardware that misses the Low budget with the full relight; if Low hits its budget without it, it stays off.
- Gate: the light-switch scene must reach the reference within N + blend frames with no pop the oracle can measure.

## Auto-detect

At first launch: Low if VRAM is under 6 GB or the device is a Steam Deck; Medium if no ray query or under 8 GB; High otherwise. Ultra is never picked automatically. The choice is written to settings and the player can change it.

## Overrides

Every knob can be set alone (settings file, console, Rhai, MCP per ADR 0019). After any override the preset name shows as Custom.

## Gates

Each preset, on its reference hardware, through `make lighting-oracle`:

1. Frame time within budget on the basic scene and the light-switch scene, with the light-switch frame within 10% of a steady frame.
2. Lighting VRAM within the table above.
3. `ref_err_near` and `grid_worst` no worse than the preset above it by more than an agreed margin.
4. No pops in the light-switch `frames_to_ref` run.

## Decisions (28 Oct 2026)

- Steam Deck: 40 fps target in the 40 Hz mode at 800p, 30 fps hard floor at the worst moment (light switch). 40 Hz paces evenly and looks much better than 30; the floor keeps the worst case gated.
- Low upscaler: FSR, because it is vendor-neutral and runs on every Low target (GTX 1060, RX 580, Deck, MoltenVK). DLSS and XeSS are optional extras on capable hardware later.

## Open questions


- Exact Medium/High/Ultra lighting VRAM targets after measuring.
