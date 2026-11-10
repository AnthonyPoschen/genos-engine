# Audio design

Status: **proposed**. ADR: [0040](../adr/0040-one-audio-mix-device-outputs-only.md). The code that already runs is the [audio record](audio.md). Words added for this plan are in `CONTEXT.md`.

The mix in `genos-audio` is the one implementation of Doppler, pan, distance, transmission, buses, and reverb. ALSA, WASAPI, CoreAudio, and a later console write only receive the buffer that mix produced. A fixed tape rendered by `mix` hashes to the same bytes on every platform that runs `cargo test`.

## 1. What already exists

| Piece | What it does | Where |
|---|---|---|
| `mix` | Allocates an interleaved `i16` stereo buffer and adds every `Source` into it. A non-positional source is copied. A positional source takes Doppler, pan, distance, and transmission for the whole buffer. Linear interpolation. One-shot stops at the last sample. A loop wraps. | `crates/audio/src/mix.rs` |
| `Listener`, `Source`, `Barrier` | Pose, clip, and one AABB with absorption in nepers per meter. `Source::transmission` replaces the barrier walk when the GPU already computed the gain. | `mix.rs` |
| `space` | Speed of sound 343 m/s. Doppler uses only the closing component. Pan is constant-power on the horizontal bearing, including sources behind the listener. Distance is `1 / max(distance, 1 m)`. Transmission is the product of `exp(-absorption * thickness)`. | `crates/audio/src/space.rs` |
| `submit` | Writes that stereo buffer and returns the frame count. Linux opens ALSA `default`, asks for 200 ms, writes, drains, and closes. Windows creates a shared-mode WASAPI client, asks for at least one second, writes, waits until the buffer is empty, and releases the client. macOS creates an AudioQueue, waits, and disposes it. Any other OS returns an error. | `crates/audio/src/device.rs` |
| GPU transmission | One compute dispatch on the renderer's device. Reads the resident occluder buffer the light pass already keeps. Absorption is `occ.albedo.a`. Cap is 256 sources. The call waits on a fence and returns `Vec<f32>`. It does not present. | `Renderer::transmission_gains`, `crates/render/shaders/transmit.comp` |
| CPU tests | Pitch rises when the source or the listener approaches, a wall cuts the level, a loop continues. Relationships, not a hash. | `crates/audio/tests/mix_scene.rs` |
| Second package | Calls the public `mix`. Does not copy the mixer. | `crates/audio-check/tests/consumer.rs` |
| One-shot example | Builds a tone, calls `mix`, calls `submit`. | `crates/audio/examples/submit.rs` |
| Cook | The asset plan stores WAV PCM as the audio subset bytes. `genos-load` decodes 16-bit WAV. OGG, MP3, and FLAC are identified and not decoded. | [asset plan](assets/plan.md), `crates/load/src/wav.rs` |
| Loader rule | Decoders do not open a sound device. | [Loading](loading.md) |

`genos-audio` depends on `genos-load` and `genos-math`. It does not depend on the window, the renderer, or the scene. The camera scene does not open a sound device. There is no `debug/audio`.

## 2. What stays

These already match the requirement that one mix run everywhere and that a platform only write the buffer.

- `mix` and `space` stay the DSP. Buses, smoothing, and reverb call into this crate. They do not get a second implementation beside it.
- `Listener`, `Source`, and `Barrier` stay the types a game fills. `positional: false` stays the dry path for music and UI.
- ALSA, WASAPI, and CoreAudio stay the three device writes. The modules stay `cfg`'d to their OS. They include no pan, no Doppler, and no reverb.
- The transmission rule stays `exp(-absorption * thickness)` on the straight path, on the CPU and in `transmit.comp`. Several solids multiply.
- `crates/audio-check` stays a second package that calls `mix`.
- The headless server opens no sound device and creates no Vulkan device. `LoadProfile::Sim` leaves PCM on disk, as the asset plan already says.
- Doppler at 343 m/s, the 1 m reference distance, and horizontal constant-power pan stay the v1 spatial model. HRTF, elevation, and surround stay out, as the system record already says.

## 3. What has to change

