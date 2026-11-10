# 0040: One audio mix, and device outputs only write that mix

Status: **Proposed**. Design: [audio design](../systems/audio-design.md). The living record of the code that already ships is [Audio](../systems/audio.md).

## Context

`genos-audio` already mixes loaded PCM and writes the result to the operating system. `mix` in `crates/audio/src/mix.rs` applies Doppler, constant-power pan, inverse distance, and direct-path transmission from `crates/audio/src/space.rs`. `submit` in `crates/audio/src/device.rs` sends interleaved 16-bit stereo to ALSA on Linux, WASAPI on Windows, and CoreAudio on macOS. Any other operating system returns an error. `Renderer::transmission_gains` runs the same transmission rule on the resident occluder list (`crates/render/shaders/transmit.comp`). `crates/audio-check` calls `mix` and does not copy it.

[ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md) keeps a third-party crate off the sound device and already rejects `cpal` and `rodio`. The Steam Deck, Windows handhelds, and consoles still have to play that same mix. A second mixer, or a mixer inside a platform library, would make the same scene sound different per machine. The mix has to be one piece of code. Each platform only writes the finished buffer.

The current `submit` opens the device, writes, and closes on every call. ALSA asks for 200 ms. WASAPI initializes a buffer of at least one second (`duration.max(10_000_000)` in 100 ns units) and releases the client before returning. `mix` allocates the output buffer. Pan and transmission call the platform math library (`sin`, `cos`, `exp`). Tests in `crates/audio/tests/mix_scene.rs` check relationships (an approaching source rises in pitch). They do not hash the buffer, so two platforms can pass while writing different samples. There is no bus, no clip read from disk during playback, no reverb zone, no held device thread, and no console write. The camera does not fill `Listener`.

## Decision

1. **One mix.** Mixing, resampling, buses, Doppler, pan, distance, transmission, and reverb stay in `genos-audio`. That code is the same on every platform. No SIMD path and no `mul_add` in the mix. `sin`, `cos`, and `exp` used by the mix move into this crate as published polynomials, so the platform math library cannot change a sample.
2. **Device outputs only write.** ALSA, WASAPI, and CoreAudio stay the three writes ADR 0007 names. Each one receives the interleaved buffer `mix` already produced. A console adds one write of that same buffer when its SDK is on the machine that commits it. The public tree does not gain a console SDK. The Steam Deck and any other Linux handheld use the ALSA write. A Windows handheld uses the WASAPI write. There is no fourth mixer for a handheld.
3. **The identical-sound gate is an offline buffer.** A fixed command tape rendered by `mix` produces a SHA-256 of the 16-bit stereo buffer. `cargo test -p genos-audio` checks that hash on every platform that runs the suite. The hash is bit-identical. The sound device, the operating-system mixer, and the speakers sit outside the hash. A lossy cook has its own error bound against WAV. That bound is not the platform gate. The platform gate hashes the cooked decode, which is still bit-identical across machines.
4. **Build on the current types.** `Listener`, `Source`, `Barrier`, `mix`, and `submit` stay the engine-facing API. New work adds a non-allocating write into a caller buffer, a held device, buses, a disk-fed clip, and reverb zones around those types. `crates/audio-check` keeps calling the public mix.
5. **Occlusion stays the straight-path transmission that already ships.** The surface light cache stores light on surface patches. It has no thickness and no absorption. A physics ray is not a public query (`ray_hits_triangle` is private in `crates/physics/src/mesh.rs`) and it would see collision shapes, not the resident occluders. The frame thread keeps calling `transmission_gains` or the CPU `Barrier` walk. The audio thread only reads the latest gain.
6. **Low is a row of knobs on that one mix.** Presets follow [ADR 0023](0023-graphics-settings-presets.md): one code path, a named table of caps. The Low ceilings (design targets, not yet measured) are 32 voices, 2 disk-fed clips, 8 MiB of decoded samples, a 480-frame quantum at 48 kHz, reverb off, and 1.0 ms of mix CPU per quantum.

