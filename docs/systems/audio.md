# Audio

Audio plays loaded PCM. A positional source changes pitch, stereo image, and level as it moves. Walls on the straight path change that level.

## Goal

A game can play several clips into one stereo stream. The stream can carry music, UI, and world sources together. World sources keep airborne Doppler, a left-right image, and distance. Solids on the direct path transmit or block the sound.

## Intent

The mix is 16-bit interleaved stereo at the device rate. Linux sends that mix to ALSA. Windows sends it to WASAPI. macOS sends it to CoreAudio. A third-party crate does not own the device.

A non-positional source has no pitch shift, no left-right image, no distance loss, and no wall loss. A one-shot goes silent after its samples. A loop continues.

Doppler uses still air at 343 m/s. Only the velocity component along the line between the source and the listener counts. Motion toward the other raises pitch. The sample amplitude falls with distance and stays finite up close. A source in front is balanced. A source to one side, including behind the listener, favors that side.

Each wall and solid has an absorption in nepers per meter. Zero leaves the level unchanged. The straight path takes the overlap with the resident box as thickness. The transmission is `exp(-absorption * thickness)`. Several solids on one path multiply.

The GPU pass is one compute dispatch on the renderer's Vulkan device. It reads the resident scene buffer that the light pass already keeps. Absorption sits in that occluder record. The dispatch does not present a swapchain image and does not upload a second copy of the occluders. A game can pass those gains back into the mix.

## Code

The crate is `crates/audio`, package `genos-audio`.

- `mix` and `mix_into` are in `src/mix.rs`. `mix_into` sums into a caller-owned buffer and does not allocate. `mix` quantizes that sum into the returned buffer.
- Sine, cosine, and the barrier exponential are the Cephes `sinf`, `cosf`, and `expf` polynomials in `src/poly.rs`, with range reduction. The mix does not call the platform math library for those and does not use a fused multiply-add.
- Doppler, balance, distance, and transmission are in `src/space.rs`.
- `start_with` in `src/engine.rs` runs the `genos-audio` thread. It mixes one quantum and writes a `DevicePath`. ALSA, WASAPI, and CoreAudio stay open. The command ring holds 256 commands. A full ring drops the newest and counts `audio_command_drop`. The listener is a latest-wins slot.
- The frame copies `listener_state` from `crates/scene` into that slot. `genos-audio` does not depend on `genos-scene`.
- `cook_ima` in `crates/load/src/adpcm.rs` writes the cooked record. A clip over 2 seconds and over 256 KiB is disk-fed: `genos-audio-io` decodes into a 200 ms ring. An empty ring fades 64 frames and counts `audio_underrun`.
- `apply_reverb` in `src/reverb.rs` adds one Schroeder tail. The listener's smallest containing zone wins. Low, and a listener outside every zone, leave the dry buffer unchanged.
- `Renderer::transmission_gains` is in `crates/render/src/gpu.rs`. The shader is `crates/render/shaders/transmit.comp`.

`crates/audio-check` calls `mix`. It does not copy the mixer.

## Game use

Load PCM with `genos-load`. Build a `Listener` and one `Source` per clip. Call `mix` with the barriers, the device rate, and the frame count. A realtime caller keeps the stereo buffer and calls `mix_into`.

Set `positional` to false for music and UI. Set `looping` to keep a clip playing. Set `transmission` when `Renderer::transmission_gains` already computed that source.

A running game calls `start_with` and `Stream::push`. After the camera step it copies `listener_state` into `Listener` and calls `Stream::set_listener`. It does not open the device per buffer. Open one `Renderer`. Call `retain_scene` when the solids change and the frame will not draw. A draw also refreshes that same resident scene when the light data changes. Call `transmission_gains` with the listener and the source positions.

## Limits

There is no HRTF, no elevation cue, and no surround layout. There is no reflection, diffraction, air absorption, or frequency-dependent wall filter. A reverb zone is one Schroeder tail. Low does not step it.

The loader does not open a device. The cooked clip is IMA ADPCM. Voice chat, MIDI, and synthesis are not in this crate.

The transmission pass reads the resident occluder bounds and absorption. The camera scene does not open a sound device.

WASAPI and CoreAudio are not run from this Linux machine.

Buses and a console write are the plan in [the audio design](audio-design.md).

`cargo test -p genos-audio` hashes the mix of `tests/golden/tape.pcm` at 48 kHz. The expected digest is `tests/golden/positional.sha256`. A second digest, `tests/golden/paths.sha256`, covers loop wrap, a Doppler cursor that wraps, a non-trivial exp reduction, an `i16` clip, and two voices. Those tests do not open a device.

## Networking

- **Replicated state.** None. The mixed buffer stays in the process that called `mix`.
- **Authority.** That process.
- **Prediction / rollback.** None. `mix` does not rewind.
- **Determinism.** `mix` is a pure function of its arguments. The stored-tape hash is the check.
- **Headless server.** No sound device. The server does not call `submit`. The design's play-command lines replace this section in the commit that sends them.

## Decisions

[ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps third-party crates off the audio device. [ADR 0041](../adr/0041-one-audio-mix-device-outputs-only.md) (accepted) keeps this mix and these device outputs. Loading stays decode-only. See [Loading](loading.md).