| Today | Change | Why |
|---|---|---|
| `mix` allocates `Vec<i16>` on every call | Add `mix_into(&mut [f32], ...)` that writes a caller buffer. `mix` allocates once, calls `mix_into`, and quantizes with `round` as it does now. | The audio thread cannot allocate. Existing callers of `mix` keep their `i16` buffer. |
| `submit` opens, writes, and closes | Hold one device for the process. A thread named `genos-audio` mixes one quantum and writes it. ALSA's latency request drops from 200 ms to the quantum. | A game cannot open ALSA, WASAPI, or CoreAudio per buffer. The Deck feels that cost on every clip. |
| Pan and transmission call platform `sin`, `cos`, and `exp` | Replace those three calls inside `space.rs` with the in-crate polynomials in ADR 0040. | The identical-buffer hash has to survive glibc, the Windows math library, and the macOS math library. |
| Tests check relationships | Add a stored SHA-256 of a fixed `mix` buffer. | Two platforms can pass `mix_scene.rs` with different samples. |
| One gain for the whole buffer | Keep one gain per `mix` call. The held thread calls `mix` once per quantum, so the pose updates every quantum. Smooth the gain with a one-pole whose state starts at the first target. | A single `mix` in today's tests stays a constant gain, so those tests keep their numbers. A running voice does not click when the next quantum's transmission jumps. |
| `Source` has no bus | Add a bus. Default: positional sources go to World, non-positional sources go to Music. With default bus gains, the samples match the pre-bus mix. | Music, UI, and world have to be separate sums without a second mixer. |
| The frame does not fill `Listener` | After the camera step, copy `Camera::position`, `look_direction(yaw, pitch)`, and `physics.bodies[view].velocity` into `Listener`. `genos-audio` does not take a scene dependency. | The listener has to follow the camera. Pan already drops the Y component of facing, so pitch is stored and the stereo image stays horizontal. |
| `transmission_gains` is a fence wait | It stays on the frame thread. The audio thread reads the last published gain. | Calling it from `genos-audio` would stall the device write on Vulkan. |

`examples/submit.rs` uses `sin` and `mul_add` to build a test tone. That file is not the mixer. The mix itself uses `first + (second - first) * frac` and does not call `mul_add`. The polynomial change does not touch the example's tone generator. The golden tape stores its samples as bytes so the input side is also free of the platform math library.

## 4. Gaps

These are absent. Each one is a stage in §11. None of them is a reason to add a second mixer.

### Disk-fed clips

This is the streaming-versus-resident split. The clip is either decoded before it plays, or read from the cook ahead of the playhead.

Every `Source` borrows a `Pcm` that is already fully decoded. A long clip occupies the resident set for as long as it is loaded. The cook's audio bytes are still the WAV PCM.

The split: a clip at or under 2 seconds, or at or under 256 KiB of decoded samples, is resident. `genos-load` decodes it before the voice starts. A longer clip is disk-fed. A thread named `genos-audio-io` reads the next cooked block and fills a ring allocated when the voice starts. `genos-audio` only reads the ring. An empty ring fades the voice over 64 frames and increments `audio_underrun`.

The cook writes IMA ADPCM (4-bit, the IMA reference step table) in this repository. The decoder lives next to `wav.rs`. No new crate. The same decoder runs on every platform, so a cooked clip hashes the same. Opus and Vorbis stay out: Opus needs `libopus` and a pre-roll to seek a loop, Vorbis needs another decoder, and both are a second code path beside a decoder small enough to live here. Authors still export WAV. The cook is the only stage that reads that WAV. OGG, MP3, and FLAC stay identified and undecoded until a later cook input says otherwise.

The stage that lands this cook edits the asset plan sentence that says the audio bytes are the WAV PCM. It writes the canonical record: version byte `1`, codec id `1` for IMA ADPCM, rate, channels, frame count, loop start, loop end, payload. The v1 cook writes that codec id only. The audio subset hash is the SHA-256 of those bytes. Two cooks of the same WAV and the same settings match. A shipping load of a cooked package does not parse RIFF. A missing or mismatched cook falls back to `decode_wav` for that asset, the same way ADR 0035 falls back a bad texture, and logs one line.

