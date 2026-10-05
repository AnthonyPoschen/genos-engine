# One Vulkan renderer, with MoltenVK on macOS

The renderer uses Vulkan on Linux, Windows, and macOS. Apple provides Metal. MoltenVK runs Vulkan on Metal, so the renderer stays one path. The macOS window gives MoltenVK a Metal layer. The instance enables portability enumeration. The surface extension is `VK_EXT_metal_surface`. A Metal renderer waits until a required Vulkan feature has no portability path.

**Rejected:** `wgpu` as the GPU API. A hand-written Metal backend in the first renderer.
