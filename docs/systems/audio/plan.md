# Audio (design)

Status: proposed, 10 Nov 2026. Owner: Engine. No code in this change. Anthony reviews [ADR 0040](../../adr/0040-audio.md) before any implementing commit.

The shipped mixer already plays several PCM clips into one stereo buffer, with Doppler, a left-right image, distance, and direct-path transmission. It does not yet run a callback, cap the voice count, page a long clip, or cook a clip. This design is that next system. It has to run on handhelds (Steam Deck and machines of that class), Windows, macOS, Linux, and consoles. Console SDKs stay out of this repository.

Binds to [ADR 0007](../../adr/0007-native-calls-go-to-the-system-and-vulkan.md), [ADR 0009](../../adr/0009-rhai-is-the-scripting-language.md), [ADR 0019](../../adr/0019-agent-operable.md), [ADR 0020](../../adr/0020-asset-pipeline-import-cook-stream.md), [ADR 0023](../../adr/0023-graphics-settings-presets.md), [ADR 0035](../../adr/0035-cooked-package-runtime-load.md), [the job system](../job-system.md), [the scene ECS](../ecs-scene.md), [the settings principles](../../engine/settings-principles.md), and [the agent rubric](../../agent-rubric.md). ADR 0031 and ADR 0032 are the proposed decision records for the job system and the scene ECS on [PR #131](https://github.com/AnthonyPoschen/genos-engine/pull/131). They are not on master. A name in [the physics plan](../physics/plan.md) wins over this file for the ray cast and for `NetId`.

## 1. Goal

A game plays music, UI, and world clips into one device stream. World clips keep airborne Doppler, a horizontal image, distance, and direct-path transmission. The callback that talks to the device does not allocate or lock. Voice count, bus count, period, and every other bound are preset knobs. A headless server records which world clips started and opens no device.

## 2. Where it sits today

| Piece | What it does now | Code |
|---|---|---|
| Mix | Allocates a `Vec<i16>` and sums clips into it | `mix` in `crates/audio/src/mix.rs` |
| Doppler, pan, distance, transmission | Radial Doppler at 343 m/s, constant-power pan, inverse distance from 1 m, `exp(-absorption * thickness)` | `crates/audio/src/space.rs` |
| Device | Opens, writes, drains, and closes. ALSA `default`, WASAPI, or CoreAudio, one OS each | `submit` in `crates/audio/src/device.rs` |
| GPU transmission | One compute dispatch, resident occluders, 256 sources, no swapchain image | `Renderer::transmission_gains`, `crates/render/shaders/transmit.comp` |
| PCM load | 16-bit WAV. Ogg, FLAC, and MP3 are not decoded | `crates/load/src/wav.rs`, [Loading](../loading.md) |
| Cook | `.gcook` (`GCK1`) is geometry. `.gtex` is textures. `.gmat` (`GMT1`) is the material record on [PR #178](https://github.com/AnthonyPoschen/genos-engine/pull/178). No audio record | `crates/cook/src/format.rs` |
| Consumer | `genos-audio-check` calls `mix`. It does not copy the mixer | `crates/audio-check` |
| Scheduler | The audio callback is named as a dedicated thread the pool does not run | [Job system](../job-system.md) §7 |
| Scene | Vecs of walls, solids, and lights. The ECS table is proposed | [Scene ECS](../ecs-scene.md) |

`crates/audio/tests/mix_scene.rs` already measures pitch, image, level, and transmission on a buffer. Those tests stay green. WASAPI and CoreAudio are not run from the Linux CI machine.

## 3. Prior art

- Bencina, "Real-time audio programming 101: time waits for nothing", 2011. <http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing>. The callback does not allocate, lock, or call anything that can wait.
- Lamport, "Specifying Concurrent Program Modules", ACM TOPLAS 5(2), 1983. <https://doi.org/10.1145/357195.357200>. A single-producer single-consumer ring. The frame thread produces mixer commands. `genos-audio` consumes them.
- Microsoft, WASAPI shared mode and the event callback. <https://learn.microsoft.com/en-us/windows/win32/coreaudio/wasapi>. Shared mode keeps the operating-system mixer. Exclusive mode bypasses it.
- Apple, Core Audio render callback, default output device. The callback renders into a buffer the unit provides.
- ALSA PCM, `snd_pcm_open` on `default`. PipeWire and PulseAudio both expose that name as an ALSA device. SteamOS 3 (Steam Deck) and Omarchy are PipeWire.
- Kuhn, "Model for the interaural time differences in the azimuthal plane", JASA 62(1), 1977. <https://doi.org/10.1121/1.381498>. Spherical-head interaural delay `ITD = (r/c) (θ + sin θ)`.
- IMA ADPCM, Interactive Multimedia Association. A step table and an index table. The decoder is small enough to live in this repository.
- FMOD virtual voices, as a description of priority and a cursor that keeps moving while a voice is silent. This design does not link FMOD, Wwise, or any other mixer library.
- ADR 0007 already rejects `cpal`, `rodio`, `winit`, and `tokio`. ADR 0020 already allows a cook-only crate (meshoptimizer) that the runtime does not link.

## 4. Device path

The word in this design is **device path**: the code that opens the operating-system audio device. It is not a [platform](../../../CONTEXT.md) and it is not a byte source.

| | `cpal` | miniaudio and its bindings | Console SDK as the common layer | Thin path in `genos-audio` |
|---|---|---|---|---|
| Who owns the device | The `cpal` crate | The C library | The vendor SDK | This repository, calling the system library |
| Linux | The crate picks a host API | ALSA, Pulse, JACK, and others compiled in | None | ALSA `default` only |
| Windows | WASAPI | WASAPI | None | WASAPI shared, event-driven |
| macOS | CoreAudio | CoreAudio | None | CoreAudio default output |
| Steam Deck | Whatever the crate picks | An extra host API beside the one ADR 0007 names | None | PipeWire's ALSA `default` |
| Consoles | Not shipped | Not shipped without a fork that would contain the SDK | The headers cannot be committed | A trait the port implements outside this tree |
| ADR 0007 | Rejected by name | A third-party crate would own the device | The SDK is not a system library of the desktop | The helper lives here, which is the rule when `std` is not enough |

**Recommendation.** Keep the thin path. ADR 0007 is unchanged.

Linux calls `snd_pcm_open` on the setting `device` when it is non-empty, and on `default` when it is empty. That one name is the Steam Deck, Omarchy, PulseAudio's ALSA device, and plain ALSA. The path does not link `libpulse` or `libpipewire`. A native PipeWire path is a later ADR, and only after a measurement misses the latency ceiling in §11.

Windows uses WASAPI shared mode with an event callback. `exclusive` defaults false. When it is set and the exclusive open fails, the path opens shared and increments `audio.exclusive_fallbacks`. macOS uses a HAL output render callback on the default device. A default-device change is observed on the frame thread, which reopens. The callback does not register that listener.

The callback may write to the device and may call that device's own xrun recovery. Close-and-open stays on the frame thread. On a fault (ALSA `-EPIPE` that recovery did not clear, a WASAPI device invalidated, a CoreAudio device that is no longer alive) the callback fills silence, increments `audio.device_faults`, and returns. The frame thread reopens on the next pump.

`DevicePath` is the trait. `open` returns the rate, the period, and the channel count the device accepted. `start` is given the mixer and calls `render` from `genos-audio`. `stop` joins that thread. `FakeDevice` and `NullDevice` implement it in this crate. Desktop paths are compiled for their own OS, as `submit` is today. A console port constructs its own implementor and passes it to `start_with`. Cargo.toml of this repository does not name a console crate. CI does not fetch one.

`submit` stays until A6, so `crates/audio/examples/submit.rs` keeps working. A6 makes `submit` a one-shot wrapper around the long-lived path. The callback path is the one the game uses.

The mixer asks for `sample_rate` (48000) and stereo. If the device accepts another rate, the mixer resamples and increments `audio.rate_mismatches`. If the device is mono, the device path downmixes `(L+R)/2` after the mixer and increments `audio.downmix`. `layout = surround_5_1` is the only request for more than two channels, and every preset leaves it at `stereo`.

## 5. The callback

The thread is `genos-audio`. It is not a `genos-job-*` worker. The pool does not run it, which is the line already in the job-system design.

Inside `render`:

- No heap allocation and no grow of a buffer.
- No mutex, rwlock, condvar, or park.
- No file read, no Vulkan call, no Rhai, no physics query, no `command::take`.
- Scratch, voice state, and bus buffers were allocated before `start` returned.
- The only system calls are the device write and that device's xrun recovery.

The frame thread is the only producer of mixer commands. MCP, Rhai, and jobs record a wish. The frame thread encodes the wish. Jobs do not push the ring. That is the same rule the job system uses for `command::call`.

The ring is Lamport's single-producer single-consumer queue. Capacity is `command_queue_cap`, rounded up to a power of two (Low default 256). Head and tail are atomics on separate cache lines. The storage is `[MixerCommand; CAP]`, allocated at start. A full ring drops the command, increments `audio.command_drops`, and the next drain reports the drop to the caller. The callback does not grow the ring.

A second ring carries completions the other way: voice ended, underrun, device fault. The frame thread drains it where it already drains MCP commands.

The voice and bus snapshot is a double buffer. The frame thread writes the inactive copy, then release-stores the index. The callback acquire-loads the index once and uses that copy for the whole callback. It never waits. A late writer leaves the previous copy in place, and `audio.snapshot_age_frames` records it. This is not a seqlock: the callback does not spin and does not read a torn row.

`mix()` stays the allocating offline function. `mix_into` writes a caller-provided buffer and is what the callback uses. A2's test renders the same inputs through both and compares samples.

**Departure from Bencina.** The no-allocation and no-lock rules are his. Reopen is pulled out of the callback onto the frame thread, so a device change cannot allocate inside `render`. Xrun recovery that the device documents as part of the write stays in the callback.

## 6. Voices, buses, and limiting

Tables are fixed. The lengths are settings. A play that does not fit a table fails the command and increments a counter. It does not allocate a row.

A **voice** is one playing clip. It holds a generation, a clip id, a cursor in frames, a pitch ratio, a gain, a bus index, a priority (`u8`), looping, positional, an optional scene handle, and the last published pose. Cursor state that the filter needs lives on the audio side of the double buffer so a parameter change does not reset it.

A **bus** is a named mix point with a gain, a parent index, and a one-pole lowpass cutoff (0 means off). Bus 0 is Master. The preset buses are Master, Music, Ui, Sfx, World, and a send bus whose effect is identity until a later reverb decision. A **submix** is a bus whose parent is not Master. Depth is capped by `bus_depth_cap` (default 4). A parent cycle is rejected at the write, the previous parent stays, and `audio.cycle_rejects` increments. Publish walks leaves toward Master so each bus is summed once.

The callback adds audible voices into their bus buffers in slot order, then adds each bus into its parent. Slot order is the determinism rule. Score order is not.

Limiting runs on the frame thread, before the snapshot flips:

1. Priority 255 is unkillable (the preset uses it for Music and Ui).
2. Every other voice is ordered by priority, then by distance gain, then by an older generation first.
3. The first `audible_cap` of that order are audible. The rest are virtual.
4. A virtual voice advances its cursor by `frames * pitch` and reads no samples. A one-shot can end while virtual. A loop stays in phase.
5. A new play that does not fit steals the lowest scored killable audible voice by making it virtual. Equal priority does not steal. A play that cannot steal starts virtual, and `audio.virtual_starts` increments. `audio.steals` counts steals.

Gain and Doppler ratio slew on the audio thread (`gain_smooth_ms`, `doppler_smooth_ms`). The offline golden sets both to 0 when it wants the unslewed `space.rs` numbers. Low sets `doppler_smooth_ms` to 20.

## 7. Spatialization

The shipped functions stay the authority for the default curve. `AudioSettings::shipped()` copies them: `SPEED_OF_SOUND` (343), Doppler clamp 0.99, reference distance 1 m, rolloff 1, no far clamp, linear interpolation, constant-power pan, broadband transmission. `mix()` keeps calling those functions. The snapshot path calls the same functions with the settings substituted in, so a test can show `shipped()` matches `mix()` sample for sample.

Doppler uses the radial component only, as `doppler_ratio` does today. The frame thread publishes the ratio. The callback applies it as the resample step and does not read positions.

Distance gain is `reference_distance / max(distance, reference_distance) ^ rolloff` out to `max_distance`. `max_distance = 0` means no far clamp, which is today's curve. Past a finite max, gain is `min_gain`. Low sets `max_distance` to 40 m and `min_gain` to 0. The existing tests place sources at 8 m, so they stay on the old curve either way.

Pan in `spatial = stereo` is `pan_gains`: constant power on the horizontal angle, including a source behind the listener. Elevation is ignored. That is the shipped image.

`spatial = hrtf` is optional and **off on every preset**, including High and Ultra. With no measured table loaded, the callback uses Kuhn's spherical-head delay for the azimuth and a one-pole shelf for elevation, and it keeps the constant-power energy. Head radius is `head_radius_m` (default 0.0875). The shelf is ours; Kuhn's paper is the delay only. A cooked table replaces the closed form when `hrtf_asset` is set. The callback convolves with `hrtf_taps` (default 32) from a history allocated at start. If that voice would push the callback over `mix_budget_ms`, that voice uses stereo pan for the callback and `audio.hrtf_fallbacks` increments.

**Departure from Kuhn.** Azimuth delay only. No frequency-dependent delay, no pinna database, no elevation in the delay. The shelf is a separate one-pole and is named here so it is not mistaken for the paper.

### Occlusion and obstruction

Transmission stays `exp(-absorption * thickness)` along the straight segment, the product over the boxes, which is `transmission_gain` and `transmit.comp`. Absorption remains the single number already stored on the occluder (`albedo.a` in the shader, `Barrier.absorption` on the CPU). This design does not add a per-band material.

The frame thread publishes one transmission float per audible voice. The callback multiplies by it. The callback does not walk boxes and does not dispatch.

`obstruction = off` is broadband only, which is today. `obstruction = on` also sets a one-pole cutoff `obstruction_hz * transmission`, clamped to `[obstruction_min_hz, rate/2]`. Low leaves the knob off. Medium and above turn it on. The state of the pole lives on the audio thread.

`occlusion_path` is `cpu` or `gpu`. Low and Medium set `cpu`. High and Ultra set `gpu`, and the frame thread falls back to `cpu` when no renderer is open. The GPU path is the existing dispatch, still capped at 256 sources, still not a present. Queries per frame are `occlusion_queries_per_frame`. A voice that was not queried keeps its last float. `audio.occlusion_age_frames` is the oldest kept value.

The physics plan's ray cast (step P13) is not built. It is not a gate. When it exists, the frame thread may fill the same transmission float for a dynamic body the occluder list does not hold. The callback still only sees the float.

The surface light cache is the wrong structure. The glossary defines it as hashed surface patches of incoming light. It is not geometry and not a portal graph, and it goes stale for lighting reasons. `transmit.comp` already walks the resident occluder list. That list is the reuse. The cache is not read.

Air absorption, diffraction, and geometric reflections are out of this design. The current [audio record](../audio.md) already lists them as absent. A reverb effect on the send bus is the open question in §16, and A1–A9 do not depend on it. The send bus exists so a later effect has a parent.

## 8. Resident clips, paged clips, and the cook

A **resident clip** is in memory before it plays. A **paged clip** is the streaming case: the file stays on disk, a Slack job fills the next page, and the callback reads the ring. That paging is playback. It is not a byte source. The setting keys keep the `stream_` prefix for this case.

ADR 0020's rule is unchanged: the runtime does not transcode a source file. [PR #178](https://github.com/AnthonyPoschen/genos-engine/pull/178) is the pattern for a sibling record: its own magic, version 1, a SHA-256 trailer, a manifest token, and verify-then-fallback. Audio follows that pattern. It does not grow a section inside `GCK1`.

The record is `.gaud`, magic `GAU1`, little-endian:

- `u32` version, value 1
- asset id, 32 bytes
- cook settings hash, 32 bytes
- `u32` class: 1 resident PCM16, 2 resident IMA, 3 paged IMA
- `u32` sample rate, `u16` channels (1 or 2), `u16` reserved 0
- `u64` frame count
- `u32` page frames (0 when resident)
- payload
- SHA-256 of every preceding byte

The package line gains ` aud=<sha256>` after the tokens that line already has (`cook=`, `gtex=`, `mat=`). A line without `aud=` is unchanged. On a bad trailer, a missing file, or a hash mismatch, the load logs `assets fallback <asset> <reason>` and uses WAV PCM when the source WAV is still there. When it is not, the clip is absent, `audio.missing_assets` increments, and the frame does not fail. That is ADR 0035's fallback rule.

Cook inputs:

| Input | This design |
|---|---|
| WAV PCM16 | Decoded by `genos-load`, which already does. No new crate |
| Ogg Vorbis | Cook input only, if the open question accepts a cook-only decoder |
| Opus | Cook input only, same condition |
| FLAC, MP3, MP4, EBML | Identified and kept as notes, which is asset-plan step 5. Not decoded here |

A cook-only decoder links `genos-cook` and does not link `genos-audio`, `genos-load`, or the device path. That is the meshoptimizer departure in ADR 0020, named again in the implementing commit. If the question is refused, A7 still ships and accepts WAV only. The `GAU1` layout does not change.

Class is chosen from the settings, not from a vendor id. A clip whose PCM is within both `resident_max_ms` (4000) and `resident_max_bytes` (256 KiB on Low) is resident PCM16. Anything larger is IMA. IMA that fits the resident bank is class 2. The rest is class 3. The bank cap is `resident_bank_bytes` (8 MiB on Low).

Resident IMA stays compressed. The callback decodes one period with the published step and index tables straight into the bus buffer. That decode allocates nothing and is covered by `callback_does_not_allocate`. Paged IMA is decoded by a Slack job into a preallocated f32 ring of `stream_page_frames` (4096). The callback copies from the ring. Disk reads stay on the Slack job. Low's Slack ceiling for that fill is 0.50 ms. Two pages in flight per ring, `stream_rings` of them (2 on Low).

The asset plan's sentence that the audio subset hash is the WAV PCM changes in the A7 commit: the hash is the SHA-256 of the `.gaud` body, trailer excluded. The sim hash does not include it. Two cooks of the same source and the same cook settings write the same bytes.

`LoadProfile::Sim` does not open a device and does not decode pages. The server may read the header (rate, channels, frame count, hash) so a play message can name the asset.

`genos-load` stays decode-only for WAV. It does not open a device. IMA decode for playback lives in `genos-audio`. The cook writer lives in `genos-cook`.

## 9. ECS, jobs, and Rhai

The scene ECS is proposed and is not on master. Until its hierarchy stage has landed, the camera loop publishes a listener pose and a list of sources, which is what a caller of `mix` does today. A8 adds components when the table exists and keeps the pose list as the fallback.

`Listener` is at most one. Its position is the world position. Its velocity is the physics body's when a `BodyId` is present, otherwise the frame-to-frame difference of the position. That difference is not part of a network checksum.

`Emitter` holds the clip asset hash, bus, gain, pitch, looping, positional, and priority. The sample cursor is audio-thread state, not a component.

The audio update is a serial Must stage, label `audio.update`, after physics and after the hierarchy stage, before the draw. It reduces wishes in scene-handle order, writes the inactive snapshot, flips the index, and pushes the ring. It does not run on a worker. A parallel stage that wants a sound writes an `AudioWish` slot. The reduce is the serial order. Two runs with the same wishes produce the same mixer-command bytes.

Rhai, under ADR 0009, calls registered functions: `play(path, x, y, z)`, `play_ui(path)`, `stop(voice)`, `listener(x, y, z, yaw)`. A script does not receive the device, the ring, or a voice pointer. The functions run in the Commands stage. A physics rollback does not call them. The job-system design already keeps Rhai off a worker and out of a replay.

`audio.update` counts inside the job system's 6.0 ms Must ceiling. It does not raise that ceiling. Its own ceiling is 0.40 ms on Low.

## 10. Networking

`docs/systems/core-networking/plan.md` is not in the tree. The names below are the ones the physics plan assigns to that file. A name there wins when it lands. This crate does not open a socket.

The implementing commit that first plays a world clip through the new path copies these five lines into the [audio record](../audio.md):

- Replicated state: a play message `{ tick, play_id, asset hash, emitter NetId or a millimetre position, bus, gain_milli, pitch_milli, priority, looping, positional, predicted, seed }`. A stop message is `{ tick, play_id }`. PCM, the cursor, the voice slot, Doppler, pan, and transmission stay local. Music and UI plays are local to the machine that handled the input. World one-shots the server starts are the replicated ones.
- Authority: the server decides which world clips exist. An owning client may start a clip its own input caused (a footstep) and stops it if the server does not confirm within `predict_sounds_ticks` (default 10). Other clients start the clip when the message arrives, at cursor 0.
- Prediction / rollback: audio is not in the physics checksum and is not rolled back. The mixer does not rewind samples. A rejected prediction increments `audio.rejected_plays` and stops that voice. The pose of a playing emitter follows the position the client is already showing.
- Determinism: the same snapshot, the same PCM, and smooth times of 0 produce the same f32 samples in one process. The i16 conversion is the same round-and-clamp `mix` uses today. Samples are not on the wire, so cross-machine float identity of the mix is not required. The play-message list in tick order is the record two peers compare. The callback's wall clock is not an input.
- Headless server: no device, no `genos-audio` thread, no transmission dispatch. `tick_server` turns wishes into play messages. `LoadProfile::Sim` does not decode pages.

`seed` is 0 in this design. It is on the message so a later variation does not grow a second record.

## 11. Presets and budgets

Nothing in the mixer is a bare literal. The preset table is the value. A stage that adds a knob adds it to `AudioSettings`, to `to_text` / `parse` / `set_key`, to this table, and to a round-trip test. Startup may pick Low on a Steam Deck. The pick writes the preset. The mixer does not branch on a vendor or a device id. That is ADR 0023.

The unset preset is Low. `AudioSettings::default()` is `preset(Low)`. `AudioSettings::shipped()` is the curve in `space.rs` and is what the old tests lock.

Low is tuned for a 4-core CPU of the GTX 1060's era and for the Steam Deck (4 Zen 2 cores, 8 threads, 800p, 40 Hz display). The audio thread sleeps on the device and shares a core with the frame thread or a job worker. It does not add a fourth job worker. The 1060's GPU is not in the mix budget. Low sets `occlusion_path` to `cpu` so the frame does not gain a fence wait.

Period 256 at 48 kHz is 5.33 ms. Two periods of device buffer is 10.67 ms. A Deck frame at 25 ms plus that buffer is 35.7 ms, inside the 40 ms motion bound. A 512-frame period would miss that bound, so Low does not use it.

| Knob | Low (unset) | Medium | High | Ultra |
|---|---|---|---|---|
| `period_frames` | 256 | 256 | 128 | 128 |
| `buffer_periods` | 2 | 2 | 2 | 2 |
| `sample_rate` | 48000 | 48000 | 48000 | 48000 |
| `layout` | stereo | stereo | stereo | stereo |
| `spatial` | stereo | stereo | stereo | stereo |
| `voice_cap` | 32 | 64 | 128 | 256 |
| `audible_cap` | 16 | 32 | 64 | 128 |
| `bus_cap` | 8 | 16 | 32 | 32 |
| `occlusion_path` | cpu | cpu | gpu | gpu |
| `occlusion_queries_per_frame` | 8 | 16 | 32 | 64 |
| `obstruction` | off | on | on | on |
| `max_distance` | 40 | 80 | 0 | 0 |
| `doppler_smooth_ms` | 20 | 20 | 10 | 10 |
| `resident_max_bytes` | 262144 | 524288 | 1048576 | 1048576 |
| `resident_bank_bytes` | 8388608 | 16777216 | 33554432 | 50331648 |
| `stream_rings` | 2 | 4 | 8 | 8 |
| `mix_budget_ms` | 1.50 | 1.50 | 0.80 | 0.50 |
| `update_budget_ms` | 0.40 | 0.40 | 0.50 | 0.50 |
| `stream_budget_ms` | 0.50 | 0.50 | 0.80 | 0.80 |
| `latency_target_ms` | 20 | 20 | 12 | 12 |
| `latency_ceiling_ms` | 30 | 30 | 20 | 20 |
| `motion_target_ms` | 40 | 40 | 30 | 25 |

Knobs that are the same on every preset: `reference_distance` 1, `rolloff` 1, `min_gain` 0, `resample` linear, `speed_of_sound` 343, `doppler_limit` 0.99, `bus_depth_cap` 4, `command_queue_cap` 256, `gain_smooth_ms` 10, `obstruction_hz` 4000, `obstruction_min_hz` 400, `hrtf_taps` 32, `head_radius_m` 0.0875, `resident_max_ms` 4000, `stream_page_frames` 4096, `mix_budget_fraction` 0.25, `predict_sounds_ticks` 10, `exclusive` false, `device` empty. When the device accepts the requested period, the mix budget is `mix_budget_ms`. When the device forces a shorter period, the budget is `min(mix_budget_ms, mix_budget_fraction * actual_period_ms)`.

`reverb` is not a preset row. The send bus is identity. §16 asks whether a later stage adds an effect.

CI does not assert wall-clock milliseconds. Tests inject a clock. A zero-cost clock expects `audio.over_budget == 0`. A clock that charges more than the budget expects the counter to rise and the buffer to still be filled. The owner's machine runs `debug/audio --preset low --check` and reads `audio.mix_ms`. A value over the ceiling moves the preset knob down. It does not add a device-id branch.

## 12. Settings surfaces

`AudioSettings` lives in `genos-audio` and implements the three methods from [the settings principles](../../engine/settings-principles.md): `to_text`, `parse`, `set_key`. `genos-scene` does not depend on `genos-audio`, and this design does not add that dependency. The debug tools hold the value.

Arguments on `debug/audio`: `--audio-preset`, `--audio-period`, `--audio-device`, `--out`. The argument wins. `GENOS_AUDIO_PRESET`, `GENOS_AUDIO_PERIOD`, and `GENOS_AUDIO_DEVICE` apply only when that argument was omitted. The implementing commit registers them in `lint/env_mirrors.toml`.

MCP and the debug menu edit the same keys. The menu page is a slider for each ordered knob and a dropdown for preset, layout, spatial, and occlusion path. A game may map its own player-facing names onto the struct, or show the keys. The engine owns the struct.

Output follows the Output rule. `debug/audio` writes `./genos-debug/audio/`. `--out` overrides. The process does not choose `$HOME`.

## 13. Verification

Offline render. `mix_into` and the snapshot renderer do not open a device. `crates/audio/tests/mix_scene.rs` stays the regression for `mix()`. New fixtures live in `crates/audio/tests/fixtures/`. `golden_shipped.i16` is the first 1024 stereo frames of `shipped()` and must match bitwise on the CI toolchain, including the existing round-and-clamp. `golden_submix.i16` is two voices into a bus at gain 0.5. Two runs in one process match bitwise when the smooth times are 0. `genos-audio-check` gains one call to `mix_into` so a second crate still does not copy the mixer.

Glitches. `audio.underruns` counts a device underrun. `audio.glitches` counts a callback that the injected clock finished after the period. `audio.over_budget` counts a callback over the mix budget. `audio.command_drops`, `audio.device_faults`, `audio.device_resets`, and `audio.late_snapshots` are the rest. The fake device raises each one on request. The buffer is still written when the budget is missed. The callback does not drop the period.

Latency. The fake device's `audio.output_latency_ms` equals `buffer_periods * period_frames / rate * 1000`. `audio.motion_to_sound_ms` is that plus the snapshot age. A unit test locks the sum. On a machine with a device, `debug/audio --latency` also reads ALSA `snd_pcm_delay`, the WASAPI clock and padding, or the CoreAudio device latency and the buffer latency, and writes the same fields. Linux CI does not open ALSA.

MCP. `get_audio_stats` returns `audio.mix_ms`, `audio.update_ms`, `audio.voices_audible`, `audio.voices_virtual`, `audio.voices_ended`, `audio.streams`, `audio.device_rate`, `audio.period_frames`, `audio.device_path` (`alsa`, `wasapi`, `coreaudio`, `fake`, or `null`), and every counter in the failure table. `debug/audio --check` plays a tone through `FakeDevice`, requires `voices_audible == 1`, stops it, and requires `voices_ended == 1`. A build that ignores play leaves the counts at 0 and exits non-zero.

Rhai. Stage A9 adds `debug/audio/scripts/mix.rhai`. It loads the shared room, plays a resident tone, steps 30 fixed frames, requires underruns and glitches at 0 and one audible voice, moves the source behind the wall, requires that voice's transmission under 0.5, stops, and requires zero audible voices. `make verify` is not changed until that script exists. Earlier stages are `cargo test -p genos-audio`.

`debug/audio` uses the shared scaffolding: scene picker, checkpoints, reset, replay of the mixer-command list, sliders, a guided-review panel, and a headless drive. The capture is `waveform.i16` and `metrics.json`, not a picture. Replay compares the command list and the golden hash.

Planted mutations, `--mutate <name>`. The harness fails while the mutation is on.

| Mutation | What it breaks | The check that fails |
|---|---|---|
| `ignore_doppler` | The ratio stays 1 | `shipped_snapshot_matches_mix` |
| `center_pan` | Both channels stay equal for a side source | the pan assertion in `mix_scene.rs` |
| `skip_transmission` | The product stays 1 through a solid | `gpu_and_cpu_transmission_agree` and the wall case |
| `alloc_in_callback` | `render` allocates | `callback_does_not_allocate` |
| `lock_in_callback` | `render` takes a mutex | the test times out, `audio.glitches` rises |
| `unordered_voices` | The sum follows score order | `submix_matches_golden` |
| `server_opens_device` | `tick_server` starts `genos-audio` | `server_tick_opens_no_device` |

## 14. Stages and gates

Anthony accepts ADR 0040 before A1. Each stage is one commit, keeps the workspace tests for the crates it touches green, and names its test in the commit message. A red gate stops the next stage. A7 may start after A1. A6 may start after A2. A8 may start after A3. A9 waits until A5 and A8 are green.

| Stage | Gate | Deliverable | Test |
|---|---|---|---|
| A1 | ADR 0040 accepted | `AudioSettings`, `preset`, `shipped`. `mix()` unchanged | `audio_settings_round_trip` |
| A2 | A1 | `DevicePath`, `FakeDevice`, `NullDevice`, both rings, `mix_into` | `callback_matches_mix_bytes`, `callback_does_not_allocate`, `a_full_ring_drops_and_counts` |
| A3 | A2 | Voices, buses, tree, virtual voices, unkillable priority | `a_cycle_is_rejected`, `virtual_voice_keeps_its_cursor`, `submix_matches_golden` |
| A4 | A3 | Spatial snapshot from the settings. GPU transmission compared on lavapipe, default `cpu` on Low | `shipped_snapshot_matches_mix`, `gpu_and_cpu_transmission_agree` |
| A5 | A4 | Obstruction lowpass and closed-form HRTF. Both default off, and off matches A4 | `obstruction_off_matches_broadband`, `hrtf_off_matches_stereo_pan` |
| A6 | A2 | Long-lived ALSA, WASAPI, and CoreAudio. Reopen on the frame thread. Linux CI stays on `FakeDevice` | `fake_device_reports_the_period`; a fake fault increments `audio.device_resets` and the next callback is silence then audio |
| A7 | A1 | `GAU1`, manifest `aud=`, PCM16, IMA, resident and paged. Cook-only Vorbis and Opus only if §16.2 is accepted | `two_cooks_match_bytes`, `a_bad_trailer_falls_back`, `stream_fill_is_not_on_the_callback` |
| A8 | A3 | Wishes, serial `audio.update`, Rhai play and stop, `tick_server` | `wishes_reduce_in_handle_order`, `audio_replay_does_not_run_rhai`, `server_tick_opens_no_device` |
| A9 | A5 and A8 | `debug/audio`, MCP, `mix.rhai`, mutations | `--check` exits 0; `ignore_doppler` fails the golden |

A4's lavapipe test compares CPU and GPU transmission floats for the same boxes. It does not use lavapipe time as a budget. A6 does not make Linux CI open a sound device. A machine with ALSA runs `debug/audio --device default --frames 32` and reads `audio.device_path == alsa`.

This proposal adds a Decisions pointer in `docs/systems/audio.md` and leaves the shipped goal, intent, game use, and limits as they are. The stage that changes one of those four edits the record in that commit.

The implementing commit that introduces the public words adds them to `CONTEXT.md`: voice, bus, submix, device path, resident clip, paged clip. This design uses those words. It does not edit `CONTEXT.md`.

## 15. Failure modes and probes

| Failure | Probe |
|---|---|
| Callback allocated or locked | `callback_does_not_allocate`; `audio.glitches` |
| Ring full | `audio.command_drops` |
| Voice stolen or started silent | `audio.steals`, `audio.virtual_starts`, `get_voice` |
| Bus cycle written | `audio.cycle_rejects` |
| Mix over the budget | `audio.over_budget`, `audio.mix_ms` |
| Device underrun | `audio.underruns` |
| Device disappeared | `audio.device_faults`, `audio.device_resets` |
| Snapshot a frame late | `audio.snapshot_age_frames`, `audio.late_snapshots`, `audio.motion_to_sound_ms` |
| Output latency over the ceiling | `audio.output_latency_ms`, `audio.latency_over` |
| Frame update over 0.40 ms on Low | `audio.update_ms` inside the Must label |
| Transmission skipped | the wall golden; `audio.occlusion_age_frames` |
| HRTF over budget | `audio.hrtf_fallbacks` |
| Cook hash mismatch | `assets fallback`, `audio.missing_assets` |
| Server opened a device | `server_tick_opens_no_device`; `audio.device_path` stays unset |
| Predicted play rejected | `audio.rejected_plays` |

## 16. Decisions for Anthony

1. Thin device path on ALSA `default`, WASAPI shared, and CoreAudio. `cpal` and miniaudio stay rejected under ADR 0007. Consoles pass a `DevicePath` into `start_with`. A native PipeWire path waits for a missed latency ceiling.
2. Cook-only Vorbis and Opus, or WAV-only. The runtime payload is PCM16 and IMA either way.
3. Closed-form HRTF behind `spatial`, default off on every preset. A measured table waits for a dataset and a license.
4. Reverb on the send bus is out of A1–A9. Say if a follow-on should add it.
5. The unset preset is Low.
6. A8 publishes poses from the camera loop until PR #131's hierarchy is on master.
7. Occlusion reuses the barrier walk and `transmit.comp`. The surface light cache is not read. The physics ray cast is not a gate.
8. Samples stay off the wire. A rejected prediction stops the voice and does not rewind it.

## References

- Bencina, 2011. <http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing>
- Lamport, ACM TOPLAS 1983. <https://doi.org/10.1145/357195.357200>
- Kuhn, JASA 1977. <https://doi.org/10.1121/1.381498>
- WASAPI. <https://learn.microsoft.com/en-us/windows/win32/coreaudio/wasapi>
- ADR 0007, ADR 0020, ADR 0023, ADR 0035
- [Job system](../job-system.md), [scene ECS](../ecs-scene.md), [PR #131](https://github.com/AnthonyPoschen/genos-engine/pull/131)
- [Asset plan](../assets/plan.md), [PR #178](https://github.com/AnthonyPoschen/genos-engine/pull/178)