Codec error against the source WAV is a separate bound, measured once and written next to the fixture. It is not the cross-platform gate. The cross-platform gate hashes the cooked decode.

### Reverb zones

There is no reflection and no reverb. Transmission is a broadband gain. A wall does not muffle, and a room does not have a tail.

A reverb zone is an axis-aligned box with a send in `[0, 1]` and a tail index. The listener's position picks at most one zone, the smallest box that contains the listener. Equal box volume resolves to the lower tail index, then the lower send, then the earlier zone in the list, so the pick is a function of the zone list. Outside every zone the send is 0. The tail is one Schroeder reverberator: four combs in parallel, then two allpasses in series, delays chosen in samples at 48 kHz, gains constant in source. The voice is scaled by transmission first. The send is a fraction of that scaled voice into the tail. The dry path is the rest. One tail on Medium. A zone on High or Ultra may select a second or further fixed coefficient set. Low forces the send to 0 and does not step the delay lines, so the Low buffer matches the dry mix.

The send moves through the same one-pole as the transmission smoother, so a zone crossing does not click. Zones do not replace transmission and do not read the surface light cache. A sound in another room is quieter because the straight path is absorbed. It does not get its own tail in this stage. Per-source tails are a later knob.

A one-pole low-pass on a voice, driven by the transmission gain, is the muffle that a thick wall is missing. It is a later knob, off on Low, coefficients in this crate. It is not required to land zones.

### Handheld and console device outputs

The Steam Deck is Linux. SteamOS exposes the device through PipeWire and ALSA `default`, the same open ADR 0007 describes for Omarchy. `device.rs` already compiles the ALSA write on that machine. There is no Deck-specific module to add, and Low is the preset the Deck runs ([ADR 0023](../adr/0023-graphics-settings-presets.md)). The gap on the Deck is the open-close cycle and the 200 ms request, which §3 changes for every Linux machine including the Deck.

A Windows handheld uses the WASAPI write the same way.

A console is the `cfg` branch that currently returns "this operating system has no audio device backend." The gap is one write function per vendor SDK: it copies the finished interleaved buffer into the vendor queue and returns the frame count. It contains no `mix` call. The public repository does not add SDK headers, samples, or vendor entry points. The stage lands on a machine that already has the SDK. Review of that stage is the write itself plus the offline hash, which does not need the SDK.

### Cross-platform identical buffer

`mix_scene.rs` and `consumer.rs` pass if the relationships hold. They do not store a hash. `space.rs` calls the platform math library, so the same source can quantize to different `i16` values on two operating systems and both test runs stay green.

The gate is a SHA-256 checked in beside the test. The input PCM is stored bytes. The listener, the barriers, the rate (48 kHz), and the frame count are constants. The test calls `mix` and compares the hash. It does not call `submit` and it does not open Vulkan. `cargo test -p genos-audio -p genos-audio-check` is the run on Linux CI, on a Windows runner, on a macOS runner, and on a console toolchain when one exists. Bit-identical is the requirement. A tolerance compare is the wrong tool for this hash: the polynomials and the ban on `mul_add` exist so the bytes match. Tolerance stays available for two other compares only: CPU transmission against `transmission_gains` (GLSL `exp` versus the polynomial), and cooked ADPCM against source WAV (codec error).

The speakers, the OS mixer, and a device that opens at a rate other than 48 kHz are outside the hash. The held device asks for 48 kHz. A device that refuses that rate increments `device_rate_rejected`. Playback on that machine may resample with the linear interpolator already in `mix.rs`. That resample is a separate test with its own hash. It is not a substitute for the 48 kHz hash.

## 5. Libraries considered

One library that both owns a production mix and ships a console device output in a form this public tree can test does not exist for us. FMOD and Wwise are the libraries that cover consoles, and they own the mix. Every other candidate either stops at the desktop or would replace `mix.rs`.

