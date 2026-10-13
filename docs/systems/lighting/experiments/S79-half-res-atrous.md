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
- Schied, Kaplanyan, Wyman, Patney, Lefohn. "Spatiotemporal Variance-Guided Filtering: Real-Time Reconstruction for Path-Traced Global Illumination." HPG 2017. Edge-stopping weights on depth, normal, and luminance. Variance is section 4.2. The adaptive pass below uses that variance, and sample age, to drop iterations. SVGF itself still runs every iteration.
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

## Adaptive passes

`denoise_adaptive` and `denoise_variance` are the second commit. High leaves adaptive off and stores the threshold at 0.02 so the knob round-trips. Medium and low turn adaptive on.

Schied et al. 2017 section 4.2 drive the luminance weight from the variance. They still run every iteration. ReLAX (Zhdan, NVIDIA Real-Time Denoisers) runs fewer a-trous iterations as history length grows. A texel takes the smaller of the two limits:

- 16 surface-cache sets, or relative deviation `sqrt(variance) / mean` at or under the threshold: 1 iteration
- 8 sets, or deviation at or under four times the threshold: 3 iterations
- otherwise the configured count

Later iterations copy the last applied level forward. They do not run the 5×5. The push still reads buffer A.

Deviations: sample age is the surface-cache set count, not a screen history length. No frame is kept. SVGF itself does not drop iterations. A threshold of 0 ignores variance and uses age only.

Expected cost: on the moving sun the set count stays high (the history is not reset by a global light change) and the estimator variance stays small, so most fine texels should leave the 5×5 after one iteration. The dispatch still launches; the threads return after a copy. That is the opposite of the dirty-block skip, which saw every block change and did the full kernel. Where the history is young or the variance is high, the configured iteration count still runs.

```
GENOS_GPU_TIMES=30 GENOS_LIGHTING_PRESET=high GENOS_LIGHTING_DENOISE_ADAPTIVE=on GENOS_LIGHTING_DENOISE_VARIANCE=0.02 make stress
```

## Shared-memory tiles

`denoise_tiles` is the third commit. High leaves it off. Medium and low turn it on (low still does not run the filter). The key is `denoise_tiles`, the environment variable is `GENOS_LIGHTING_DENOISE_TILES`, and the debug menu knob is `lighting_denoise_tiles`.

Levels 1–5 bind `lighting_atrous_tile.comp`. Stages 0 and 6 stay on `lighting_atrous.comp`, so the scalar pipeline does not reserve the tile's shared memory. Each workgroup finds the 8×8 lattice origins it owns and loads that footprint once: slot, packed colour, variance, and mask. The thread for each center then runs the B3 kernel, including the 3×3 variance blur. A clean texel is not rewritten. The last iteration still copies into buffer A.

The footprint is `8 + 4 * step` texels on a side, with a halo of `2 * step`. At steps 1, 2, and 4 that is 12, 16, and 24, and 24×24 entries fit in the 16 KB shared minimum. Steps 8 and 16 run `lighting_svgf_iter` instead. Half-resolution fine texels use a lattice step of 2 cells; world-radiance-cache cells stay on a step of 1. Shared memory dies with the dispatch. No frame is stored.

Tiles replace the S78X work lists for levels 1–5. The moving scene is already the dense sweep, which is the path the tile accelerates. Lists stay when tiles are off. Dirty stamps are still read.

Deviations, besides the ones above:

- The tile is a surface-cache lattice, not a screen tile.
- Weights call `lighting_svgf_wzn` on the gathered slots. They are not an anchor-plane depth plus a quantized octahedral normal. `sigma_n` is 128, and that quantization would change the picture. The search this removes is the 5×5 hash (`LIGHTING_CACHE_SEARCH` probes per tap).
- A center whose object salt or texel size differs from the tile origin is not written by the tile. Its own thread runs the scalar kernel, so two workgroups do not write the same slot.

Expected cost: the hash probes inside the 5×5 become one cooperative load of the footprint, for the three finest iterations. The two coarsest stay scalar. On the moving scene the lists were already abandoned, so this does not depend on a dirty-block skip.

```
GENOS_GPU_TIMES=30 GENOS_LIGHTING_PRESET=high GENOS_LIGHTING_DENOISE_TILES=on make stress
GENOS_GPU_TIMES=30 GENOS_LIGHTING_PRESET=high GENOS_LIGHTING_DENOISE_RES=half GENOS_LIGHTING_DENOISE_ADAPTIVE=on GENOS_LIGHTING_DENOISE_TILES=on make stress
```

## Expected cost

The five wavelet dispatches dominate the 5.5 ms. They run on about a quarter of the fine texels, so those dispatches should fall to roughly 1.5–2 ms. The upsample is four taps inside the push, which already runs. This commit alone is not expected to hit 2 ms for the whole bucket; the variance pass still visits the even texels' neighbourhood, and the push still visits every fine texel.

Not timed here. The owner runs:

```
GENOS_GPU_TIMES=30 GENOS_LIGHTING_PRESET=high GENOS_LIGHTING_DENOISE_RES=full make stress
GENOS_GPU_TIMES=30 GENOS_LIGHTING_PRESET=high GENOS_LIGHTING_DENOISE_RES=half make stress
```

RTX 4070, 1920×1080, the moving stress scene. The `atrous` column is the one that should move. Lavapipe checks mean and p99 error against the full-resolution picture, error at depth discontinuities, and flicker variance on a still camera. Lavapipe does not time the GPU.

Half resolution, adaptive passes, and tiles together should bring the whole bucket near 2 ms. None of the three is timed here. High stays on the full-resolution scalar wavelet until the owner runs the A/B above.
