# S79: Half-resolution a-trous of bounce light

- **Date:** 2026-10-13
- **Result:** Tried and failed. Half-res and the shared-memory tile path are removed. Adaptive stays an off-by-default experimental knob. The plain shader split stays: high compiles without that branch.
- **Commit:** remove the half-resolution wavelet and the tiled kernel

## Idea

Run the a-trous wavelet on indirect light at half the surface-cache resolution, then upsample with depth and normal. Drop later iterations where the sample age or the variance says the texel has settled. Gather each 8×8 wavelet footprint once into shared memory. Do not skip work by dirty blocks: the stress scene's sun moves every frame and dirties every block.

## Motivation

On the owner's RTX 4070 at 1920×1080, in the moving stress scene, a-trous was 5.5 ms of an 18 ms lighting frame. The target was about 2 ms. PR #26 skipped unchanged blocks and lost the moving scene, because the day-clock sun changes every texel every frame.

## Papers

- Dammertz, Sewtz, Hanika, Lensch. "Edge-Avoiding À-Trous Wavelet Transform for Fast Global Illumination Filtering." HPG 2010. The B3 kernel `(1/16, 1/4, 3/8, 1/4, 1/16)` and the doubled step per iteration.
- Schied, Kaplanyan, Wyman, Patney, Lefohn. "Spatiotemporal Variance-Guided Filtering: Real-Time Reconstruction for Path-Traced Global Illumination." HPG 2017. Edge-stopping weights on depth, normal, and luminance. Variance is section 4.2. SVGF itself still runs every iteration.
- Zhdan, ReBLUR, in NVIDIA Real-Time Denoisers. The spatial filter of indirect light runs at half resolution. The upsample is a joint bilateral filter guided by depth and normal. ReLAX runs fewer a-trous iterations as history length grows.
- Wright et al. "Lumen: Real-Time Global Illumination in Unreal Engine 5." SIGGRAPH 2022. Direct lighting stays in the lighting pass. The denoiser sees indirect light.

## What was tried

The wavelet input was layer-1 bounce irradiance. Pixel direct and the texel direct cache were not read. Half mode replaced an even fine texel's level-0 colour with an edge-aware 2×2, ran the iterations on that even lattice with the cell step doubled, and upsampled odd texels in the push from the four even corners. No screen-sized image was allocated. Shared-memory tiles bound a second pipeline for levels 1–5 (`lighting_atrous_tile.comp`); stages 0 and 6 stayed scalar. Adaptive took the smaller of a sample-age limit and a relative-variance limit, and the last stage published the level that ran into buffer A.

Deviations, which are why the screen-space papers do not transfer:

- The grid is the surface-cache lattice, not a screen framebuffer. Odd and even slots share a warp, so skipping the odd 5×5 does not shrink the dispatch.
- No screen-sized image and no previous frame. The downsample lived in the even texel's level-0 slot. ReBLUR and Lumen dispatch a quarter as many pixels because they have that image.
- `w_z` is the plane offset this filter already uses (half a texel, S72V), not a screen depth gradient. Normals are cache normals.
- Sample age is the surface-cache set count, not a screen history length.
- The lavapipe-safe tile kernel cannot put `barrier()` in a loop or in an `if`, and shared `atomicAdd` faults, so it always ran eight straight-line barrier pairs.

## 4070, before the plain pipeline

Owner, RTX 4070, 1920×1080, stress scene. The `atrous` column is the sum of the seven a-trous dispatches. Moving is the day-clock sun. Still holds the camera; full still stays on the S78X work lists.

| run | moving ms | still ms |
|---|---:|---:|
| gi-v2 / P29 default | 7.5 | |
| `--preset high` full | 11.7 | 3.0 |
| `--denoise-tiles off` | 11.6 | |
| `--denoise-adaptive on` | 5.1 | |
| `--denoise-res half` | 17.8 | 9.5 |
| `--denoise-tiles on` | 18.3 | 8.9 |
| half + adaptive | 12.0 | |
| half + adaptive + tiles | 15.2 | 10.8 |