| Library | Mix | Public device outputs | Consoles | Licence | Decision |
|---|---|---|---|---|---|
| `genos-audio` (this crate) | `mix` and `space` already | ALSA, WASAPI, CoreAudio, write-only | A later write of the same buffer. No SDK in the public tree. | In-repo | Keep. This is the one implementation. |
| FMOD | Engine owns it | Desktop, mobile, and the consoles FMOD ships | Yes, under the FMOD licence and the vendor agreement | Free under $200k yearly revenue and a budget under $600k. Paid tiers on that page are $2,000, $6,000, and $18,000 per game. "Platforms: All". | Reject. The mix would leave this crate. CI cannot hash it without the SDK. |
| Wwise | Sound engine owns it | Desktop, mobile, and consoles | Yes, per-platform fee | Free under a $250k budget, extra platforms included. Pro is $8,000 for the first platform and $4,000 for each extra. | Reject, for the same reason as FMOD. |
| miniaudio | Engine and node graph | WASAPI, DirectSound, WinMM, Core Audio, ALSA, Pulse, JACK, sndio, audio(4), OSS, AAudio, OpenSL, Web Audio, null, plus a custom output | Custom output, written by the integrator | Public domain or MIT-0 | Reject. A console is still our write. The engine would replace `mix`. |
| `cpal` | None | WASAPI, Core Audio, ALSA, and others | No | MIT or Apache-2.0 | Already rejected by ADR 0007. |
| `rodio` | Sinks on `cpal` | Whatever `cpal` opens | No | MIT or Apache-2.0 | Already rejected by ADR 0007. |
| Kira | Tracks, effects, spatial scene, command list | Default output is `cpal`. Another output can be supplied. | No | MIT or Apache-2.0 | Reject. It would replace `mix`, and the default output is a crate ADR 0007 rejects. |
| SDL and SDL_mixer 3 | Basic pan. Listener fixed at the origin. No Doppler. Can render to memory. | SDL's audio device | No public console output | zlib | Already rejected by ADR 0007. |
| oddio | Pulls into a caller buffer | None | No | MIT or Apache-2.0 | Reject. The pull split is what `mix_into` adds. The spatial scene would replace `space.rs`. |
| Steam Audio | HRTF, occlusion, reflections, reverb on its own scene | None. It is a spatial library. | No official output. A partner ports it. | Apache-2.0 | Reject for v1. It is a second occluder world beside `transmit.comp`. |

The minimal set of device outputs is the three that already link, plus the offline buffer the test writes, plus one console write per SDK when that SDK is present. Adding `cpal` or miniaudio would replace the three with one crate and would still leave consoles as a custom write.

## 6. Engine API

Games keep building a `Listener` and one `Source` per clip. They call `mix` in tests and tools. A running game enqueues commands. It does not call `submit`, and it does not call into `device.rs`.

```text
Bus          World | Music | Ui
ClipId       u32 index into a slab published before Play
VoiceId      u32 index into the voice pool

Command      Play { voice, clip, bus, positional, looping, gain, position, velocity }
             Stop { voice }
             SetGain { voice, gain }
             SetPose { voice, position, velocity }
             SetTransmission { voice, gain }
             SetBusGain { bus, gain }

mix_into(out: &mut [f32], sources, listener, barriers, rate)   // no allocation
mix(...) -> Vec<i16>                                           // today's function, calls mix_into
render_offline(tape) -> Vec<i16>                               // test and debug/audio only
submit(interleaved, rate)                                      // one-shot, the example
// held device: genos-audio writes one quantum; games do not call it
```

`Play` on a positional source defaults the bus to World. `Play` on a non-positional source defaults the bus to Music. Ui is for clicks and menu sounds, which are non-positional and skip the world tail. Master is the sum of the three buses after their gains. It is not a value of `Bus`.

The command ring holds 256 `Copy` commands on Low. Overflow drops the newest command and increments `audio_command_drop`. The listener pose is a latest-wins slot with a sequence count, not a ring entry.

`AudioSettings` lives in `genos-audio` with `to_text`, `parse`, and `set_key`, the three methods from the [settings principles](../engine/settings-principles.md). The trait stays in `genos-scene` until a second settings type needs it in a crate both can see. `genos-audio` does not depend on `genos-scene`, and `genos-scene` does not depend on `genos-audio`. The frame that already owns the camera holds the settings value. MCP commands are `get_audio_settings`, `set_audio_settings`, and `get_audio_stats`. A read of stats returns the probe counters from §9.

