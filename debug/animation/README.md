# debug/animation

`genos-animation`, the animation debug binary. This revision is a headless
import dump. It does not open a window or a Vulkan device. Replay, checkpoints,
the settings MCP, and `--measure-skin` arrive with the CPU runtime and the
GPU skin pass.

```
cargo run -p genos-animation-debug -- --help
cargo run -p genos-animation-debug -- path/to/character.gltf
```

The JSON lists each skin (joint name, parent index, source node, notes), each
clip (name, duration, channel path and interpolation), the influence count,
the skin index on each mesh node, `skin_notes`, and the import notes.

A missing inverse bind, a vertex reduced from more than four influences, a
zero weight sum, and an omitted cubic-spline sampler are the failure modes
this dump exposes. They are also the strings `GltfScene::skin_notes` and
`Skin::notes` carry.
