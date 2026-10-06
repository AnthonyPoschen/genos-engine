# Loading

Loading turns file bytes into assets. A caller can load an asset with no window and no GPU.

## Goal

A game can load an image, a texture, a texture map, a mesh, a material, an animation, PCM samples, and a font. The same bytes produce the same asset from a file path and from memory.

## Intent

Decoders read bytes only. They do not open a window, an audio device, a GPU, or a socket.

A file path and a memory block are byte sources. A network payload and a disc payload use the same decoders after the bytes arrive.

An image is RGBA8. The origin is the top-left. A texture keeps a name and that image.

A texture map keeps one image, frame rectangles, and named clips. The description is text. It is not a second image codec.

A mesh is triangles from a Wavefront OBJ file. A material is a flat color and an optional texture file name.

An animation is timed transform keys. Rotation is a quaternion in x, y, z, w order. `Quat::slerp` takes the short arc when the dot product is negative.

PCM is 16-bit, mono or stereo. Samples are interleaved. A font glyph has a horizontal advance and a simple outline in font units.

A missing path, a truncated stream, and unrecognized bytes return `LoadError`. They do not return an asset.

## Code

The crate is `crates/load`, package `genos-load`.

- `ByteSource`, `FileSource`, and `MemorySource` are in `src/source.rs`.
- `load_image` decodes a PNG in `src/png.rs`. Inflate is in `src/inflate.rs`.
- `load_texture` and `load_texture_map` start in `src/lib.rs`. The map parser is in `src/texture.rs`.
- `load_mesh` reads OBJ text in `src/mesh.rs`.
- `load_material` reads the material text in `src/material.rs`.
- `load_animation` reads keys in `src/animation.rs`. Rotation uses `genos-math` `Quat::slerp`.
- `load_pcm` reads WAV data in `src/wav.rs`.
- `load_font` reads a TrueType font in `src/ttf.rs`.

`crates/load-check` calls this public API. It does not link the window or the renderer.

The texture map text is:

```text
frame <name> <x> <y> <width> <height>
clip <name> <frame-seconds> <frame-name> <frame-name> ...
```

The material text is `color <red> <green> <blue>` and optional `texture <file-name>`.

The animation text is `key <time> <tx> <ty> <tz> <qx> <qy> <qz> <qw> <sx> <sy> <sz>`.

`#` starts a comment line in each text file.

## Game use

Build a `FileSource` or a `MemorySource`. Pass that source to the `load_*` function for the file kind.

Pass the image source and the description source to `load_texture_map`. Call `TextureMap::sample` with a clip name and a time in seconds.

Call `Animation::sample` with a time in seconds. Call `Font::glyph` with a character. Read `pixel` on a texture.

Pixels, triangles, frame rectangles, glyph outlines, and PCM samples stay in the load result. Upload or playback is a later call.

## Limits

The loader reads the whole source into memory. It does not stream.

PNG images are 8-bit and not interlaced. Each edge is at most 8192 pixels. JPEG, GIF, WebP, BMP, DDS, KTX, and block-compressed textures are not decoded.

glTF and FBX are not decoded. A face with more than three vertices becomes a triangle fan.

The material stores a file name. It does not sample that texture.

The animation does not move a mesh. WAV playback is the audio system. This crate does not open a device. OGG, MP3, and FLAC are not decoded.

A composite TrueType glyph returns an error. The loader does not hint, kern, or shape text.

This crate does not upload pixels or triangles to Vulkan.

## Decisions

[ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps third-party crates off the audio device and the network. This crate does not link those devices. Inflate lives in this crate.
