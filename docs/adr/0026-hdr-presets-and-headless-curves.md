# 0026: HDR follows the four presets, and its curves have a headless reference

Status: **accepted** (Engine bot, 29 Oct 2026). Anthony Poschen delegated design approval to the Engine bot; he reviews later. Design: [`docs/systems/rendering/hdr-pipeline.md`](../systems/rendering/hdr-pipeline.md). Builds on [ADR 0015](0015-hdr-output.md) and [ADR 0023](0023-graphics-settings-presets.md).

## Context

ADR 0015 decided the HDR pipeline: a scene-referred `R16G16B16A16_SFLOAT` image, EV100 exposure with a clipped log histogram, AgX as the default display transform with ACES 2.0 as an alternate, a thresholdless dual-filter bloom with the Karis first downsample, and HDR10 / scRGB / SDR swapchain selection. Its preset table uses Potato, Low, Medium and High.

ADR 0023 later fixed the shipped presets as Low, Medium, High and Ultra, with Low on a GTX 1060 and the Steam Deck. The HDR tables no longer name real presets.

The CPU references for the curves live in `crates/render/src/hdr_output.rs`. That crate's build compiles shaders, so a box without the GPU toolchain cannot run them, and the curves are only checked where a Vulkan build works.

## Decision

1. ADR 0015 stays the pipeline: scene-referred image, EV100 exposure, AgX default, ACES 2.0 alternate, dual-filter bloom, HDR10 and scRGB output with 203-nit graphics white. This ADR does not reopen those choices.
2. HDR maps onto the ADR 0023 presets. ADR 0015's Potato column becomes the **fixed-exposure fallback** used only when the device misses the Low gate; it is not a shipped preset. Low takes ADR 0015's Low column, Medium its Medium, High its High. Ultra equals High plus a 7th bloom level and HDR10 peak read from the display (ADR 0015 open question 3).
3. Low forces SDR output on desktop. The Steam Deck LCD forces SDR; the Deck OLED may take HDR10 when the compositor lists it, because the display pass costs the same.
4. The scalar curves (EV100, camera EV100, compensation, adaptation step, Karis weight, PQ ST 2084, the nit map, and the per-channel AgX curve) live in `genos-math::tonemap` and are tested there with published values. `cargo test -p genos-math` needs no GPU and no shader compiler.
5. `hdr_output.rs` calls `genos-math::tonemap` for those curves in a follow-up change, so there is one copy. Until then both copies must agree, and the follow-up deletes the render copy.

## Rejected

- Keeping Potato as a fifth preset. ADR 0023 fixed four, and a no-GI tier has no target device.
- Making ACES the default. AgX keeps hue under strong saturated light (lamps, fire) and the engine has no ACES grading workflow to match.
- A separate `genos-hdr` crate for five pure functions. `genos-math` already holds the medium and optics closed forms.
- Keeping the references only in `genos-render`. A reference that needs a shader compiler is not checked on every machine.

## Consequences

Post cost stays outside the lighting caps (ADR 0013). Per-preset ceilings are in the design and remain unmeasured until the 4070 run. A wrong PQ or exposure constant now fails a headless test before any GPU comparison.