## Rejected

- **One third-party library for the whole path, including consoles.** FMOD's published game fee is free for a studio under $200k yearly revenue on a development budget under $600k. The same page lists $2,000, $6,000, or $18,000 per game on the paid tiers, and the included row is "Platforms: All". Wwise is free under a $250k budget, extra platforms included on that row. The Pro row is $8,000 for the first platform and $4,000 for each extra. Both own the mix, the spatializer, and the device. A golden hash would need their SDK in CI, and the console binaries sit behind the vendor agreement. That conflicts with ADR 0007 and with a mix this repository can render offline.
- **miniaudio as that one library.** The public tree writes WASAPI, DirectSound, WinMM, Core Audio, ALSA, PulseAudio, JACK, sndio, audio(4), OSS, AAudio, OpenSL ES, Web Audio, and a null output. A console is a custom output the integrator writes. The engine and node graph would replace `mix.rs`. Licence is public domain or MIT-0. SIMD is on unless `MA_NO_SSE2`, `MA_NO_AVX2`, and `MA_NO_NEON` are set. Using it for the mix moves the samples out of this crate. Using it only as the device write replaces ALSA, WASAPI, and CoreAudio, which already exist.
- **`cpal`, `rodio`, and SDL.** ADR 0007 rejects all three. `cpal` is a device layer (WASAPI, Core Audio, ALSA, and others) with no console output and no mix. `rodio` mixes on top of `cpal`. Kira has a game-thread command list and can take an output other than `cpal`. The default output is `cpal`, and there is no console output. SDL_mixer 3 can render to a memory buffer and pan a track. Its listener stays at the origin, and it has no Doppler. Our `Listener` already has position, facing, and velocity.
- **oddio as the mix.** It pulls frames into a caller buffer and does not open a device, which is the right split, and it has no console output, no cook, and no occluder walk. Adopting it would replace `mix` and `space.rs`.
- **Steam Audio as the spatializer.** Apache-2.0. Supported platforms are Windows, Linux, macOS, Android, and iOS. Console support is a port the integrator does. It traces its own scene for occlusion and reverb. That is a second world beside `transmit.comp`. It does not open the sound device.
- **Reading the surface light cache for occlusion.** The cache is light on surface patches ([glossary](../lighting/GLOSSARY.md)). A stale page would change a gain for a lighting reason. Audio keeps the occluder segment test.
- **A physics ray per source.** No public segment query. The shapes are the collision set, not the resident occluders the picture uses.

## Consequences

- `device.rs` grows a held device and, later, one console write. It does not grow a mixer.
- The asset plan's sentence "the audio bytes are the WAV PCM" changes when the cook stage writes IMA ADPCM. Short clips stay resident. Long clips are read from the cook during playback. The runtime still does not transcode a source file (ADR 0020).
- The audio thread is `genos-audio`. It is not a job-system worker. It does not allocate, lock, or call `transmission_gains` (that call waits on a fence).
- `debug/audio` is the stage that exposes the probes. Until it lands, the system record stays the description of today's code.
- Accepting this ADR does not retarget ADR 0007. It keeps the three device libraries named there.

## Departures

- From Schroeder, "Natural Sounding Artificial Reverberation", JAES 1962: the comb and allpass tail is the reverb zone's tail. There is no geometric early reflection in that stage. A zone picks the send and, on High and Ultra, which fixed coefficient set.
- From the Cephes `sinf` and `expf` polynomials (Moshier, public domain): the mix uses those approximations for pan and transmission. The published error bound is part of the stage that locks the hash. GLSL `exp` in `transmit.comp` stays. CPU and GPU gains match inside a tolerance. The golden hash uses the CPU value.
- From Giacomoni, Moseley, and Vachharajani, "FastForward for Efficient Pipeline Parallelism", PPoPP 2008: one producer (the frame thread) and one consumer (`genos-audio`) share a fixed command ring. The listener pose is a separate latest-wins slot, so a full ring still moves the listener.
