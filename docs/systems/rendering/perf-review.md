# Renderer performance review

Written 2026-10-13 against the former `gi-v2` tip at `9e08d7d` (now on `master`). Terms follow `docs/lighting/GLOSSARY.md` (plain **GI**). This is a review. The timer split in this change makes `GPU_MS` report lighting apart from the raster. It does not change a pass. Every later cut still draws the whole frame.

**Goal.** A moving stress frame at 1920×1080 in about 5 ms (200 fps). The measured frame is 13.4–16.7 ms (62–77 fps).

**Binding.** [Lighting](../lighting/README.md) is the pass list and the papers. [ADR 0007](../../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps a vendor upscaler out. A cut that skips a pixel, a probe, or a texel for a frame is out. S77B (per-pixel shadow reuse) tore edges and stays reverted.

## Measured frame

Owner, zanven-pc, RTX 4070, windowed moving stress, `gi-v2` `9e08d7d`, `GENOS_GPU_TIMES=30`.

| | ms | share |
|---|---:|---:|
| Frame | 13.4–16.7 | 62–77 fps |
| Lighting (`LIGHTING_MS` total) | 12.6–16.0 | about 93 % |
| Everything else | about 0.8 | |

The stress scene has few triangles. The 0.8 ms is the depth prepass, the shaded raster, the overlay, the swapchain copy, and the picture copies around the lighting passes. Raster is already close to free. The 5 ms budget is a lighting budget.

`GPU_MS` used to print one `raster` number whose stamp sat after `lighting_passes`, so that number contained the lighting span. The line now prints `gbuffer`, `lighting`, and `nonlight` apart. `nonlight` is `setup + gbuffer + aa + overlay + copy`. `lighting` is the `lighting_passes` span (the dispatches, plus the picture copy into the output buffer and the copy back onto the colour image). `LIGHTING_MS` is the dispatch sum inside that span, which is the table below. The two picture copies are the difference between `lighting` and the `LIGHTING_MS` total, and they are part of the 0.8 ms outside the dispatch sum.

Per-pass averages from the same run (`place` was not called out; `LIGHTING_MS` still prints it):

| Pass | Measured ms | What it is |
|---|---:|---|
| `light` | 4.7 | Lamps and shadow rays at every pixel and at the kept hits that are due this frame (`lighting_direct.comp`) |
| `atrous` | 1.7–4.5 | Seven slot dispatches: variance, five wavelet levels, push. The wide swing is levels 1–5 |
| `compose` | 2.1 | Full 1920×1080 composite. The 8×8 dirty-block list does not run while the scene hash changes |
| `trace` | 1.9 | This frame's probe rays, re-traces, and fills |
| `gather` | 0.95 | Probe bins |
| `filter` | 0.87 | Three probe-space rounds |
| `cache` | 0.35 | Surface-cache batch |
| `compact` | 0.18 | Slot sweep that clears the a-trous lists |

The passes in that table, apart from `atrous`, sum to 11.05 ms. Adding the a-trous range gives 12.75–15.55 ms of those dispatches. That sits against the 12.6–16.0 ms lighting total. `place` was not called out, and the ends of the two ranges are not the same frames, so the remainder is `place` plus the gaps between stamps.

## Budget

Targets sum to 5.00 ms. The cut is measured minus target. `atrous` has two cuts because the pass swings.

| Pass | Measured ms | Target ms | Closes |
|---|---:|---:|---:|
| `light` | 4.70 | 1.40 | 3.30 |
| `atrous` | 1.70–4.50 | 0.70 | 1.00–3.80 |
| `compose` | 2.10 | 0.50 | 1.60 |
| `trace` | 1.90 | 0.50 | 1.40 |
| `gather` | 0.95 | 0.35 | 0.60 |
| `filter` | 0.87 | 0.30 | 0.57 |
| `cache` | 0.35 | 0.20 | 0.15 |
| `compact` | 0.18 | 0.15 | 0.03 |
| `place` | not called out | 0.10 | |
| non-lighting | 0.80 | 0.80 | 0 |
| **frame** | **13.4–16.7** | **5.00** | **8.4–11.7** |

`light`, `atrous`, and `compose` are the gap. Together they close 5.9 ms on a quiet a-trous frame and 8.7 ms when a-trous is at 4.5 ms. The other lighting passes close about 2.8 ms if they all hit their targets. Non-lighting is already inside the budget. A smaller colour format, a removed fullscreen copy, or a present-mode change can move the 0.8 ms. It cannot move a 14 ms lighting span to 4.2 ms.

## Ranked cuts

Each cut keeps a shaded pixel in every frame. Still and moving, pop, brightness, and a moving-shadow picture stay the gate from the lighting notes. A cut that does not move `frame` on the 4070 is reverted.

### 1. Light pass, 3.30 ms

`lighting_direct.comp` runs one lamp loop and one shadow ray per lamp at every pixel and at every kept hit that is due. On an older moving frame whose light pass was 6.60 ms, switching off kept-hit relight or the per-pixel loop each removed several milliseconds, and the moving lamps made that work real ([S78K](../lighting/experiments/S78K-held-probe-strata.md)). The pass is down to 4.7 ms and is still the largest steady cost.

The target is 1.40 ms with every pixel and every due hit still shaded. The candidate is one shade of a lamp reused by the hits that see the same visibility, instead of a second full loop. Lumen shades the surface cache and the screen from that cache ([W22](../lighting/README.md)). Many-light importance sampling is [LT](../lighting/README.md) (Conty Estevez and Kulla, HPG 2018). S77B's per-pixel shadow reuse is not the candidate: it tore shadow edges and was reverted.

### 2. A-trous, 1.00–3.80 ms

Seven dispatches each launch 1,048,576 slots (`LIGHTING_CACHE_SLOTS`). Stage 0 is the variance estimate (Schied et al., "Spatiotemporal Variance-Guided Filtering", HPG 2017, sec. 4.2, [SVGF](../lighting/README.md)): the texel's moment variance, or a 7×7 spatial estimate when the texel is young. Stages 1–5 are the edge-avoiding à-trous levels (Dammertz et al., HPG 2010) with the B3 kernel. Stage 6 is the pull-push (Gortler et al. 1996). Layer 2 never writes layer 1.

Levels 1–5 use the work lists while last frame's dirty fraction stays at or under 0.25, and the dense slot sweep above that ([S78X2](../lighting/experiments/S78X2-atrous-dense-fallback.md)). The lists are the 1.7 ms end. The dense sweep is the 4.5 ms end. Variance and the push run the full slot grid on both ends, so the swing is the five wavelet levels. A moving scene crosses 0.25 often, because most filtered texels are dirty.

The 0.70 ms target has to hold on the dense end. The lists already did the quiet end, and they got slower than the dense sweep when the append was an atomic and the taps lost coalescing (owner, 6.6 → 9.4 ms, before this fallback). The next try is a dense prefix of the filtered texels (about 108k in a still scene, more while moving), built in the compact pass, so the levels read neighbours that sit together. Page skips were already measured and did not pay: hashed texels touch almost every page ([S78X](../lighting/experiments/S78X-atrous-work-lists.md)).

`GENOS_GPU_DISPATCH=1` prints the seven `8:` lines inside `atrous`. The next moving run should say which of variance, the five levels, and the push is the 1.7 and which is the 4.5.

### 3. Compose, 1.60 ms

`lighting_compose.comp` is an 8×8 group per block. A full frame covers 1920×1080. A held frame dispatches only the blocks whose direct light, probe SH, or filtered texel changed. The moving stress scene changes the scene hash every frame (boxes, sun), `still` stays 0, and the pass takes the full dispatch. That is why the dirty-block skip does not move this number. The shader still returns on a pixel with no surface, after the group has launched.

The 0.50 ms target is a cheaper full composite, not a skip. Four probe irradiance reads plus a cache fallback per pixel is the body of the 2.1 ms ([W22](../lighting/README.md) screen probes; the probe weight in the shader is the plane distance and the normal). A moving frame still writes every pixel that has a surface.

### 4. Trace, 1.40 ms

1.9 ms is this frame's stratum, the re-traces of kept rays that cross a change, and the fills. Held strata already cut the still trace ([S78K](../lighting/experiments/S78K-held-probe-strata.md)); the moving lamps keep the re-traces. The target is 0.50 ms with the same check ray per held probe. [ReSTIR GI](../lighting/README.md) (Ouyang et al., HPG 2021) is the resampling reference. A smaller ray count that leaves the moving picture noisier is not the cut: [S72A](../lighting/experiments/S72A-sparse-fill.md) dropped rays and the frame rate did not move, because the light pass is per pixel.

### 5. Gather and filter, 0.60 ms and 0.57 ms

Gather is 0.95 ms after the shared-memory bin ([S78P](../lighting/experiments/S78P-gather-shared-memory.md)). Filter is 0.87 ms for three rounds. The quiet filter ([S78Y](../lighting/experiments/S78Y-quiet-filter.md)) drops rounds whose 3×3 neighbourhood did not change; a moving probe grid keeps more of them. Targets 0.35 ms and 0.30 ms. The probe filter's published shape is the same three-round filter the lighting notes already cite against Lumen.

### 6. Cache and compact, 0.15 ms and 0.03 ms

Cache is 0.35 ms and compact is 0.18 ms. Both sit on their targets. [S78V](../lighting/experiments/S78V-se-stop-rule.md) already found that stopping more settled texels does not move the moving frame: the rays are not the cost. Leave them until the three passes above have moved.

### 7. Non-lighting, 0 ms of the gap

0.8 ms for setup, the G-buffer, FXAA when it is on, the overlay, and the copies. The depth prepass already shades each opaque pixel once. The swapchain is mailbox when the surface lists it, then immediate, then FIFO. None of that is the 8–12 ms gap. The timer split is the non-lighting work this change does, so the next `GPU_MS` line can be compared with the table.

## Command

Release, on zanven-pc, from a clean tree. The window is the normal stress window (not `--proof`, not `--bench`). `--frames` steps the sun and the boxes at 1/60 s and then exits. `GENOS_GPU_TIMES=30` matches the run above.

```sh
GENOS_GPU_TIMES=30 cargo run --release -p genos-stress -- --size 1920x1080 --frames 180
```

Read the last `GPU_MS` and the `LIGHTING_MS` beside it. The two picture copies now sit in `lighting`, so that number runs a little above the `LIGHTING_MS` total and `nonlight` a little under the old 0.8 ms remainder. `fps` is wall clock; `frame` is the timestamp sum. A one-shot dispatch split of the a-trous swing:

```sh
GENOS_GPU_TIMES=30 GENOS_GPU_DISPATCH=1 cargo run --release -p genos-stress -- --size 1920x1080 --frames 90
```

The entries whose pass id is 8 (`N:8=`) are variance, the five levels, and the push, in that order.