High full at 11.7 ms was the same seven dispatches as gi-v2's 7.5 ms. Half, adaptive, and the tile-origin recording were compiled into the hot functions, and the live registers were the max of every branch. That is an occupancy drop. High now binds `atrous-plain`, compiled with `LIGHTING_DENOISE_PLAIN`, so those branches are not in that SPIR-V.

Half, moving 17.8 against full 11.7, still 9.5 against 3.0. The dispatch count does not change (7 × 1,048,576/64 workgroups). `lighting_svgf_half_skip` called `lighting_sc_true_cell` (up to nine key probes) on every slot of every stage, because `floor(position / size)` is a cell off (S72V). The push of an odd texel skipped the S78B idle and did four lookups. An even-only list is past the 65,536 cap on a moving sun.

Tiles, moving 18.3 against tiles-off 11.6, still 8.9 against full 3.0. Tiles do not append S78X work lists, so levels 1–5 are always the dense sweep. The kernel always runs eight barrier pairs and about 9 KB of shared memory. Steps 8 and 16, and centers the tile does not own, also run the scalar 5×5.

Adaptive at 5.1 ms still launched all seven dispatches. A clean skip used to return before copying into buffer A, so the push blended toward the 25 cm world-radiance cache. Image diff against full, 0–255: still mean 15.9 / p99 41, moving 21.9 / 51. The publish fix is what the retest below measures.

## Retest at 476ec13

Owner, RTX 4070, 1920×1080, after the plain pipeline and the buffer-A publish.

| run | result |
|---|---|
| high full, moving | atrous ~8.0 ms, against gi-v2 7.8 ms |
| medium (then adaptive on), moving | 22 ms / 43 fps, against 25.2 ms / 38 fps |
| adaptive, still atrous | 3.13 ms against full 2.7 ms. No total-time gain |
| adaptive vs full | mean 2.8, p99 11–13. Bar is 1.5 / 10 |
| full, run to run | mean 1.1–1.4, p99 9.3 |
| adaptive ×8 diff | broad blotches |
| half-res and tiles | slower than the scalar wavelet. The first-pass milliseconds above still stand |

## Postmortem

Half-res does not shrink the dispatch on a hashed surface-cache lattice: odd and even slots share a warp, and the parity test pays up to nine cell probes on every slot, so the 4070 measured 17.8 ms moving and 9.5 ms still against full at 11.7 and 3.0.
Tiles turn the work lists off and the lavapipe-safe kernel always runs eight barrier pairs plus shared memory, so levels 1–5 are a dense sweep: 18.3 ms moving and 8.9 ms still against tiles-off 11.6 and full still 3.0.
Adaptive's empty buffer A was fixed, but the settled one-iteration picture still misses the bar (mean 2.8 / p99 11–13 against 1.5 / 10; full's own run-to-run noise is 1.1–1.4 / 9.3) with broad blotches in the ×8 diff, and still-atrous got slower (3.13 vs 2.7) with no total-time gain, so it stays an off-by-default experimental knob.
The plain pipeline, compiled without those branches, is what put high full back to about 8.0 ms against gi-v2's 7.8 ms moving.
Medium is the full-resolution scalar wavelet at 3 iterations with adaptive off; the 22 ms / 43 fps gain was the adaptive-on medium, and that picture does not pass the bar.

## What remains

The scalar full-resolution wavelet. High binds `atrous-plain` (adaptive compiled out). The feature pipeline still has the adaptive branch, selected only when `denoise_adaptive` is on. The key is `denoise_adaptive`, the flag is `--denoise-adaptive`, the environment variable is `GENOS_LIGHTING_DENOISE_ADAPTIVE`, and the debug knob is `lighting_denoise_adaptive`. `denoise_variance` is read only then. A threshold of 0.1 forces one iteration so a short run can enter the skip. No preset should turn adaptive on: the retest failed the picture bar.