Rhai registers `audio.play`, `audio.stop`, and `audio.bus_gain` in the stage that adds the debug binary. A script does not call `submit`. That is ADR 0009.

When the scene ECS lands, the camera entity carries a listener component with the same three fields `Listener` has. The frame copies that component into the latest-wins slot. Until that table exists, the frame copies the camera as §3 describes. `genos-audio` still does not query the ECS.

## 7. Threading

Ross Bencina, "Real-time audio programming 101: time waits for nothing", is the constraint. The device thread does not allocate, does not lock, and does not call anything that can wait.

```text
frame thread
  camera step
  copy Listener
  enqueue Commands
  every 2 frames on Low: transmission_gains or the CPU barrier walk
  publish gains

genos-audio-io          (disk-fed clips only)
  read cooked blocks
  decode IMA ADPCM into the voice ring

genos-audio             (the device thread, not a job worker)
  read commands and the latest Listener
  read voice rings and resident slabs
  mix_into the quantum
  one-pole on transmission and reverb send
  Schroeder step when the preset's send is live
  quantize to i16
  write the device
```

The job-system plan does not run this thread. A stolen worker would miss the device deadline. The same reason keeps `vkQueueSubmit` on one thread in that plan.

Voice slots, bus buffers, the command ring, and the reverb delay lines are allocated when the device opens, sized from `AudioSettings`. A preset change that resizes the pool happens between quanta: the frame thread publishes a new pool, the device thread swaps at the next quantum boundary. The quantum function itself does not allocate.

The allocation gate is a test `#[global_allocator]` that panics on alloc. It calls `mix_into` and the quantum function. `mix` and `render_offline` may allocate. They are the test and tool path.

`transmission_gains` writes an 8192-byte ray buffer and a 4096-byte gain buffer and waits on `audio_fence`. That stays on the frame thread. A gain older than two frames increments `occlusion_age_frames`. The mix uses the CPU `Barrier` walk when no renderer is open, which is the offline hash and the headless tool path.

## 8. Occlusion, the cache, and physics

Keep the walk that already matches on both sides.

- CPU: `transmission_gain` over `Barrier` boxes in `space.rs`.
- GPU: `transmit.comp` over `scene_occ`, thickness from the same segment test, absorption from `occ.albedo.a`. The frame calls `retain_scene` when solids change and the frame will not draw. A draw refreshes that resident scene when the light data changes. `sync_audio_scene` runs only when audio asks.

The surface light cache is a hash of surface patches that store arriving light. Audio does not read it. The resident occluder list is the geometry the transmission shader already uses, and it is updated with the scene, not with the lighting budget.

