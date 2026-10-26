# HDR output and bloom

Status: **accepted** (Engine bot, 2026-10-26). Anthony Poschen delegated design approval to the Engine bot. ADR: [0015](../adr/0015-hdr-output.md). Terms follow [`docs/lighting/GLOSSARY.md`](../lighting/GLOSSARY.md). The CPU references live in `crates/render/src/hdr_output.rs`. They are not dispatched. A later phase edits [Renderer](renderer.md) in the same change that changes a game use or a limit.

The metric gate is `crates/render/tests/hdr_output_gate.rs`. It calls those references. Where the pass does not call one yet, the live line is `NOT-IMPLEMENTED`. It does not invent a picture.

## 1. Goals and non-goals

**Goals**

- The light the GI passes compute stays scene-referred until a display pass. A pixel brighter than 1 is stored, not folded into a shoulder.
- Exposure is one scale. Auto-exposure reads a histogram. A camera (aperture, shutter, ISO) writes the same scale. A still scene does not hunt.
- The default display transform is AgX. ACES is selectable. The legacy shoulder remains until the byte gate no longer needs it.
- Linux (Wayland) prefers HDR10, then scRGB. Windows prefers scRGB, then HDR10. SDR is the fallback. macOS stays on the one Vulkan path. Hyprland and Mesa version claims are unverified (§4.3).
- Bloom is a wide, thresholdless, energy-preserving filter on the scene-referred image.
- The path-trace grade keeps scoring linear Rec.709 radiance. The display transform is not in that grade.

**Non-goals**

