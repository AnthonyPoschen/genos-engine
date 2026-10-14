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

The kernel is the hash wavelet: `lighting_svgf_variance`, `lighting_svgf_iter`,
and `lighting_svgf_push`. Variance, the luminance weight, the dirty hold, and
the Gortler pull are those functions. A screen-sized B3 of a gathered bilinear
is not that kernel. The luminance weight is nonlinear, so it does not commute
with the bilinear, and a screen filter has no hold (ADR 0012). The hash blurs
a texel while the spatial estimate is still wide and then holds that blur.
Rebuilding from the settled mean (variance near zero) leaves the 12.5 cm
grid. Widening phi to hide the grid spreads energy the hold never spread.
That was the ceiling, wall, and floor blotch, including the 1920×1080 x8
diffs.

The launch is the picture. Stage 0 stores the fine slot under each pixel, one
hash find, keyed by the geometric axis in the G-buffer. Stage 1 picks one
pixel per visible cell, the visible hit closest to the cell centre, and runs
variance there. The pixel the centre projects to is the wrong owner once the
surface is edge-on: that ray hits a nearer cell, the texel has no thread, and
its raw mean shows through as grain. The next stages are the five hash
iterations and the push, on that same pixel. Compose bilerps the pushed field
(`lighting_sc_shown`), which is the hash picture. Slice 1 stays empty so the
per-pixel buffer does not replace that bilinear. 25 cm cells are not given a
screen thread. The push reads whatever the hash kernel last wrote for them.

A two-sided back face (`gl_FrontFacing` false) stores the opposite axis, so
the front and the back do not share a cell. `gl_FrontFacing` does not follow
the normal map, so a small camera move on one side of a cloth keeps the key.
The curtains shot is this: lamps on the far side of the central banner, and
bright squares on the side the camera sees. Those squares are the far side's
12.5 cm cells when the two sides share a key. The test fills the lamp side of
a two-sided quad, then cuts to the other side and moves the camera. That side
stays dark: its p95 stays under half the lamp side, and it has no cell-sized
peaks. One shared axis makes the far side as bright as the lamp side.

The same cells show on a sun-shadowed floor, where the direct sun is gone
and only bounce is left. A wall blocks the sun. A small emitter lights one
cell. The hash wavelet keeps that cell brighter than the four around it.
The screen wavelet keeps the same peak: erasing it was the pixel-step blur.
The world-locked flicker of those floor points stays with the hash path.
The stress hall, still and moving, is the full-picture gate at 960×540, where
a foreground cell is wider than a pixel kernel. Eight warmup frames, then the
scored pair. Mean and p99 against the hash wavelet stay inside the picture
bar (1.5 / 10), and the screen path's own frame-to-frame change stays with
the hash path. The synthetic floor did not see the 1920×1080 miss. A longer
lavapipe settle (16 warmup frames) leaves a far-wall tail around p99 11:
an input step after the variance has collapsed rewrites the held blur. The
hash launch's filtered level stays inside 1% across that step.

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

- The dispatch visits cells a pixel of this frame can see. The hash pipeline
  also filters a fine texel for 30 frames after it leaves the picture, and
  every 25 cm cell. This launch does not.
- The owner is the visible hit closest to the cell centre, not a projected
  centre whose ray hits a nearer cell. A cell split by a nearer surface has
  two such hits. One exchange on a word past the sun block
  (`LIGHTING_SCREEN_OWNER_AT`, one per slot) keeps a single kernel thread.
  The kernel reads the slot, so which hit wins does not change the colour.
- The spatial stamp (a neighbour's 7×7 inputs moved) is cleared before
  variance. On the full hash launch that redo stays inside the held blur.
  Neighbours this frame cannot see have an empty level, so the same redo
  rebuilds from the settled mean and the 12.5 cm grid returns. A texel whose
  own input moved still recomputes.
- Phi, the luminance weight, the depth and normal weights, and the pull are
  the hash kernel's. `denoise_variance` does not floor them, and there is no
  screen-space moment standing in for the spatial estimate.
- Every configured iteration runs. The dirty hold is the hash kernel's hold
  of a texel whose input moved less than 1%. It is not a held picture
  (ADR 0012).
- Two-sided keys use the raster geometric axis. Back faces use the opposite
  axis. The shading normal is not the key.
- Compose shows the bilinear of the pushed cells. It does not show a
  per-pixel filter.
- S79 is not evidence against a screen launch. That trial kept the
  million-slot dispatch.

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

The 4070 measurement of the pixel-step wavelet (the launch that filtered in
screen space) was Sponza high 9.6 to 19.8 fps, denoise 63.6 to 5.6 ms, and
stress corner 174 to 269 fps. Medium did not move: the screen wavelet is off
there. Those pictures were outside the bar (stress still mean 15.5 / p99 32,
moving 9.5 / 27.7, Sponza atrium p99 41 against 2.7). Lavapipe does not time
the GPU, and it does not stand in for a retest of this hash-kernel launch.
