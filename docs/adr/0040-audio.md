# 0040: Audio keeps a thin system device path and a lock-free mixer

Status: **Proposed** (10 Nov 2026). Anthony reviews before any code. Design: [`docs/systems/audio/plan.md`](../systems/audio/plan.md). Binds to [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md), [ADR 0009](0009-rhai-is-the-scripting-language.md), [ADR 0019](0019-agent-operable.md), [ADR 0020](0020-asset-pipeline-import-cook-stream.md), [ADR 0023](0023-graphics-settings-presets.md), [ADR 0035](0035-cooked-package-runtime-load.md). The job pool and the scene table are proposed as ADR 0031 and ADR 0032 on [PR #131](https://github.com/AnthonyPoschen/genos-engine/pull/131). The designs on master are [the job system](../systems/job-system.md) and [the scene ECS](../systems/ecs-scene.md). This number is 0040 so it does not take 0031 or 0032.

## Context

`genos-audio` already mixes loaded PCM to interleaved 16-bit stereo and can submit that buffer to ALSA, WASAPI, or CoreAudio. `mix` applies Doppler, constant-power pan, inverse distance, and direct-path transmission. `Renderer::transmission_gains` and `transmit.comp` compute the same transmission from the resident occluder list, up to 256 sources. The device function opens, writes, drains, and closes on the caller. There is no mixer thread, no bus, no voice cap, no paged clip, and no cooked audio record.

The runtime has to run on handhelds (Steam Deck and machines of that class), Windows, macOS, Linux, and consoles. Console SDKs are under NDA and cannot live in this repository. ADR 0007 already names the desktop libraries and rejects `cpal` and `rodio`. ADR 0023's floor is a GTX 1060-class machine at 1080p and a Steam Deck at 800p, whose display mode is 40 Hz. The cook writes `.gcook`, `.gtex`, and, on [PR #178](https://github.com/AnthonyPoschen/genos-engine/pull/178), `.gmat`, each with a SHA-256 trailer and a manifest token.

## Options

| Option | For | Against |
|---|---|---|
| `cpal` | Callback-shaped; WASAPI, CoreAudio, ALSA | ADR 0007 rejects it by name. It would own the device. No console path. Linux still has to pick among host APIs |
| miniaudio (or a binding) | One C file covers many desktop host APIs | A third-party library would own the device and a decoder. An NDA SDK cannot be split out without a fork. ADR 0007 |
| Console SDK as the common layer | Matches the machine the port ships on | The headers cannot be committed. Desktop CI would not build |
| **Thin device path in this repository** | Same libraries ADR 0007 already names. A trait is the console plug. The mixer is ours and lock-free | We keep the three desktop paths and the null path |

## Decision

1. The device path stays in `genos-audio`. Linux opens ALSA `default` (PipeWire on SteamOS and on Omarchy, the PulseAudio ALSA plugin where that is what `default` is, plain ALSA otherwise). Windows opens WASAPI shared mode, event-driven. macOS opens the default CoreAudio output. Exclusive mode is a setting and defaults off. A third-party crate does not own the device.
2. Consoles implement the same `DevicePath` trait and pass it to `start_with`. This repository ships `FakeDevice` and `NullDevice` only. No console SDK is named or fetched.
3. One dedicated thread, `genos-audio`, runs the callback. The job pool does not. The callback does not allocate, lock, read a file, call Vulkan, run Rhai, or query physics. Mixer commands arrive on one preallocated single-producer single-consumer ring from the frame thread. A published double buffer holds the voice and bus snapshot.
4. Voices, buses, and a submix tree live in tables sized by `AudioSettings`. Priority virtualizes voices above `audible_cap`. The float sum is in slot order so two runs match.
5. Spatialization keeps `space.rs`: radial Doppler at `SPEED_OF_SOUND`, constant-power pan, inverse distance, and `exp(-absorption * thickness)`. HRTF is a knob, default off on every preset. Occlusion reuses the CPU barrier walk and `transmit.comp`. It does not read the surface light cache. A physics ray cast is not a gate.
6. Cook writes a sibling `.gaud` (`GAU1`, SHA-256 trailer, manifest token `aud=`), the same shape as `.gmat` on PR #178. The runtime payload is PCM16 or IMA ADPCM. Vorbis and Opus, if accepted, are cook inputs only. Paged playback fills a ring from a Slack job. The callback reads samples that are already in memory.
7. Every number above is a field of `AudioSettings` and a row of `AudioPreset`. The unset preset is Low. Low is the Deck and the 1060-class machine. `mix()` stays bit-stable until a stage switches its caller.
8. Samples are not replicated. The server records a play message and opens no device.

## Latency and CPU floor

Unset and Low, 48 kHz, 256-frame period, two periods of device buffer (10.67 ms):

| Bound | Low, Deck and 1060-class | High |
|---|---|---|
| Mixer callback | ≤ 1.50 ms | ≤ 0.80 ms |
| Frame-thread audio update, inside the 6.0 ms Must ceiling | ≤ 0.40 ms | ≤ 0.50 ms |
| Output latency | ≤ 20 ms, ceiling 30 ms | ≤ 12 ms, ceiling 20 ms |
| Motion-to-sound (snapshot age plus device latency) | ≤ 40 ms, including a 25 ms Deck frame | ≤ 30 ms |

The callback sleeps on the device. It does not take a job-pool worker. CI checks the counters with an injected clock. Wall time is the owner's machine. Lavapipe times are not a budget.

## Observability and debug binary

`AudioSettings` is a public value with `to_text`, `parse`, and `set_key`. MCP, `POST /cmd`, and Rhai share `get_audio_settings`, `set_audio_settings`, `list_audio_presets`, `apply_audio_preset`, `get_audio_stats`, `play_audio`, `stop_audio`, and `get_voice`. `debug/audio` is stage A9, on the shared scaffolding, and writes `./genos-debug/audio/` (`--out` overrides; the process does not write under `$HOME`). `--check` plays a tone through `FakeDevice` and exits non-zero when the audible count stays 0. Planted mutations (`ignore_doppler`, `alloc_in_callback`, `lock_in_callback`, `unordered_voices`, `skip_transmission`) must fail their checks.

## Migration

No implementing commit until this ADR is accepted. One commit per stage. `mix_scene.rs` stays green. A7, A6, and A8 may proceed in parallel once their gates are green. A9 waits for A5 and A8.

| Stage | Gate | Change | Test |
|---|---|---|---|
| A1 | this ADR accepted | `AudioSettings`, presets. `mix()` unchanged | `audio_settings_round_trip` |
| A2 | A1 | `DevicePath`, fake and null devices, rings, `mix_into` | `callback_matches_mix_bytes`, `callback_does_not_allocate`, `a_full_ring_drops_and_counts` |
| A3 | A2 | Voices, buses, tree, virtual voices | `a_cycle_is_rejected`, `virtual_voice_keeps_its_cursor`, `submix_matches_golden` |
| A4 | A3 | Spatial snapshot. GPU path compared, not default on Low | `shipped_snapshot_matches_mix`, `gpu_and_cpu_transmission_agree` |
| A5 | A4 | Obstruction lowpass and closed-form HRTF, both default off | `obstruction_off_matches_broadband`, `hrtf_off_matches_stereo_pan` |
| A6 | A2 | Long-lived ALSA, WASAPI, CoreAudio. Linux CI uses the fake device | `fake_device_reports_the_period`, fault injection reopens off the callback |
| A7 | A1 | `.gaud`, `aud=`, PCM16 and IMA, paged ring | `two_cooks_match_bytes`, `a_bad_trailer_falls_back` |
| A8 | A3 | Listener and emitter wishes, play messages, Rhai | `wishes_reduce_in_handle_order`, `audio_replay_does_not_run_rhai`, `server_tick_opens_no_device` |
| A9 | A5, A8 | `debug/audio`, MCP stats, `mix.rhai`, mutations | `--check` exits 0; `ignore_doppler` fails the golden |

## Consequences

- Desktop playback stays on the libraries ADR 0007 already lists. The callback replaces open-write-drain-close.
- A console port links outside this repository.
- The runtime does not link an Opus or Vorbis decoder.
- The surface light cache stays a lighting structure.
- The audio record gains a pointer to this ADR. The shipped goal, intent, game use, and limits stay until the stage that changes them.

## Open questions

1. Confirm the thin device path, and that `cpal` and miniaudio stay rejected. A native PipeWire path would be a later ADR, and only after a measurement misses the latency ceiling.
2. Cook-only Vorbis and Opus decoders in `genos-cook`, with the runtime limited to PCM16 and IMA. Or WAV-only until those crates are accepted?
3. Closed-form HRTF behind a knob that defaults off, and a measured table only after you name a dataset and a license?
4. A reverb bus effect is not in A1–A9. In a follow-on, or out of this design?
5. The unset preset is Low, so a missing config stays inside the Deck and 1060-class ceilings. Confirm?
6. Stage A8 waits on PR #131 for world positions. Until then the camera loop publishes the listener. Confirm?
7. Occlusion reuses `transmission_gain` and `transmit.comp`. The surface light cache is not read. A physics ray cast is optional after that query exists and is not a gate. Confirm?
8. Samples stay off the wire. A predicted one-shot stops when the server rejects it. The mixer does not rewind. Confirm?