- Depth of field, motion blur, TAA, lens dirt, streaks, anamorphic flares.
- Spencer, Shirley, Zimmerman, and Greenberg, "Physically-Based Glare Effects for Digital Images", SIGGRAPH 1995. That is the eye's point-spread function. This pass is not that.
- An FFT convolution bloom (Unreal's convolution bloom) as the first kernel.
- The Narkowicz 2016 rational fit, used as if it were ACES.
- Writing exposure or bloom into the surface light cache, the world radiance cache, or a screen probe.
- Retuning the GI fail line in [the reference check](lighting/reference-check.md). That line stays 0.12.
- Requiring `VK_KHR_ray_query`. The software tracer is unchanged. This pass does not trace.
- A vendor HDR layer, DXGI, or a Metal swapchain. [ADR 0003](../adr/0003-one-vulkan-renderer-moltenvk-on-macos.md) and [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) stay in force.
- Help text on the debug menu. Labels are glossary terms.
- Changing the lamp unit (72) into nits. Nits are a display encoding. The grade stays in the lamp unit.

## 2. What the tree does today

Inventory, read before this design. Nothing here is replaced until the phase that names it.

| Piece | Where | Fact |
|---|---|---|
| Display curve | `tone_channel` in `crates/render/shaders/lighting_compose.comp` and `lighting_sun_compose.comp`; `mesh::tone` in `crates/render/src/mesh.rs` | Identity at or below 0.64. Above that, `0.64 + 0.14 × (x − 0.64) / (x − 0.64 + 1.1)`. The asymptote is 0.78. |
| Inverse | `linear_from_display` in `crates/render/src/trace.rs` | Undoes that shoulder. The grade in `crates/lighting-ref` uses it, then Rec.709 luminance `(0.2126, 0.7152, 0.0722)`. |
| Dither | `crates/render/shaders/dither.glsl` | Triangular noise of at most one 8-bit code, shrunk to nothing at 0 and at 1. |
| Swapchain | `create_targets` in `crates/render/src/gpu.rs` | Prefers format 44, else 37. Color space 0. The scene color image uses the same format. Two frames in flight. |
| Present | `renderer.md` | Mailbox, else immediate, else FIFO. |
| Antialias | `crates/render/src/aa.rs` | `off` or FXAA, on the display image, before the overlay. |
| Grade | [reference-check.md](lighting/reference-check.md) | Blurred relative error `\|Yc − Yr\| / max(Yr, 0.25 × mean(Yr), 0.02)`. Fail line 0.12. Not a per-scene nudge. |
| Post sketch | [extras-plan.md](rendering/extras-plan.md) step 7 | `tone(exposure × (linear + bloom))`. Exposure 1 and bloom 0 match the old bytes. The pyramid runs when the scene key changes. |
| Lighting caps | [ADR 0013](../adr/0013-lighting-invalidation-is-local.md) | Per-preset work caps. High lighting cap 8 ms at 1080p on a 4070. Post is not part of that cap. |
| Stored light | [ADR 0012](../adr/0012-gi.md) | What is stored does not depend on the camera. The camera only orders work. |

A value of 16 becomes about 0.771 through the shoulder. The inverse can rebuild 16 from that code. AgX and ACES cannot be undone the same way. That is why the grade has to move to the scene-referred image before those transforms ship.

## 3. Fit with the renderer and with GI

```text
G-buffer
  → direct, probes, surface light cache, compose
  → scene-referred image          RGBA16F, Rec.709 linear, no exposure, no bloom, no curve
  → histogram                     log luminance, tails clipped, after the previous frame's fence
  → bloom pyramid                 only when the highlight check says the image changed
  → exposed image                 (scene + bloom) × exposure
  → display transform             AgX, ACES, or the legacy shoulder
  → swapchain encode              SDR UNORM, HDR10 PQ, or scRGB linear
  → FXAA on the 8-bit image       skipped on 10-bit and on 16-bit until that shader is widened
  → overlay                       not graded, not bloomed
```

The surface light cache, the world radiance cache, and the screen probes are upstream of the scene-referred image. They never read it. Exposure is a camera quantity. Bloom is a lens quantity. ADR 0012 forbids both from becoming stored light.

Compose reuse stays. A quiet 8×8 block keeps the radiance it already holds. The histogram and the bloom check see that held image. They do not force a full compose.

Frame reuse, as the extras plan describes it, skips the pyramid when the scene key matches and the highlight check agrees. The check still runs. A skip is a result of the check, which is the rule in the lighting record (S74A) and in ADR 0013.

The lighting work caps are unchanged. Post has its own ceilings in §7. A slow bloom does not borrow rays from the cache, and a lighting frame does not skip the display pass to pay for GI.

Presets stay the lighting presets. `HdrSettings` is a second struct on the scene, not new fields inside `LightingSettings`, so a lighting preset replace cannot turn bloom on inside the cache. Potato and Low force SDR output. Medium and High use `output=auto`. Every preset keeps the scene-referred image, including Potato, so the grade is the same buffer on every preset. Potato does not run the pyramid or the histogram. Its exposure is fixed.

Near the camera still matters most, and it matters in the grade, not by allocating a finer bloom. Pixels whose G-buffer distance is under 2 m have weight 4 in the scene-referred score. Far pixels have weight 1. Bloom is half resolution and coarser down the pyramid on every preset. That is the Jimenez pyramid, not a camera-dependent kernel. A camera-dependent kernel would change the filter as the camera moves, which is the pop this design is written to avoid.

## 4. Alternatives

### 4.1 Display transform

| | Legacy shoulder (today) | ACES 2.0 output transform | **AgX** (pick) |
|---|---|---|---|
| Source | `mesh::tone` | Academy Software Foundation, ACES 2.0 Output Transform, 2024, [acescentral.com](https://acescentral.com/) | Troy Sobotka, AgX; Blender's formation by Eary Chow, [AgX_LUT_Gen](https://github.com/EaryChow/AgX_LUT_Gen) |
| Shape | Identity, then a shoulder that stops near 0.78 | Academy rendering transform, sRGB ODT or a nit ODT | Inset into a working space, log sigmoid, outset |
| Hue | Not a hue model. Channels are independent | ACES 1.x skewed primaries toward the "notorious six". 2.0 is the Academy's correction | The inset is there so a primary does not collapse to white |
| Invert for the grade | Yes, below the asymptote | No | No |
| Real-time cost | A few flops | A full ODT, heavier | A 3×3, a log, a sigmoid, a 3×3 |

**Pick: AgX base**, the Eary Chow generator ([AgX_LUT_Gen](https://github.com/EaryChow/AgX_LUT_Gen) `AgX_Base_Rec2020`, then the sRGB cube path). Log2 runs from −10 to +6.5 around middle grey 0.18. The sigmoid slope is 2.4 and both powers are 1.5. The hue mix is 40%. The SDR code is power 2.4, which is what that cube stores. The CPU reference is `agx_rec709`. The gate compares it to generator samples, max CIEDE2000 ≤ 0.5.

**Departure:** `legacy_bytes` binds the plain compose compile. That SPIR-V is the pre-HDR compose shader, with no scene-image or bloom binding. The HDR compile (`GENOS_HDR_COMPOSE`) stores scene radiance, adds bloom, applies exposure, and writes the shoulder. The lattice sample is a later pass (`hdr_agx.comp`) that samples `agx_rec709`. It is not a branch in the GI compose shader.

**Rejected as the implementation of this pick:** Krzysztof Narkowicz, "ACES Filmic Tone Mapping Curve", 2016, [knarkowicz.wordpress.com](https://knarkowicz.wordpress.com/2016/01/06/aces-filmic-tone-mapping-curve/). The coefficients `(2.51, 0.03, 2.43, 0.59, 0.14)` are a rational fit of an older ACES curve, then a clamp to 1. Shipping that fit and calling it ACES or AgX would be a silent departure. `display=aces` is the Academy 2.0 transform, not this fit. It lands in phase 5's alternate, after AgX is the default.

**Departure from a community GLSL port** that quotes `min_ev = −12.47393`, `max_ev = 4.026069`. Those are not the generator's limits. We use −10 and +6.5.

**Departure from Blender's IEC sRGB piece.** The generator's cube encodes with power 2.4, not the piecewise sRGB OETF. SDR uses that power so the gate matches the LUT. HDR keeps the same formation through the linear Rec.709 result, then the nit map in §4.3. The polynomial sigmoid in community ports that quote a mean squared error against the LUT is a fit. We do not use it.

### 4.2 Exposure

| | Fixed scale 1 (today) | Reinhard log-average | **Histogram plus a camera** (pick) |
|---|---|---|---|
| Source | exposure is absent; the shoulder is the only map | Erik Reinhard, Michael Stark, Peter Shirley, James Ferwerda, "Photographic Tone Reproduction for Digital Images", SIGGRAPH 2002 | Sébastien Lagarde and Charles de Rousiers, "Moving Frostbite to Physically Based Rendering", SIGGRAPH 2014, §5.1. The same formulas are in Filament `Exposure.cpp` |
| Meter | none | one log-average of luminance | 256-bin log histogram, drop the lowest 10% and the highest 10% of samples |
| Camera | none | none | `EV100 = log2(N² / t × 100 / S)`, then the same scale |
| Still scene | stable, because nothing moves | a single firefly moves the mean | the tail clip and a hold (§6) keep it still |

The scale, from the Frostbite notes and from Filament, with the saturation-based speed and `q = 0.65`:

```text
Lmax     = 1.2 × 2^EV100
exposure = 1 / Lmax
EV100    = log2(L × 100 / 12.5)          meter, K = 12.5 (ISO 2720)
EV100    = log2(N² / t × 100 / S)        camera
```

`K = 12.5` is the Canon / Nikon / Sekonic constant the course adopts. The metered luminance lands near a tenth of sensor saturation, not at 0.18. That headroom is the course's half stop (`18% / √2 ≈ 12.7%`, and the `1.2` factor moves it a little further). **Departure:** we do not add a second scale to pin the meter on 0.18. Doing so would leave the cited formula.

**Departure from Reinhard 2002:** the meter is a histogram with a 10% clip on each tail, not one log-average. A sun that covers one pixel must not move EV100 by more than 0.10 on the firefly fixture in §8. Reinhard's key is the idea we keep. His single mean is not.

Auto and camera meet at EV100. Compensation adds stops after that: `exposure ×= 2^compensation`. Positive compensation brightens. One meaning.

### 4.3 Swapchain

| | SDR UNORM (today) | HDR10 (Linux / Wayland pick) | **scRGB (Windows pick)** |
|---|---|---|---|
| Format | 44 or 37, color space 0 | 10-bit UNORM (`A2B10G10R10` 64, else `A2R10G10B10` 58) with `VK_COLOR_SPACE_HDR10_ST2084_EXT` (1000104008) | `R16G16B16A16_SFLOAT` (97) with `VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT` (1000104002) |
| Encoding | the display transform writes the 8-bit code | PQ, SMPTE ST 2084:2014, BT.2100 | linear extended sRGB. Code 1.0 is 80 nits |
| Extra bandwidth at 1440p, 3 images | already paid | none (4 bytes, same as SDR) | 42.19 MiB over the SDR images |

The Vulkan wording is the `VkColorSpaceKHR` table in the `VK_EXT_swapchain_colorspace` extension. HDR10 is "HDR10 (BT.2020), encoded according to SMPTE ST 2084". Extended sRGB linear is "extended sRGB color space, encoded using a linear transfer function". The nonlinear extended-sRGB enum (1000104014) is the scRGB specification's other encoding. We do not pick it. Windows literature for the linear one is `DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709`. We do not call DXGI.

**Pick:** query `vkGetPhysicalDeviceSurfaceFormatsKHR` after the surface exists. On Linux (Wayland) prefer HDR10 when that pair is listed, else scRGB, else the current SDR pair (44, then 37, color space 0). On Windows prefer scRGB, else HDR10, else SDR. Potato and Low ignore HDR pairs and take SDR. Medium and High follow the host preference when `output=auto`. macOS takes the first HDR pair MoltenVK listed, else SDR. The reference is `pick_hdr_output`.

**Departure:** `create_targets` records that pick (`resolve_swapchain` through `surface_choice`) and computes graphics-white codes with `swapchain_nits` (`pq_st2084`, `scrgb_code`). The SDR search is still 44, then 37, and it does not read the color space. When the resolved pair is listed, the image is that pair: `A2B10G10R10` or `A2R10G10B10` with `HDR10_ST2084`, or `R16G16B16A16_SFLOAT` with `EXTENDED_SRGB_LINEAR`. `VK_EXT_swapchain_colorspace` is enabled when the instance lists it. `VK_EXT_hdr_metadata` is enabled when the device lists it, and `vkSetHdrMetadataEXT` runs only for a created HDR10 image (BT.2020 primaries, peak 1000 nits). An unlisted mode stays on the SDR image. The display pass still writes the 8-bit `AgX` code for that fallback, and it also writes the PQ or scRGB values into a side buffer so a readback can grade them. A listed 10-bit image is packed in that pass. A listed fp16 image is copied from the half-float tail of the same buffer.

**Why the split.** HDR10 is PQ in a 10-bit UNORM image, which is the usual Wayland HDR advertisement. scRGB stores linear extended sRGB, which is the Windows linear HDR path (`DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709` in the Windows literature; we do not call DXGI). This preference is a decision, not a driver measurement.

**Nit mapping, HDR only.** SDR runs AgX through to the power-2.4 code. HDR10 and scRGB run the same inset, log, and sigmoid, then map graphics white to 203 nits (ITU-R BT.2408) on both:

- scRGB: code 1.0 stays 80 nits. Graphics white is `203/80 = 2.5375`, written 2.54. The sigmoid's top maps to the display peak, or to 1000 nits when `VK_EXT_hdr_metadata` does not report a peak.
- HDR10: the same 203 nits, then PQ into 10-bit BT.2020. The sigmoid's top maps to the same peak.

**Departure:** Blender never emits nits. The inset, the log range, and the sigmoid are the generator. The last stage is ours. SDR does not take this departure.

Whether Hyprland lists these color spaces, from which version, or with `cm = "hdr"` and `bitdepth = 10`, is unverified. Whether Mesa or a current NVIDIA driver exposes `VK_EXT_swapchain_colorspace` without `VK_hdr_layer` is unverified. The renderer does not configure the compositor. If the surface list has no HDR space, the mode is SDR and the probe records the list. We do not vendor `VK_hdr_layer`. An older driver that needs the layer stays on SDR.

macOS uses MoltenVK ([ADR 0003](../adr/0003-one-vulkan-renderer-moltenvk-on-macos.md)). The same query runs. An empty HDR list is SDR. The software tracer is the ray path on that machine. It is not a second display path.

### 4.4 Bloom

| | Kawase iterations | UE convolution bloom | **Dual filter** (pick) |
|---|---|---|---|
| Source | Masaki Kawase, "Frame Buffer Postprocessing Effects in DOUBLE-S.T.E.A.L", GDC 2003 | Epic, Unreal Engine convolution bloom (FFT of a kernel image) | Jorge Jimenez, "Next Generation Post Processing in Call of Duty: Advanced Warfare", SIGGRAPH 2014, [iryoku.com](https://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare/) |
| Kernel | a few wide taps, repeated | any authored PSF, in frequency space | 13-tap downsample, 9-tap tent upsample |
| Energy on a flat field | not the published contract | the kernel's | the 13 taps sum to 1 |
| Fireflies | a bright pixel flickers as it crosses texels | the kernel can ring | Karis weight on the first downsample only |
| Cost | small | an FFT, too much for the 1060 as a default | a pyramid that starts at half resolution |

The 13-tap weights, which sum to 1:

```text
centre                  0.125
four taps at (±1, ±1)   0.125 each
four taps at (±2, 0)    0.0625 each
four taps at (±2, ±2)   0.03125 each
```

The first downsample groups those taps into five 2×2 boxes. The inner box has weight 0.5. Each outer box has weight 0.125. Each box is weighted by `1 / (1 + Y)` with Rec.709 `Y`, then the boxes are renormalized. That weight is Brian Karis, "High Quality Bloom", in the same 2014 course. Later levels use the linear 13-tap.

The upsample is a 3×3 tent: 0.25 at the centre, 0.125 on the edges, 0.0625 on the corners, added onto the finer level.

**Pick: the dual filter.** It is thresholdless, the flat-field sum is 1, and the widest level is a handful of pixels. "Physically based" in this design means those three facts plus the order: bloom runs on the scene-referred image, before exposure, so a linear kernel commutes with the exposure scale. It does not mean Spencer's eye PSF, and it does not mean a measured lens.

**Departures from Jimenez 2014**

- The pyramid runs when the highlight check says the scene-referred max moved by more than half a stop, or the scene key changed. The course shows a pyramid every frame. A reused pyramid is the frame-reuse rule, not a different kernel.
- There is no artist threshold and no soft knee. A threshold pops. The slides argue for thresholdless; some ports add a knee anyway. We do not.
- The Karis luma is Rec.709, the same weights as `luminance` in `trace.rs`. The course does not pin the coefficients.
- Karis does not commute with exposure. It runs on scene luminance, before exposure, so a later exposure change does not reshape the firefly weight. Named, because a purely linear filter would commute.
- The pyramid is a GPU pass on the scene image compose just stored (`hdr_bloom.comp`), same frame, before the display pass. It follows `bloom_pyramid`: Karis, then the 13-tap, then the tent, bloom only. `bloom_pyramid` stays the CPU reference the gate grades. Intensity 0 does not read the buffer. The histogram is still one frame late.

**Rejected:** Kawase, because the energy contract is weaker and a single bright pixel is less stable. Convolution, because the FFT and the kernel asset do not pay for themselves on a GTX 1060, and because a lens-dirt texture is a non-goal. Either can be a later mode once the dual filter has a measured cost. The mode would be a new glossary term in that change.

## 5. Data structures and passes

`HdrSettings` implements `EngineSettings` and lives on the scene next to `LightingSettings`. Keys:

| Key | Values | Default on High |
|---|---|---|
| `exposure_mode` | `auto`, `camera`, `fixed` | `auto` |
| `ev100` | float | the meter; `fixed` uses the stored value |
| `compensation` | stops | 0 |
| `iso` | float | 100 |
| `aperture` | f-number | 4 |
| `shutter` | seconds | 1/60 |
| `adapt_down` | seconds, exposure falling (scene got brighter) | 0.40 |
| `adapt_up` | seconds, exposure rising (scene got darker) | 0.80 |
| `display` | `agx`, `aces`, `legacy` | `agx`. `aces` presents the legacy shoulder until `aces2_rec709` is the Academy CTL |
| `bloom` | `off`, `on` | `on` from phase 4; `off` on Potato |
| `bloom_intensity` | float, scales the pyramid add | 0.05 (0 on Potato) |
| `bloom_levels` | 0..6 | 6 on High, 5 Medium, 4 Low, 0 Potato |
| `output` | `auto`, `sdr`, `hdr10`, `scrgb` | `auto` on Medium and High; forced `sdr` on Potato and Low |

GPU images, all `R16G16B16A16_SFLOAT` unless noted:

| Image | Size at 1440p | Notes |
|---|---|---|
| Scene-referred | 2560×1440, one image | Not per frame in flight. Compose writes it. The grade reads it. |
| Bloom levels | 1280×720 down to 40×22 | The whole pyramid is kept so a reused frame samples it. |
| Exposed | same as the scene, or the swapchain's transient | The display pass can read the scene and the bloom sum and write the swapchain. A stored exposed image is not required for SDR. |
| Histogram | 256 × `uint` | Device buffer. The CPU reads it after the fence. |
| Exposure state | two floats | Current scale and target scale. CPU owned. |

Passes, in order, each skipped when its inputs are off:

1. **Compose** writes radiance. It does not call `tone_channel`. Emission, direct, and indirect stay in lamp units.
2. **Histogram.** One compute over the scene image. Log2 luminance, bins from −16 to +16. The CPU clip and the EV100 math run on the 256 counts, after the fence, one frame late. `Renderer::draw` does not wait.
3. **Highlight check.** The top occupied bin. If it moved by half a stop or less and the scene key matches, the pyramid is not dispatched.
4. **Downsample**, then **upsample**, adding onto the finer level with `bloom_intensity`.
5. **Expose and display.** `(scene + bloom) × exposure`, then AgX, ACES, or the legacy shoulder, then the swapchain encoding. SDR applies `dither8`. HDR10 applies PQ. scRGB writes linear nits.
6. **FXAA** when the picture mode is FXAA and the swapchain is 8-bit.
7. **Overlay.**

Adaptation is exponential on the exposure scale:

```text
e += (target − e) × (1 − exp(−dt / τ))
```

`τ` is `adapt_down` when the target is below `e`, and `adapt_up` when it is above. When `|target − e| / max(e, 10⁻⁶) < 0.001`, `e` is held. **Departure from a pure exponential:** the hold is ours. It is the no-flicker rule. A still histogram must not move the picture by a ulp per frame.

The one-frame latency of the histogram is a departure from a textbook meter that finishes inside the frame. It is how the draw stays free of a fence wait, which [renderer.md](renderer.md) already requires.

## 6. Presets

| | Potato | Low | Medium | High |
|---|---|---|---|---|
| Who | a floor card, no GI | GTX 1060 6 GB, RX 580 | mid | RTX 4070 |
| Exposure | `fixed`, EV100 0, compensation 0 | `auto`, 64 bins | `auto`, 128 bins | `auto`, 256 bins |
| Adapt (down / up) | hold | 0.50 s / 1.00 s | 0.40 s / 0.80 s | 0.40 s / 0.80 s |
| Bloom levels | 0 | 4 | 5 | 6 |
| Display | legacy, then AgX when phase 5 flips the default | same | same | same |
| Output | SDR | SDR | auto | auto |

Low's histogram is 64 bins so the reduction is smaller. The EV100 formula does not change. A bin is coarser, and the meter tolerance in §8 (0.10 stops) still holds. **Departure:** the course does not specify a 64-bin meter. The clip stays 10% on each tail.

## 7. VRAM and milliseconds

1440p is 2560×1440. Bytes are MiB (`÷ 1024²`). These milliseconds are ceilings. `post_ms_device` is the bloom and display timestamp. `post_vram_device` is the post allocation. The 1.50 ms ceiling is the 4070 at 1440p. Lavapipe reports the timestamp and is not that score.

Scene image: `2560 × 1440 × 8 = 28.125 MiB`.

Bloom pyramid:

| Level | Size | MiB |
|---|---|---|
| 1 | 1280×720 | 7.031 |
| 2 | 640×360 | 1.758 |
| 3 | 320×180 | 0.439 |
| 4 | 160×90 | 0.110 |
| 5 | 80×45 | 0.027 |
| 6 | 40×22 | 0.007 |

| | Potato | Low | Medium | High |
|---|---|---|---|---|
| Scene image | 28.125 | 28.125 | 28.125 | 28.125 |
| Bloom | 0 | 9.338 | 9.366 | 9.373 |
| Histogram | 0 | < 0.01 | < 0.01 | < 0.01 |
| **Working set** | **28.125** | **37.463** | **37.491** | **37.498** |
| Swapchain extra | 0 | 0 | 0, or 42.188 if scRGB is chosen | 42.188 if scRGB, else 0 |
| **With scRGB** | 28.125 | 37.463 (SDR forced) | **79.678** | **79.685** |
| ms ceiling, 4070, 1440p | 0.25 | 0.80 | 1.00 | 1.20 |
| ms ceiling, 1060, 1440p | 1.00 | 2.50 | not the target | not the target |

The 4070 ceilings assume a bandwidth-bound fullscreen pass. A 1440p RGBA16F read plus write is about 59 MB. At 504 GB/s that is under 0.2 ms before cache effects. The pyramid's taps are about 150 MB of traffic, under half a millisecond on paper. The ceilings sit above those floors so a phase that lands at the floor still passes, and a phase that doubles the traffic still has a number to fail.

Frame budgets, using numbers already in the tree:

- Basic scene, above 500 fps, means a frame under 2.0 ms. High post 1.20 ms plus the frozen lighting floor of 0.43 ms from ADR 0013 is 1.63 ms.
- Stress scene, about 100 fps, means a frame under 10 ms. High post 1.20 ms plus the ADR 0013 High lighting cap of 8 ms at 1080p is 9.20 ms. The 8 ms cap is not a 1440p measurement. If 1440p lighting exceeds 8.80 ms, the lighting cap moves. This design does not raise the post ceiling to pay for it.
The three-decimal rows are the unrounded sums, then rounded. Medium with scRGB is 79.678 MiB. High is 79.685 MiB. Rounding those to two decimals is 79.68 and 79.69. Adding the coarser parts `28.13 + 9.37 + 42.19` made both columns 79.69, because Medium bloom is 9.366.

- 4 GB is the minimum. High with scRGB is 79.685 MiB of post, plus the lighting soft budget of about 200 MB. The sum stays under 512 MB. The rest of the 4 GB is the working set the renderer already has.

A steady basic scene on Potato or on a held High frame dispatches the display pass only. The pyramid and the histogram reduction are off or reused. That is the path that has to leave room for 500 fps.

## 8. Success metrics

Each metric has one threshold. The gate prints `name value threshold PASS`, `FAIL`, or `NOT-IMPLEMENTED`. `FAIL` exits non-zero. `NOT-IMPLEMENTED` does not. A check that has no engine input is not given a passing number.

The relative luminance error is the reference-check formula, on Rec.709 `Y`:

```text
|Yc − Yr| / max(Yr, 0.25 × mean(Yr), 0.02)
```

| Metric | Threshold | What it grades | Live, this tree |
|---|---|---|---|
| `legacy_inverse` | ≤ 1e−4 | `linear_from_display(tone(x))` against `x` for 0, 0.25, 0.64, 1, 4, 16. `tone` is `mesh::tone`. Relative, denominator `max(x, 1)` | PASS |
| `legacy_shader` | = 0 codes | lavapipe readback of the shipped hall, Potato, `legacy_bytes`, ambient raised to 8 so the shoulder is in frame, against the stored plain-compose picture. The shoulder in that shader is the shared `tone_channel`. Two settled frames must match | PASS. Moving a colour byte by 8, or changing `tone_channel`, fails |
| `gpu_legacy` | ≤ 1 code | the same hall through `hdr_agx.comp` mode 0 (`display=legacy`, not `legacy_bytes`), against `tone` of exposure times the scene image. Dither may move one code. A pixel above the shoulder start must be in frame | PASS. Changing the shared shoulder fails |
| `exposure_ev` | ≤ 1e−9 | `ev100_from_luminance` and `exposure_from_ev100` on 0.01, 0.18, 1, and 12.5. EV100 is `log2(L × 100 / 12.5)`. Exposure is `1 / (1.2 × 2^EV100)` | PASS |
| `exposure_histogram` | ≤ 0.10 stops | `histogram_ev100` on a grey field with a one-pixel sun, and on a field whose top 9% is that sun, against `log2(0.18 × 100 / 12.5)`. The 9% sits inside the dropped tail | PASS. Removing the 10% drop fails the heavy tail |
| `exposure_still` | ≤ 0.001 | max consecutive relative change of exposure over 8 identical frames | PASS. `exposure_still_change` holds on `step_exposure` |
| `exposure_adapt` | outside `[0.40, 2.00]` s is 0 | time to cover 90% of a 4-stop step. The value is the distance outside that window | PASS. `exposure_adapt_outside` at τ 0.40 s |
| `exposure_overshoot` | ≤ 0.02 | progress past the target on that step | PASS. `exposure_overshoot` on the same step |
| `physical_camera` | ≤ 1e−9 | `exposure_from_camera` against `1 / (1.2 × N² / t × 100 / S)` | PASS |
| `bloom_energy` | ≤ 1e−4 | `bloom_pyramid` against `karis_downsample_rgb`, `bloom_downsample`, and `bloom_upsample` on a flat field | PASS. Doubling a channel inside `bloom_pyramid` fails |
| `bloom_firefly` | ≤ 1.0 | first downsample of a white pixel of 1000 in a field of zeros, Karis on | PASS. `karis_firefly_response(1000)`, which calls `karis_weight`. A box filter (`1000 × 0.125`) fails |
| `bloom_commute` | ≤ 0.01 | `\|bloom(k x) − k bloom(x)\|` for the linear 13-tap | PASS. `bloom_commute_error` calls `bloom_downsample`. A 0.125 residual fails |
| `gpu_agx` | ≤ 0.02 | lavapipe readback of one fixed hall frame against `agx_rec709(exposure × (scene + bloom × k))`. Exposure is `step_exposure` from EV100 2, not the uploaded scale. Measured residual on that frame is 0.0025; a ×4 bloom add moves it by about 0.20 | PASS when the picture matches. A skipped `AgX`, a wrong lattice, a ×4 bloom add, or an exposure left at 1 fails |
| `gpu_exposure` | ≤ 1e−4 | `Renderer::hdr_exposure` against that same `step_exposure` | PASS. A scale pinned at 1 fails |
| `gpu_bloom` | ≤ 1e−3 | GPU bloom buffer of the current frame against that same tap pyramid. The graded frame changes the lamp, so the previous frame's pyramid does not match | PASS. A pyramid that doubles energy fails |
| `hdr_encode` | ≤ 0.02 | lavapipe readback of the encode buffer on the shipped hall, Medium with GI off, fixed EV100 2, bloom off, `output=hdr10`, against `hdr10_pq(agx_display_linear(exposure × scene))`. Paper white is 203 nits. The surface is the SDR pair or a listed HDR10 pair | PASS. An 80-nit paper white fails |
| `hdr_scrgb` | ≤ 0.02 | the same frame with `output=scrgb`, against `scrgb_from_display` of that display-linear colour. Code 1.0 stays 80 nits. The surface is the SDR pair or a listed scRGB pair | PASS. An 80-nit paper white fails |
| `output_pick` | = 0 mismatches | Linux: HDR10, else scRGB, else SDR. Windows: scRGB, else HDR10, else SDR. macOS: first listed HDR pair, else SDR. Potato and Low force SDR. An asked HDR pair that is not listed is SDR | PASS. `pick_hdr_output` and `resolve_swapchain`. Forcing HDR10 when it is not listed fails |
| `agx_de` | ≤ 0.5 | max CIEDE2000 of `agx_rec709` against `AgX_LUT_Gen` sRGB-cube samples | PASS |
| `agx_highlight` | outside `[0.998, 0.9995]` is 0 | power-2.4 code of linear 16. A hard clip to 1 fails | PASS |
| `aces_ctl` | ≤ 1e−3 | `aces2_rec709` against Academy CTL `Output.Academy.Rec709-D65_100nit_in_Rec709-D65_sRGB-Piecewise` | NOT-IMPLEMENTED. The function returns `None` until that CTL is the body |
| `pq_st2084` | ≤ 1e−7 | `pq_st2084` at 0, 100, 203, 1000, and 10000 nits against the ST 2084 inverse EOTF | PASS. Zero nits is `c1^m2`, not a hard 0 |
| `scrgb_code` | = 2.5375 | `scrgb_code(203)` against the literal 2.5375. The expected code is not recomputed as `203/80` | PASS. An 80-nit paper white (`80/80`) fails |
| `post_ms_device` | ≤ 1.50 on the 4070 | `hdr_ms`, bloom plus the display pass, one frame behind. Zero when the device has no timestamps. Lavapipe prints the timestamp and passes when the query returned. It does not apply the 1.50 ceiling | PASS when the timestamp is finite and positive. A missing timestamp fails |
| `post_vram_device` | ≤ 96 | post buffers in MiB: scene image, host copy, bloom, encode, lattice, params | PASS. The sum is the allocation |

The §7 ceilings are not rows. They are not measured, and comparing them to themselves is not a result.

`scene_peak`, `linear_grade`, and `grade_is_linear` graded `scene_referred` against itself, so a shader change could not fail them. `near_error` was `near_grade(0, 0.20)`, which is 0.04 on every build. `still_pop` compared two equal flats. Those rows are gone. `still_pop_share` stays on the renderer probe. GI keeps 0.12 in the lighting reference check. A GI miss fails there.

## 9. Debug tool and tests

HDR output is a renderer pass, not a new system. It does not add `debug/hdr/`. The lighting binary (`debug/lighting`) gains an Output page when phase 1 lands. The page is the shared scaffolding: the scene picker, checkpoints, reset, replay, screenshots, sliders, the guided-review panel, and the headless CLI. No new binary.

The menu draws glossary terms. Exposure mode, display, and output are dropdowns. EV100, compensation, ISO, aperture, shutter, bloom intensity, and bloom levels are sliders. There is no description string and no tooltip. The exposure phase draws this page and serves `get_hdr`. The surface list is the swapchain pair in use. Swapchain selection fills that list. The output row is labeled `SDR output` because that is the glossary headword for the swapchain image; the choices are `auto`, `sdr`, `hdr10`, and `scrgb`.

Probes, one per failure mode. They land with the phase that can fail that way. Names are the MCP catalogue in `crates/debug/src/commands.rs`, so MCP, `POST /cmd`, and Rhai share the JSON.

| Failure | Probe |
|---|---|
| Exposure hunts on a still scene | `get_hdr` returns `ev100`, `exposure`, and the last 8 scales. `exposure_still` reads those |
| The histogram fence stalls the frame | `get_hdr` returns `hdr_ms` and `post_ms_device`, the bloom and display timestamp, one frame behind. Zero when the device has no timestamps. The read stays after the fence. `post_vram_device` is the post buffers in MiB |
| A sun pixel pins the meter | histogram dump, 256 counts, under `./genos-debug/hdr-output/`. `exposure_ev` |
| A one-pixel firefly flickers in bloom | bloom level 1 dump. `bloom_firefly` |
| Bloom pops | `still_pop_share` on subsampled scene luma, reported by `get_hdr`. The gate does not compare two equal flats |
| The grade ran on a display image | `gpu_agx` grades the picture against `agx_rec709` of the scene image. `get_hdr` records `grade=scene` |
| HDR was asked and the surface is SDR | `get_hdr` lists every `(format, color_space)` and the chosen mode |
| The legacy bytes moved | `legacy_shader`. The stored picture is the plain compose of the shipped hall |

`get_hdr_settings` and `set_hdr_settings` are the settings commands. Arguments:

```text
--exposure-mode auto|camera|fixed
--ev100 F
--compensation F
--iso F
--aperture F
--shutter F
--adapt-down F
--adapt-up F
--hdr-display agx|aces|legacy
--display agx|aces|legacy   (only when the value is one of those three; otherwise it is the Wayland display name)
--bloom on|off
--bloom-intensity F
--bloom-levels N
--output auto|sdr|hdr10|scrgb
--out DIR
```

`GENOS_HDR_EXPOSURE_MODE`, `GENOS_HDR_EV100`, `GENOS_HDR_COMPENSATION`, `GENOS_HDR_ISO`, `GENOS_HDR_APERTURE`, `GENOS_HDR_SHUTTER`, `GENOS_HDR_ADAPT_DOWN`, `GENOS_HDR_ADAPT_UP`, `GENOS_HDR_DISPLAY`, `GENOS_HDR_BLOOM`, `GENOS_HDR_BLOOM_INTENSITY`, `GENOS_HDR_BLOOM_LEVELS`, `GENOS_HDR_OUTPUT`, and `GENOS_HDR_OUT` mirror those arguments. The thirteen design-table keys are the first thirteen. `GENOS_HDR_OUT` mirrors `--out`. The argument wins. The variable applies only when the argument was omitted. `--help` lists each one. The debug menu does not. `get_hdr` and the lighting stats return `hdr_ms`, `post_ms_device`, `hdr_vram_bytes`, and `post_vram_device`. `post_ms_device` is `hdr_ms`: bloom plus the display pass, one frame behind, zero when the device has no timestamps. `post_vram_device` is the post buffers in MiB. The 1.50 ms ceiling is the 4070 at 1440p. Lavapipe time is not that score. The 96 MiB ceiling is the allocation.

Screenshots and the histogram dump go to `./genos-debug/hdr-output/` unless `--out` is set. Nothing is written under `$HOME`.

### Gate that exists now

```text
cargo test -p genos-render --test hdr_output_gate -- --out genos-debug/hdr-output
```

The test prints one line per metric (`LIVE` or `PROOF`, name, value, threshold, status) and writes `live.json`, `proof-good.json`, and `proof-broken.json`. `PROOF` known-good must be `PASS`. `PROOF` broken must be `FAIL`. A live `FAIL`, or a proof line on the wrong side, exits non-zero. `legacy_shader`, `gpu_agx`, `gpu_exposure`, `gpu_bloom`, `hdr_encode`, and `hdr_scrgb` open the proof window and read a lavapipe frame back. The other rows stay on the CPU references.

The broken fixtures call a mutated form of the real function: an exposure scale that drops the `1.2`, a meter that averages the sun in, `step_exposure` plus 0.05 a frame, an adaptor that snaps past the target, a camera scale that drops the `1.2`, the real pyramid with its red channel doubled, the firefly sum with the Karis weight left at 1, the commute check with `k` dropped, a picture graded as the linear color instead of `AgX`, an exposure pinned at 1, a bloom buffer scaled by 2, the encode buffer graded as an 80-nit paper white, a picker that always returns SDR, AgX replaced by white, a highlight clipped to 1 before `agx_rec709`, an ACES channel 0.2 off, PQ replaced by `nits/10000`, and an 80-nit paper white (`80/80` instead of `scrgb_code(203) = 2.5375`). The device lines of 0.40 ms, 37.5 MiB, 5 ms, and 4000 MiB are comparator fixtures for rows that have no engine input. They are not a 4070 measurement.

### Gates that wait for a phase

Rhai, on `genos-lighting`, fixed step 1/60, `--menu hidden`, report under `./genos-debug/hdr-output/`. They are not in this change. The host functions do not exist yet, and a script that called them would be a fake.

| Script | Scene | Pass |
|---|---|---|
| `hdr-bytes.rhai` | `room` | `display=legacy`, exposure fixed at the scale that matches today, bloom off. Mean 8-bit code within 1 of the pre-phase picture |
| `hdr-linear.rhai` | `room`, hall bench | read the scene-referred image, score `linear_grade` and `grade_is_linear` against the path tracer. No display transform in the score |
| `hdr-still.rhai` | `room`, camera held | `exposure_still` and `still_pop` |
| `hdr-sun.rhai` | a grey room and one emissive pixel | `exposure_ev` |
| `hdr-bloom.rhai` | one bright pixel | `bloom_firefly`, `bloom_energy` |
| `hdr-output.rhai` | `room` | `get_hdr` records the surface list and the chosen mode |

Owner commands, not run here:

```text
genos-lighting --scene room --preset high --size 2560x1440 --frames 300 \
  --display agx --exposure-mode auto --bloom on --output auto \
  --gpu-times 30 --out genos-debug/hdr-output
```

The 4070 at 1440p is the High timing machine. The 1060 is the Low timing machine. Lavapipe checks the wiring only. Timing on lavapipe is not a result.

The path tracer stays the GI reference. Bloom and the display transform are graded by the analytic metrics, not by widening the GI line and not by a visual override. A picture that fails `linear_grade` fails. A picture inside 0.02 does not get a second, kinder threshold.

## 10. Networking

- **Replicated state:** `HdrSettings` only. The scene image, the pyramid, the histogram, and the swapchain stay on the client that presents.
- **Authority:** that client. A server does not choose the display transform.
- **Prediction / rollback:** none. Exposure is presentation. A rollback does not replay the histogram.
- **Determinism:** the same settings and the same scene-referred image select the same EV100 on the CPU meter. The GPU histogram is not cross-device deterministic. The analytic gate is deterministic. Pixels are not.
- **Headless server:** no swapchain, no pyramid, no histogram. It can store the struct and forward it.

## 11. Papers and departures

Collected so a phase cannot drift in silence. Lighting departures also belong in the lighting record when a phase changes lighting. This pass does not.

| Source | Taken | Departure |
|---|---|---|
| Sobotka / Eary Chow AgX_LUT_Gen | Inset, log −10 to +6.5, sigmoid slope 2.4, 40% hue mix, power 2.4 code | HDR replaces that power encode with a 203-nit map. The −12.47393 to 4.026069 range is not used. The polynomial community fit is refused |
| ACES 2.0 | Alternate `display=aces`, the Academy transform | Not in the default. Not the Narkowicz fit |
| Lagarde and de Rousiers, SIGGRAPH 2014, §5.1; Filament `Exposure.cpp` | EV100, K = 12.5, `q = 0.65`, scale `1 / (1.2 × 2^EV100)` | Histogram tails instead of one mean. Hold at 0.001 relative. One frame of readback latency. No second scale onto 0.18 |
| Reinhard et al., SIGGRAPH 2002 | A log meter as the auto-exposure idea | Not a single log-average |
| Jimenez, SIGGRAPH 2014 | 13-tap, 9-tap, thresholdless pyramid, weights sum to 1 | Not every frame. No knee. Rec.709 Karis luma |
| Karis, same course | `1 / (1 + Y)` on the first downsample | Runs before exposure, so it does not commute with that scale |
| SMPTE ST 2084, BT.2100, BT.2408 | PQ, BT.2020, graphics white at 203 nits on HDR10 and on scRGB | scRGB code of that white is 2.5375, written 2.54. Code 1.0 stays 80 nits |
| Vulkan `VK_EXT_swapchain_colorspace` | The two enums and the SDR color space | Potato and Low force SDR even when HDR is listed |
| ADR 0012 | Stored light ignores the camera | Exposure and bloom are downstream of compose |
| ADR 0013 | Budgets are ceilings. A cheap check decides a skip | The check is the histogram max bin. Post ceilings are not the lighting caps |
| extras plan, step 7 | `tone(exposure × (linear + bloom))`, byte match at exposure 1 and bloom 0 | The grade reads the scene image before that product. AgX replaces the curve only in phase 5 |

## 12. Phases

Each phase is its own change, with the gate it names. A miss is dropped: a note, the branch deleted, the picture left as it was.

| Phase | Change | Gate |
|---|---|---|
| P0 | This design, the glossary, ADR 0015, the metric gate, and the CPU references the passes will call. No display dispatch | `hdr_output_gate` exits 0. Rows whose pass is not the reference stay `NOT-IMPLEMENTED` |
| P1 | Compose writes the scene-referred image. A display pass runs the legacy shoulder. FXAA stays on the display image | `legacy_inverse` still passes. `hdr-bytes.rhai` within 1 code. `display=legacy`, bloom off, fixed exposure |
| P2 | The reference check reads the scene image. `linear_from_display` is not used on `agx` or `aces` | `linear_grade`, `grade_is_linear`. The GI fail line is untouched |
| P3 | Histogram, camera, hold | `exposure_ev`, `exposure_still`, `exposure_adapt`, `exposure_overshoot`, `physical_camera` |
| P4 | Dual-filter bloom | `bloom_energy`, `bloom_firefly`, `bloom_commute`, `still_pop`. Intensity 0 matches P3 |
| P5 | AgX default. ACES alternate. Legacy remains a mode | `agx_de`, `agx_highlight`. `aces_ctl` once `aces2_rec709` is the CTL. `display=legacy` still matches P1 bytes |
| P6 | scRGB, HDR10, SDR fallback. Output page, `get_hdr` | `output_pick` against the live surface list. `post_ms_device`, `post_vram_device` on the 4070. 1060 runs Low |

P2 is before P5 on purpose. AgX is not invertible. Shipping it before the linear readback would make the path-trace grade a guess.

## 13. Open questions

1. On the owner's NVIDIA driver, does Hyprland list HDR color spaces without `VK_hdr_layer`? Phase 6 records the list. A missing list is SDR. We do not vendor the layer in this design.
2. Low forces SDR. HDR10 is 4 bytes, the same as today. If a 1060 timing shows the 10-bit encode inside the Low ceiling, Low can allow HDR10 and still refuse scRGB. That is a later measurement, not the default.
3. The HDR peak falls back to 1000 nits. A display that reports 400 nits should win. Phase 6 reads `VK_EXT_hdr_metadata` when the device has it.
4. FXAA's shader is 8-bit. Phase 6 skips it on HDR10 and on scRGB rather than widening it in the same change.
5. ADR 0013's 8 ms lighting cap is 1080p. The stress sum at 1440p is unmeasured. Post stays at 1.20 ms on High either way.
6. Whether `display=aces` ships in phase 5 or in a later phase depends on the Academy transform fitting in the display pass without a LUT asset. A LUT under `assets/` is a last resort and needs the asset record. A fit that is not the Academy's code does not ship under the name ACES.
