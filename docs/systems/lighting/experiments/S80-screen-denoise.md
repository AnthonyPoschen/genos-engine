# S80: Screen wavelet of visible fine texels

High's denoiser is a screen-sized spatial wavelet rebuilt each frame from this
frame's visible 12.5 cm texels. The hash-slot à-trous stays available
(`denoise_screen=0`, and full effort) and is the picture reference.

The cost of the hash wavelet follows occupied cache slots and probe-chain
length, not pixels. Stress filtered about 108k slots (11–15 ms on an RTX 4070
at 1920×1080). Sponza filtered about 800k of the million-slot table (93–98 ms).
S79 half-resolution and tiles kept that launch and got slower. Draft PR #38
estimated a coherent screen filter at about 1.4 ms on both scenes. This note
does not claim those milliseconds: lavapipe does not time the GPU.

## Method

Stage 0 gathers one hash find per pixel. The key is the geometric axis stored
in the G-buffer, not the shading normal. Empty texels and 25 cm cells are not
filtered. The input is `lighting_svgf_in` (the texel's own mean times its sun
gain). Stages 1..N are a B3 à-trous (Dammertz, HPG 2010: 1/16, 1/4, 3/8, 1/4,
1/16) on a ping-pong in the picture buffer. The step is the projected 12.5 cm
cell, times 2^i, clamped to 48 px. Depth, shading normal, and luminance stop
the blur. Compose reads the last iteration (slice 1) per pixel. Stage 6 still
writes the cell (`LIGHTING_WORLD_AT.y`, freshness in the à-trous stamp) for
the change test and for pixels the wavelet skipped. Showing that cell is one
color per 12.5 cm, which is the banner's block, so the picture does not.
Certainty below 1 blends the 25 cm running mean into the cell write (Gortler
pull, min(1, 4 wm / wall)).

A two-sided back face (`gl_FrontFacing` false) stores the opposite axis, so
the front and the back do not share a cell. `gl_FrontFacing` does not follow
the normal map, so a small camera move on one side of a cloth keeps the key.

## Papers and deviations

- Schied et al. "Spatiotemporal Variance-Guided Filtering." HPG 2017. §4.2 is
  the luminance weight. §3 temporal accumulation is not implemented (ADR 0012:
  no radiance history).
- Dammertz et al. "Edge-Avoiding À-Trous Wavelet Transform for fast Global
  Illumination Filtering." HPG 2010. The kernel is B3.
- Zhdan. ReBLUR / ReLAX. Half-resolution indirect and dropping iterations as
  history grows are not this filter. S79 rejected the short filter.
- Wright et al. "Lumen." SIGGRAPH 2022. The denoiser sees indirect light.
  Pixel direct is composed afterwards and stays sharp.

Deviations:

- Taps are screen pixels, not hash cells.
- The step is a projected 12.5 cm cell, clamped at 48 px, not a fixed pixel
  step and not an unclamped world step.
- 25 cm cells are outside the wavelet. They blend in only when certainty is
  below 1.
- Variance is this frame's 3×3 spatial moment, at one pixel and at one cell,
  floored at `(denoise_variance * mean)^2`. The cell-stride moment is what
  blurs a piecewise-constant texel grid. Without it phi is ~0 inside a cell
  and the grid never blurs.
- Every configured iteration runs.
- The whole picture is filtered every frame. Filtering only the shadow
  boundary would republish last frame. Layer 1 still decides which texels
  were retraced; the sun gain is applied before the wavelet.
- Irradiance-cache radius is not recomputed. A new texel with R == 0 already
  takes period 1.
- Two-sided keys use the raster geometric axis. Back faces use the opposite
  axis. The shading normal is not the key.
- Compose shows the per-pixel wavelet. The cell scatter is not the picture.
- S79 is not evidence against a screen filter. That trial kept the
  million-slot launch.

## Settings

`denoise_screen` is on for high and off for every other preset. Full effort
forces it off so the reference stays the dense hash wavelet. The key is
`denoise_screen`, the flag is `--denoise-screen`, the variable is
`GENOS_LIGHTING_DENOISE_SCREEN`, and the debug knob is `lighting_denoise_screen`.
Control bit 19. The pipeline is `screen-denoise`.

`GENOS_LIGHTING_DEBUG` writes `./genos-debug/denoise/latest.txt` and appends
`frames.log`: frame, pixels, coarse, changed, filtered, mode, ran. Changed is
measured (input versus the stored field, relative luminance above 0.02). The
launch stays the picture either way.

## Owner commands

The `atrous` column is the measurement. Do not read this as a lavapipe time.

```
cargo run --release -p genos-stress -- --size 1920x1080 --gpu-times 30 --preset high
cargo run --release -p genos-stress -- --size 1920x1080 --gpu-times 30 --preset high --denoise-screen off
cargo run --release -p genos-stress -- --size 1920x1080 --gpu-times 30 --preset high --still-boxes --freeze-sun --dynamic 0
GENOS_GPU_DISPATCH=1 GENOS_LIGHTING_DEBUG=1 cargo run --release -p genos-lighting -- --scene sponza --checkpoint atrium --size 1920x1080 --frames 180 --preset high --gpu-times 30
GENOS_GPU_DISPATCH=1 GENOS_LIGHTING_DEBUG=1 cargo run --release -p genos-lighting -- --scene sponza --checkpoint colonnade --size 1920x1080 --frames 180 --preset high --gpu-times 30
GENOS_GPU_DISPATCH=1 GENOS_LIGHTING_DEBUG=1 cargo run --release -p genos-lighting -- --scene sponza --checkpoint curtains --size 1920x1080 --frames 180 --preset high --gpu-times 30
GENOS_GPU_DISPATCH=1 GENOS_LIGHTING_DEBUG=1 cargo run --release -p genos-lighting -- --scene sponza --checkpoint dark-corner --size 1920x1080 --frames 180 --preset high --gpu-times 30
```

Potato on the atrium, and `--denoise-screen off` on the curtains checkpoint,
are the other two rows. The curtains checkpoint is the central banner.

## Expected gain

If the filter is a coherent image, stress à-trous 11–15 ms and Sponza 93–98 ms
both move toward about 1.4 ms. One hash find per visible pixel remains. On a
full table that find can dominate, and the gain is smaller than 1.4 ms, but
the 5×5 is no longer a walk of the hash. A large local change still filters
the picture, not a dilated dirty set. The picture bar against the hash wavelet
is mean 1.5 / p99 10 on 0..255, the run-to-run noise of high on the 4070.