`crates/physics` does not expose a segment query. `ray_hits_triangle` is private and tests a triangle for the mesh cooker path. A physics ray would also see the collision shapes (the physics record's box and hull caps), which are not the occluder boxes `transmit.comp` walks. This design does not add that ray.

The new test, beside `crates/render/tests/transmit.rs`, runs one listener and one wall through `transmission_gain` and through `transmission_gains` on lavapipe. The absolute difference is at most `1e-4`. `transmit.rs` keeps its relationship checks. The golden audio hash uses the CPU gain so a GPU math difference cannot move the hash.

## 9. Probes

Every failure mode and every tunable has a probe. The stage that adds the feature adds the probe in the same commit. `get_audio_stats` returns these counters. The debug overlay lists them. `debug/audio` writes them to `./genos-debug/audio/metrics.json`.

| Failure or tunable | Probe |
|---|---|
| Device open failed | `device_open_error` (the OS string, one line) |
| Device refused 48 kHz | `device_rate_rejected` |
| OS period above 40 ms (1920 frames) | `device_period_frames` |
| ALSA xrun or a WASAPI/CoreAudio underrun | `device_xrun` |
| Disk-fed ring empty | `audio_underrun` |
| Command ring full | `audio_command_drop` |
| Voice pool full | `audio_voice_drop` |
| Resident bytes over the cap | `audio_resident_drop` |
| Occlusion gain older than two frames | `occlusion_age_frames` |
| Quantum mix time | `mix_us` |
| Voices playing, disk-fed fills | `voices_active`, `disk_feeds_active` |
| Per-voice transmission target and smoothed gain | voice dump |
| Bus gains, zone send, tail index | `get_audio_settings` |
| Preset, voice cap, disk-fed cap, resident cap, quantum, reverb, occlusion | `AudioSettings` keys, `set_key` |

A snapshot is the command tape plus the `Listener` slot plus the settings text. `render_offline` restores that snapshot into a buffer. That is the reproduce path for a bad mix. The device is not part of the snapshot.

## 10. Low ceilings

These are design ceilings. Nothing here has been measured on a Deck or on a 1060-class machine. The debug binary records `mix_us` and resident bytes. A knob that misses its ceiling on that hardware moves down a preset, as ADR 0023 requires. One mixer covers every row. Audio bytes are system memory. They are not part of the lighting VRAM budget.

| Knob | Low | Medium | High | Ultra |
|---|---|---|---|---|
| Voices | 32 | 64 | 96 | 128 |
| Disk-fed clips | 2 | 4 | 8 | 8 |
| Decoded resident | 8 MiB | 16 MiB | 32 MiB | 48 MiB |
| Rate | 48 kHz | 48 kHz | 48 kHz | 48 kHz |
| Channels | stereo | stereo | stereo | stereo |
| Quantum | 480 frames (10 ms) | 480 | 256 frames | 256 |
| Reverb tails | 0 | 1 | 2 | 4 |
| Occlusion paths | 32, every 2 frames | 64, every frame | 96, every frame | 128, every frame |
| Mix CPU ceiling | 1.0 ms | 1.5 ms | 1.5 ms | 2.0 ms |
| Command ring | 256 | 512 | 1024 | 1024 |

Low is the Steam Deck row and the GTX 1060 / RX 580 row. The GPU occlusion dispatch stays inside the existing 256-source cap. Low asks for 32 of those. The 1.0 ms ceiling is on `genos-audio`, not on the frame. Frame-thread occlusion is a separate counter in `get_audio_stats` (`occlusion_us`) and must stay under 0.5 ms on Low. That number is also a design ceiling.

8 MiB of decoded `f32` mono at 48 kHz is about 44 seconds of unique resident audio. A shared clip occupies the slab once. The disk-fed rings (200 ms, stereo `f32`, two clips) are about 150 KiB and count toward the 8 MiB. A Schroeder tail is tens of kilobytes and is unallocated on Low.

## 11. Stages and gates

Each stage is a later commit. This document is stage 0. A stage edits the [audio record](audio.md) when it changes a goal, an intent, a game use, or a limit.

| Stage | Work | Gate |
|---|---|---|
| 0 | This design and ADR 0040 | The documents name the current files, the changes, and the gaps. No behavior change. |
| 1 | `mix_into`. `mix` keeps the `i16` result. | `cargo test -p genos-audio -p genos-audio-check` passes. A `#[global_allocator]` test calls `mix_into` and panics if it allocates. The existing relationship tests keep their results. |
| 2 | In-crate `sin`, `cos`, and `exp`. Stored golden hash. | `crates/audio/tests/golden/positional.sha256` matches `mix` of the stored tape at 48 kHz. The test does not open a device. The same command is the Windows, macOS, and console run. |
| 3 | Held device. `genos-audio` writes one quantum. Linux first. WASAPI and CoreAudio take the same hold on their machines. | A test feeds a known buffer and asserts the write path does not call `mix`. `device_period_frames` and `device_xrun` exist. ALSA is asked for 10 ms. WASAPI is asked for one quantum, replacing the one-second floor. The one-shot `submit` stays for `examples/submit.rs`. The example and `genos-audio` are not open on the same device together. A running game uses the held device. |
| 4 | Command ring and latest-wins listener. | Two threads: the producer enqueues a fixed tape, the consumer runs quanta. The `i16` hash matches stage 2. Overflow increments `audio_command_drop` and the listener slot still updates. |
| 5 | Buses. Defaults preserve today's positional flag. | With default gains, the hash matches stage 2. Music gain 0 silences the non-positional source and leaves the positional source. A new hash file records that case. |
| 6 | Camera pose copied into `Listener` once per frame. | A headless scene step moves the camera and the offline hash changes. A scripted path has its own hash. `genos-audio` still does not depend on `genos-scene`. |
| 7 | One-pole on the published transmission gain. GPU path unchanged. | The one-shot `mix` matches stage 2, because the smoother starts at the target. A lavapipe test bounds CPU versus `transmission_gains` by `1e-4`. No read of the surface light cache. No physics ray. |
| 8 | IMA ADPCM cook. Resident clips and disk-fed clips. | Two cooks of one WAV share a subset hash. The audio thread's allocator test still passes while a disk-fed clip plays. Forcing an empty ring increments `audio_underrun`. The asset plan's WAV-PCM sentence is edited in this commit. |
| 9 | Reverb zones. | Low's hash matches the dry hash from stage 5. A listener inside a zone changes the hash. A listener outside every zone matches the dry hash. `mix_us` with the tail forced off stays at the dry measurement. |
| 10 | `AudioSettings`, MCP, `debug/audio`. | `debug/audio --render-offline --out <dir>` writes `metrics.json` and the wav under that directory, default `./genos-debug/audio/`. `--out` wins over `GENOS_AUDIO_OUT`. The variable is registered in `lint/env_mirrors.toml` as a mirror of `--out` in that commit. The binary uses the shared debug scaffolding. The probes in §9 have a command each. |
| 11 | One console write, only on a machine with that SDK. | The module contains the vendor write and no `mix` call. The stage 2 hash passes on that toolchain. No SDK header is added to the public tree. |

Stage 3's WASAPI and CoreAudio holds are verified on those machines. This Linux tree compiles them and does not run them, which the system record already says.

## 12. Networking

The implementing commit that first sends a play command copies these lines into the audio record. Until then the record's lines are the ones for today's process-local mix.

- **Replicated state.** World `Play` and `Stop` commands: asset id, sim tick, pose, gain, loop, bus. Music and UI stay on the client. The mixed buffer and the transmission gain stay on the client.
- **Authority.** The server is the authority for world one-shots. The client is the authority for its UI and its own footsteps. A predicted footstep stops when the server does not confirm the step.
- **Prediction / rollback.** The mix does not rewind. A rejected one-shot stops at the next quantum. Source poses follow the replicated transform.
- **Determinism.** `render_offline` of one tape is bit-identical across platforms. The device clock is not a lockstep input. The server does not mix.
- **Headless server.** No sound device, no `genos-audio` thread, no `transmission_gains`. The server still records the world play commands it replicates. `LoadProfile::Sim` leaves the audio bytes on disk.

## 13. Debug binary

`debug/audio` is stage 10. It follows [Debugging](debugging.md) and the lighting inventory: scene picker and hotkey, checkpoints, reset and loop, replay, metrics, live sliders for `AudioSettings`, a guided-review panel, and a headless CLI. The picture analogue is the wav plus `metrics.json` under `./genos-debug/audio/`. `--out` overrides that directory. The process does not write under `$HOME`.

The guided review listens for three things: a positional source moves across the stereo field when the camera yaws, a wall drops the level, and a reverb zone on Medium changes the tail while Low stays dry. The hash is the pass/fail. The wav is what a person hears on the owner's machine.

## 14. Papers

The mix follows these, and the departures are in ADR 0040.

- Doppler is the still-air ratio already in `space.rs`: `(c + listener) / (c - source)` along the line between them, `c = 343`.
- Transmission is Beer-Lambert through the overlapped thickness, the product the fog design uses for optical depth, applied here to occluder boxes. The code already implements it. This plan does not change the formula.
- Schroeder, JAES 1962, for the zone tail. No geometric early reflections.
- Cephes `sinf` and `expf` for the two transcendental calls the mix needs. Pan's `cos` comes from the sine polynomial.
- Giacomoni, Moseley, and Vachharajani, PPoPP 2008, for the single-producer command ring.
- Bencina, "Real-time audio programming 101", for what the device thread is allowed to do.
- IMA ADPCM reference step table, for the cooked clip. The decoder is in this repository.

Linear interpolation stays the resampler `mix.rs` already uses. A bandlimited resampler is a later knob, off on Low, and it would be another in-crate function with its own hash. It is not a stage in §11.
