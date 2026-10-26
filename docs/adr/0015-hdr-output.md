# 0015: HDR output stays scene-referred until the display

Status: **proposed** (Anthony Poschen, 2026-10-26). Design: [`docs/systems/hdr-output-design.md`](../systems/hdr-output-design.md).

## Context

Compose writes the display curve into an 8-bit UNORM image. The swapchain is `B8G8R8A8_UNORM` (format 44) or `R8G8B8A8_UNORM` (format 37), color space `VK_COLOR_SPACE_SRGB_NONLINEAR_KHR` (0). The curve is the identity at or below 0.64 and a shoulder above, and it never reaches code 1. The path-trace grade undoes that shoulder with `linear_from_display` and scores Rec.709 luminance. A highlight that the shoulder has crushed cannot come back, so the grade is not the scene the GI passes computed.

Bloom, auto-exposure, and an HDR swapchain are named in the lighting backlog and sketched in the rendering extras plan. They are not built. Exposure and bloom depend on the camera and the lens. Stored light must not (ADR 0012).

## Decision

1. Compose writes a **scene-referred image**: Rec.709 linear radiance in `R16G16B16A16_SFLOAT`, with values above 1 left intact. Exposure, bloom, and the display transform run after it and do not write the surface light cache, the world radiance cache, or the screen probes.
2. The path-trace grade reads that image, before exposure and bloom. It does not invert the display transform. `linear_from_display` stays only for the legacy shoulder, until that readback exists.
3. **Exposure** is one scale, `1 / (1.2 × 2^EV100)`. Auto-exposure takes EV100 from a log-luminance histogram with the outer 10% of samples clipped. A physical camera (aperture, shutter, ISO) writes the same EV100. A still scene holds the scale.
4. The default **display transform** is AgX (Sobotka, the Blender / Eary Chow formation). ACES 2.0 is an alternate. The legacy shoulder stays a mode so today's bytes can be reproduced. The Narkowicz rational fit is not an implementation of either.
5. **Bloom** is the Call of Duty dual-filter pyramid (Jimenez, SIGGRAPH 2014), thresholdless, on the scene-referred image, before exposure. The first downsample uses the Karis weight. A convolution bloom and an eye point-spread function are not the kernel.
6. The swapchain prefers **scRGB** (`R16G16B16A16_SFLOAT`, `VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT`) when the surface lists it, then **HDR10** (10-bit UNORM, `VK_COLOR_SPACE_HDR10_ST2084_EXT`), then the current SDR UNORM pair. Potato and Low force SDR. macOS uses whatever MoltenVK lists, which is SDR unless an HDR space is actually listed. Ray queries stay optional. This pass does not trace.

## Rejected

- Tonemapping inside compose, which is what the tree does today.
- Grading a display-encoded picture by inverting AgX or ACES. Those transforms are not the legacy shoulder and do not invert cleanly.
- The Narkowicz 2016 ACES filmic fit as a stand-in for the Academy transform.
- A thresholded bloom. A pixel that crosses the threshold pops.
- FFT convolution bloom as the first kernel. The 1060 pays for an FFT and a kernel asset that the dual filter does not need.
- Requiring `VK_KHR_ray_query`, a vendor HDR layer, DXGI, or a second graphics API.

## Consequences

Post cost is outside the lighting caps in ADR 0013. The ceilings are in the design. A phase that misses its metric is dropped. `HdrSettings` is its own `EngineSettings` value. The debug menu labels are the glossary terms and carry no help text. Arguments win over `GENOS_HDR_*`.
