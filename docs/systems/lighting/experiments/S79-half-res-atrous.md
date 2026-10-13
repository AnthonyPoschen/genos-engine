# S79: Half-resolution a-trous of bounce light

- **Date:** 2026-10-13
- **Result:** in progress (owner times the RTX 4070; lavapipe checks the picture)
- **Commit:** half-resolution wavelet and joint-bilateral upsample

## Idea

Run the a-trous wavelet on indirect light at half the surface-cache resolution, then upsample with depth and normal. Direct light stays sharp. Do not skip work by dirty blocks: the stress scene's sun moves every frame and dirties every block.

## Motivation

On the owner's RTX 4070 at 1920×1080, in the moving stress scene, a-trous is 5.5 ms of an 18 ms lighting frame. The target is about 2 ms. PR #26 skipped unchanged blocks and lost the moving scene, because the day-clock sun changes every texel every frame.

## Papers

- Dammertz, Sewtz, Hanika, Lensch. "Edge-Avoiding À-Trous Wavelet Transform for Fast Global Illumination Filtering." HPG 2010. The B3 kernel `(1/16, 1/4, 3/8, 1/4, 1/16)` and the doubled step per iteration.
- Schied, Kaplanyan, Wyman, Patney, Lefohn. "Spatiotemporal Variance-Guided Filtering: Real-Time Reconstruction for Path-Traced Global Illumination." HPG 2017. Edge-stopping weights on depth, normal, and luminance. Variance is section 4.2. This pass still uses those weights. It does not yet drop iterations from the variance; that is the next commit.
- Zhdan, ReBLUR, in NVIDIA Real-Time Denoisers. The spatial filter of indirect light runs at half resolution. The upsample is a joint bilateral filter guided by depth and normal.
- Wright et al. "Lumen: Real-Time Global Illumination in Unreal Engine 5." SIGGRAPH 2022. Direct lighting stays in the lighting pass. The denoiser sees indirect light.

## What was implemented

`denoise_res` is `full` or `half` on `DenoiserSettings`. High is `full`. Medium and low record `half` (low does not run the filter, because it has no surface cache). The key is `denoise_res`, the environment variable is `GENOS_LIGHTING_DENOISE_RES`, and the debug menu knob is `lighting_denoise_res`. MCP `get_lighting_settings` / `set_lighting_settings` read and write it with the rest of the text form.

Half mode, in `lighting_cache.glsl`:

- The wavelet input is layer-1 bounce irradiance (`lighting_svgf_in`). Pixel direct (the G-buffer term in `lighting_compose.comp`) and the texel direct cache are not read.
- An even fine texel replaces its level-0 colour with the edge-aware 2×2 of layer-1 bounce (`w_z * w_n`).
- Iterations run only on that even lattice, with the cell step doubled.
- An odd fine texel is not a wavelet output. The push upsamples it from the four even corners: bilinear weight times `w_z * w_n`, no luminance term. The display field is written in that same dispatch.
- World-radiance-cache cells stay on the full filter.

## Deviations

- The grid is the surface-cache lattice, not a screen framebuffer. SVGF, ReBLUR, and Lumen denoise screen pixels.
- No screen-sized image is allocated. The half-resolution colour lives in the even texel's existing level-0 slot. The upsample does not sample a previous frame.
- `w_z` is the plane offset this filter already uses (half a texel, S72V), not ReBLUR's screen-space depth gradient.
- Normals are the cache normals, not a screen G-buffer sample.
- The finest full-resolution wavelet (step 1 cell) does not run. The upsample restores edges. It does not replace that high-frequency blur.
- The downsample's variance is the even texel's own variance, not a variance of the 2×2.

## Expected cost

The five wavelet dispatches dominate the 5.5 ms. They run on about a quarter of the fine texels, so those dispatches should fall to roughly 1.5–2 ms. The upsample is four taps inside the push, which already runs. This commit alone is not expected to hit 2 ms for the whole bucket; the variance pass still visits the even texels' neighbourhood, and the push still visits every fine texel.

Not timed here. The owner runs:

```
GENOS_GPU_TIMES=30 GENOS_LIGHTING_PRESET=high GENOS_LIGHTING_DENOISE_RES=full make stress
GENOS_GPU_TIMES=30 GENOS_LIGHTING_PRESET=high GENOS_LIGHTING_DENOISE_RES=half make stress
```

RTX 4070, 1920×1080, the moving stress scene. The `atrous` column is the one that should move. Lavapipe checks mean and p99 error against the full-resolution picture, error at depth discontinuities, and flicker variance on a still camera.
