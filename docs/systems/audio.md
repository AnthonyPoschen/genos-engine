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

- `mix` is in `src/mix.rs`. Doppler, balance, distance, and transmission are in `src/space.rs`.
- `submit` is in `src/device.rs`. ALSA, WASAPI, and CoreAudio are compiled only on their own operating system.
- `Renderer::transmission_gains` is in `crates/render/src/gpu.rs`. The shader is `crates/render/shaders/transmit.comp`.

`crates/audio-check` calls `mix`. It does not copy the mixer.

## Game use

Load PCM with `genos-load`. Build a `Listener` and one `Source` per clip. Call `mix` with the barriers, the device rate, and the frame count.

Set `positional` to false for music and UI. Set `looping` to keep a clip playing. Set `transmission` when `Renderer::transmission_gains` already computed that source.

Call `submit` with the interleaved buffer. Open one `Renderer`. Call `retain_scene` when the solids change and the frame will not draw. A draw also refreshes that same resident scene when the light data changes. Call `transmission_gains` with the listener and the source positions.

## Limits

There is no HRTF, no elevation cue, and no surround layout. There is no reflection, reverb, diffraction, air absorption, or frequency-dependent wall filter.

The loader does not open a device. Compressed audio, voice chat, MIDI, and synthesis are not in this crate.

The transmission pass reads the resident occluder bounds and absorption. The camera scene does not open a sound device.

WASAPI and CoreAudio are not run from this Linux machine.

## Decisions

[ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps third-party crates off the audio device. Loading stays decode-only. See [Loading](loading.md).
