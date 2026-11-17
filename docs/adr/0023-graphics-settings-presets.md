# 0023: Graphics settings come as four presets over one set of knobs

Status: **Accepted** (28 Oct 2026). Engine decided with Anthony's delegation on 28 Oct; Anthony will review later. Design: [`docs/systems/rendering/settings-presets.md`](../systems/rendering/settings-presets.md).

## Context

The renderer has many quality knobs (lighting, shadows, reflections, fog, textures) but no shipped grouping. Lighting VRAM today is about 175 MB at Low, 750 MB at Medium and 1 GB at High (1440p). ADR 0020 sets the floor: the engine must run at Low on a 4 GB GTX 1060. Ray tracing hardware is not universal, the Steam Deck is a target, and macOS runs through MoltenVK (ADR 0003) without hardware ray queries.

## Decision

1. Four presets: **Low, Medium, High, Ultra**. A preset is only a named table of knob values; every knob can still be overridden one by one, and the UI shows "Custom" once one is.
2. Low is tuned for a GTX 1060 6 GB / RX 580 at 1080p and the Steam Deck at 800p. Minimum spec is Vulkan 1.3 and 4 GB VRAM.
3. Ray tracing is optional. `VK_KHR_ray_query` is used when present; without it the same lighting runs on the software tracer (the macOS path), so no preset requires RT hardware.
4. Lighting has a soft VRAM budget of about 200 MB at Low. Other presets scale from measured numbers, and all memory shares the one `VK_EXT_memory_budget` budget (ADR 0014, ADR 0020).
5. Direct light is computed every frame on every preset. Low may add one optional knob, **bounce spread**: world-space bounce relight is spread over N frames and then swapped in with a short blend. It is off by default and never applies to direct light or to screen-space terms, so camera turns stay immediate.
6. Each preset has a frame-time and VRAM gate on its reference hardware, measured with the lighting oracle (`make lighting-oracle`). A knob value that misses its gate moves down a preset.
7. Startup picks a preset from the device (VRAM, RT support, known device IDs) and the player can change it.
8. Steam Deck targets 40 fps in the Deck's 40 Hz display mode (25 ms), with a hard floor of 30 fps (33.3 ms) at the worst moment, such as a light switching on. 40 Hz divides evenly, so frame pacing stays smooth, and it is clearly better than 30 fps for the same battery class; 30 fps as the floor keeps the light-switch gate meaningful.
9. Low upscales with FSR (spatial or temporal). It is vendor-neutral and runs on the GTX 1060, RX 580, Steam Deck and MoltenVK, which covers every Low target. DLSS and XeSS may be added later as optional extras on hardware that supports them.

## Rejected

- Requiring RT hardware for Medium and above (rules out macOS and many 4 GB cards).
- Separate code paths per preset (presets only change knob values, so the harness covers one renderer).
- Amortising direct light across frames (stale shadows and visible lag when a light switches on).

## Departures

- Bounce spread trades bounce latency for cost only on Low; it is off unless the frame budget needs it.
