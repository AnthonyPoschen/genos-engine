# Path to 200 fps on the moving stress scene

Research note for `gi-v2` at `5b38ca20` (2026-10-13). No lighting code changes. The target is a moving stress frame at 1920×1080 in 5 ms or less on an RTX 4070, which is 200 fps. The owner measurement this note starts from is about 30 ms, about 34 fps, with the card shared: other GPU work was using about 35% of it.

That 30 ms is the number to beat. It is not an idle-card time. If the other work is concurrent and this frame is bandwidth-bound, an idle card is faster, and the gap is not something this note divides out. The still-scene wavelet times already on file (about 1.8–3 ms) sit in the same direction as a contended 11–15 ms moving wavelet. An exclusive run is a profiling experiment below, not a corrected baseline.

Five columns the owner named already account for almost the whole frame. A middle of the reported bands sums to 30.0 ms:

| Bucket on the `LIGHTING_MS` line | Owner | Used below |
|---|---:|---:|
| `atrous` | 11–15 ms | 13.0 |
| `light` (pixel direct and the hit-light dispatch together) | 11.5 ms | 11.5 |
| `compose` | 1.8–3 ms | 2.2 |
| `trace` (probe rays and cache-ray traces; one pipeline) | 1.5 ms | 1.5 |
| `gather` | 0.9 ms | 0.9 |
| raster, AA, overlay, copy, compact, place, filter, cache | not split | 0.9 |
| frame | ~30 ms | 30.0 |

`shadows` on that line is the same `light` time added again while shadows are on. It is not a second pass.

Work in flight elsewhere, and not counted twice in the budget: a change-driven denoiser, weighted lamp picking ([PR #30](https://github.com/AnthonyPoschen/genos-engine/pull/30), plan only), probe ring wakes ([PR #31](https://github.com/AnthonyPoschen/genos-engine/pull/31), unmerged; the owner measures direct at about 6 ms on that branch), and screen-space-weighted verification.

## Constraints the 5 ms frame still meets

- No baked lighting. Import-time mesh SDFs and cards stay. A lightmap, a precomputed transport, or a trained field that stands in for the live answer does not. [ADR 0012](../../../adr/0012-gi-v2.md) already rejects baked lightmaps, and it rejects a radiance history.
- A moving frame does not reuse or hold a whole frame. Visibility (kept hits) may stay. The light on those hits is re-read. A check still runs. The settled idle (pixel direct and compose, bounce field left as it was) is a still-scene path. It is not the moving-scene budget. Temporal upscalers that republish last frame's picture are the same hold.
- Pre-fill stays. World-radiance-cache scout rays run on a moving camera (256 batch entries, 16 rays each, inside the cache-ray dispatch). Off-screen and around-corner texels keep a share of the ray budget. Deleting that scout is not one of the cuts below.

The default stress scene these numbers describe is one 25 m section, 5 lamps, half of them moving, a 120 s day, and the hall view (`examples/stress`). `make stress` opens at 1280×720. Every launch size below is 1920×1080, which is what the owner timed.

## What one moving frame launches

Preset `high`, cache round, picture 1920×1080. The group list is `light_groups` in `crates/render/src/lighting_dispatch.rs`.

Screen probes: tile 16 px (`lighting_tile` clamps `height/45` to 16). Grid `ceil(1920/16) × ceil(1080/16) = 120 × 68 = 8160` probe slots. An earlier 1080p count on the ring-wake branch placed about 8093 of them. Rays per probe 64, one stratum a frame, 8 strata kept. Allocated kept hits: `8160 × 64 × 8 = 4,177,920`.

| Dispatch | Workgroups | Threads launched | What those threads do |
|---|---:|---:|---|
| compact | 16,384 | 1,048,576 | one thread per cache slot, priority buckets |
| place | 128 | 8,160 live | one probe on the G-buffer |
| gather, picks | 128 | 8,160 | which neighbour owns each stratum |
| compact args | 1 | 64 | indirect sizes |
| trace | 138,720 | 8,878,080 | `8160×64` stratum rays plus `2 × kept` (crossings and fills) |
| cache rays | indirect | up to 1,048,576 | ceiling `LIGHTING_SC_BASE_BATCH` (65,536) × 16; the budget is a ceiling |
| direct | 162,960 | 10,429,440 | `2,073,600` pixels plus `2 × kept`, mode 1 (no cache rays in this launch) |
| hit light | indirect | live entries × 16 | cache-ray hits, same shader |
| gather, bin | 8,160 | 522,240 | one workgroup per probe, 64 rays, each thread walks 8 strata in shared memory |
| filter ×3 | 128 × 3 | 8,160 × 3 | direction-space à-trous, then SH |
| cache | indirect | live entries | MSME update |
| à-trous ×7 | 16,384 × 7 | 7,340,032 | stages 0..6: variance, five iterations, push |
| compose | 240 × 135 | 2,073,600 | 8×8 thread groups over the picture |

The trace pipeline's timestamp is the probe launch and the cache-ray launch added together (`LIGHTING_MS` indexes by pipeline). The light pipeline's timestamp is direct and hit-light added together. `GENOS_GPU_DISPATCH=1` prints each launch; the owner split does not.

### Memory the passes sit on

| Buffer | Size at 1080p | Why it is that big |
|---|---:|---|
| Surface cache | 1,048,576 × 368 B = 368 MB | hash table, 12.5 cm texels and 25 cm hit cells in one table |
| Cache keys | about 52 MB | scheduler, five à-trous marks of a million slots, compose rects |
| Work | on the order of 150 MB | every kept hit of all 8 strata (4.18 M records) plus pixels' direct-reuse words |
| G-buffer | about 33 MB | 16 B × 2.07 M pixels |
| Output | about 25 MB | picture, glow, bounce slice |

The RTX 4070's peak DRAM is about 504 GB/s (192-bit GDDR6X at 21 GT/s). Its L2 is 36 MB. The 368 MB cache does not fit. A 1080p color image does.

ADR 0012's lighting budget is about 200 MB, and Ultra may go over it when that buys speed. This frame already does. The cuts below rearrange that traffic. They do not add another cache of the same size.

### Why the wavelet is 11–15 ms

`lighting_atrous.comp` runs seven dispatches over all 1,048,576 slots. A slot whose key is 0 returns after a few loads. Empty slots are about 7 × 1 M × (a 4 B key and often one 16 B record) ≈ 0.1 GB. At any plausible achieved bandwidth that is under a millisecond. The 11–15 ms is the live set.

A live texel in iterations 1–5 (`lighting_svgf_iter`):

- a 3×3 of the input variance, 9 neighbour finds
- a 5×5 à-trous (B3 weights, step 1, 2, 4, 8, 16), 25 neighbour finds
- each find is `lighting_sc_cell_slot`: rebuild the cell, then `lighting_cache_find_sized`, up to `LIGHTING_CACHE_SEARCH = 16` dependent key loads, then a 16 B packed color and, for the weight, the neighbour's position and normal (32 B more)

Stage 0, when the variance is spatial, walks a 7×7. Stage 6 pushes against four 25 cm cells and writes the field compose reads.

On a still scene the work lists cover about 5–18 k dirty texels out of about 108 k filtered, and à-trous is about 1.8–3 ms (S78X2, S79). On the moving scene the day-clock sun and the moving lamps push the dirty fraction over the list threshold (0.25, cap 65,536). Levels 1–5 then take the dense sweep. S78X measured that regression directly: moving à-trous 6.6 → 9.4 ms when the lists stayed on past the point they help. S79's plain pipeline, full resolution, moving, was about 8 ms before this contended 11–15 ms band.

Model, checked against that still-scene ratio. Take the still filtered set, about 108 k texels, now dense because the sun moved:

- 34 finds × about 64 B of uncoalesced key and record traffic × 5 iterations × 108 k ≈ 1.2 GB
- the 7×7 variance pass on the same set ≈ 0.3 GB
- push and stamp traffic is smaller

About 1.5 GB of dependent gathers. Random 16 B loads on this card land nearer 100–150 GB/s of useful bandwidth than the 504 GB/s copy peak. 1.5 GB / 120 GB/s ≈ 13 ms. The still-scene ratio lands in the same place: 18 k / 108 k × 13 ms ≈ 2.2 ms, next to the measured still wavelet. So the moving wavelet is the still wavelet with the dirty set opened up to the whole filtered set, plus the dense launch.

Two details make the sun expensive:

- `lighting_svgf_active` returns true for every slot whose texel size is above 0.2 m. Hit cells and world-radiance-cache cells are 0.25 m, so they take the filter even when no pixel is showing them.
- `LIGHTING_DIRTY_NOISE` is off. A sun step that moves a stored mean by more than 1% (`LIGHTING_DIRTY_EPS`) dirties the 5×5 around it. A slow sun still crosses that line across the lit set, which is why "skip clean blocks" lost the moving scene (S79, PR #26).

S79 also tried half-resolution and shared-memory tiles on this hash. The dispatch stayed 7 × 1,048,576/64. Odd and even slots share a warp, and `lighting_sc_true_cell` can hash nine candidates per tap. Half-resolution got slower (17.8 ms moving against 11.7). The screen-space papers never had that launch.

### Why direct light is 11.5 ms

`lighting_direct.comp` is one shader for pixels, kept probe hits, fills, and cache-ray hits. The picture launch is `(pixels + 2 × kept) / 64 = 162,960` workgroups, 10.43 M threads. Cache-ray hits are the second, indirect launch inside the same `light` timestamp.

What survives the early-outs on a moving frame:

- All 2.07 M pixels. `LIGHTING_PROBE_PICKS` is false, and the pixel path does not use the two-lamp pick. Each pixel builds `lamps_at` and traces a shadow per lamp in range. S78Q recorded about five lamps per pixel on this moving scene. That is on the order of 10 M software shadow rays (`trace_occluded` → `scene_ray` over the analytic shapes, plus a mesh-SDF step).
- Kept hits on this stratum and the halfway stratum, and any hit near a mover or a lamp that changed since it was traced. A sun or lamp move past 1% fails the S78D skip, so those hits walk every lamp too. Probe picks are off.
- Texel-direct-light reuse (S75) can zero the lamp count when the signature matches. A moving lamp changes the signature, so the reuse misses on the surfaces that lamp reaches.

The ring-wake branch (PR #31, unmerged) is the owner's measurement that a large piece of this is the kept-hit tail: direct falls to about 6 ms when a kept hit is not shaded only because a lamp moved, and the launch is the live list rather than `pixels + 2 × kept`. The PR's earlier A/B at `e8fe3a39` did not move the moving total (28.1 vs 28.2 ms) because that tail was still launched and returned inside the shader. The 11.5 → 6 ms figure is the later launch shrink, on the shared card, and it has not been re-measured as a full `LIGHTING_MS` line in the PR text.

The 6 ms that remains is the pixel shadow work, plus probes the rings actually woke, plus the hit-light dispatch. S78Q's pixel reservoirs, on an older tree, moved the light pass 6.59 → 6.38 ms on this same five-lamp scene, and 47.9 → 35.2 ms in a 100-lamp room, because the hits still walked every lamp. Lamp picking pays the hell scene first. It pays this scene only after the launch is the pixels.

### Why the small passes cost what they cost

**Trace, 1.5 ms.** The launch is 8.88 M threads. The rays that hit geometry are one stratum (about 522 k) plus fills, crossings, and whatever cache-ray batch the ceiling allowed this frame. Scout pre-fill is 256 × 16 = 4,096 rays inside that batch on a moving frame. A software ray here is cheaper than a shadow ray in the light pass only because most of the 8.88 M threads return on the crossing test. The timestamp does not say how the 1.5 ms splits. The dispatch dump does.

**Gather, 0.9 ms.** 8,160 workgroups, shared-memory binning (S78P). Each thread reads 8 strata. That is 4.18 M kept-hit loads. Gather hold (S78R3) dropped a still gather from 0.85 ms to 0.09 ms. On the moving scene the lamps change kept light, the hold misses, and every probe bins again.

**Compose, 1.8–3 ms.** 240 × 135 groups. Each pixel reads four probes, 9 × 3 SH coefficients (`lighting_probe_irradiance`), and falls through to a hash find of the surface cache when the probe weight is under 0.05. The SUN note split an older compose into about 2.4 ms of probe walk and about 0.4 ms of full-picture write. A moving sun marks most 8×8 blocks, so the quiet list is nearly the full grid. The write is the small part.

**The 0.9 ms residual.** Compact touches a million keys and is the scheduler. Place, the three filter rounds, and the cache update are thousands of threads. The G-buffer raster of one section, the AA resolve, and the copy sit in the same residual. None of them is the 5 ms problem. The profiling section checks that claim.

## A 5 ms budget

The budget is one moving frame at 1080p, pre-fill included, a check still running, the whole picture still produced this frame. It sums to 5.00 ms.

| Pass | Now | Budget | What has to be true |
|---|---:|---:|---|
| Raster, AA, overlay, copy | inside 0.9 | 0.40 | stays a small G-buffer |
| Compact | inside 0.9 | 0.15 | the million-slot touch stays; it only buckets |
| Place | inside 0.9 | 0.05 | 8 k probes |
| Probe trace and crossings | inside 1.5 | 0.25 | the launch is the verification stride, fills, and crossings |
| Cache rays and pre-fill | inside 1.5 | 0.40 | scouts stay; the ceiling is not spent because the sun ticked |
| Pixel direct and shadows | inside 11.5 | 1.00 | one visibility sample per pixel; the sun stays exact |
| Kept-hit and texel-direct shade | inside 11.5 | 0.25 | rings, not a lamp-range reshade of the kept tail |
| Gather | 0.9 | 0.15 | woken probes plus the stride |
| Filter | inside 0.9 | 0.10 | three rounds on the probes that gathered |
| Cache update | inside 0.9 | 0.15 | MSME on the texels that got rays |
| À-trous | 13.0 | 1.40 | a screen-sized spatial filter of this frame's bounce |
| Compose | 2.2 | 0.70 | full-picture direct write, four probes, hash find only at the existing fallback |
| **Frame** | **30.0** | **5.00** | |

The two structural cuts are the wavelet (13.0 → 1.40, save 11.6) and the light pass (11.5 → 1.25, save 10.25). Together with gather (save 0.75), compose (save 1.5), and the trace split (1.5 → 0.65, save 0.85), the frame lands on 5 ms. Each of those saves is the same milliseconds as several ideas below. Adding the idea column over-counts.

## Ranked ideas

Expected milliseconds are off the contended 30 ms moving frame at 1080p, five lamps, sun moving. Overlaps are named so the column is not a sum. Risk is the chance the picture misses the gate (mean / p99 against the current frame, and the gi-check). Picture impact is what a player would see if the save lands and the gate is the current one.

| # | Idea | Expected save | Risk | Picture | Source |
|---|---|---:|---|---|---|
| 1 | Change-driven wavelet: a sun step dirties the shadow boundary, and the stored sun share scales the rest | 8–10 ms, same milliseconds as 2 and 3 | medium | bounce at the moving shadow edge can trail by a filter step; direct is this frame | S63 sun share; S78B; Schied et al., SVGF, HPG 2017; ReLAX (fewer iterations as the sample settles). In progress. |
| 2 | Spatial wavelet on a screen-sized bounce image, rebuilt from this frame's texels every frame | 8–10 ms, same as 1; the pair is the 1.40 ms budget line | high | bounce can be softer; direct stays full resolution | Dammertz et al., edge-avoiding à-trous, HPG 2010; SVGF; Zhdan, ReBLUR (half-res indirect, joint bilateral upsample); Wright et al., Lumen, SIGGRAPH 2022 (the denoiser sees indirect). S79 missed because the launch stayed the hash table. |
| 3 | Leave 25 cm world-radiance-cache cells out of the screen wavelet | 3–6 ms, inside 1 and 2 | medium | the push already blends those cells; they stay the running mean | Lumen's world radiance cache; SHaRC's hash is this cache, not the screen filter. `lighting_svgf_active` is the line. |
| 4 | Ship probe ring wakes with a launch that is the live list (PR #31) | about 5.5 ms on `light` (11.5 → ~6), and part of the trace tail | medium | a missed ring leaves a probe until the stride hits it | [WAKE](../experiments/WAKE-neighbour-wake-design.md); DDGI (Majercik et al. 2019); hierarchical radiosity (Hanrahan 1991); group testing (Dorfman 1943). Unmerged. |
| 5 | One visibility sample per pixel, sun exact; hits and cache rays pick first (PR #30) | 1.5–2.5 ms of the 6 ms that remains after 4, on this five-lamp scene; several times that at 100 lamps | medium-high | penumbra noise unless the sample is stable per cell | Bitterli et al., ReSTIR DI, 2020; MegaLights (Epic); S78Q (shelved: +0.2 ms on this scene, washed wedges). Conflicts with ADR 0012's deterministic tile cut; the lamp-picking plan already names that. |
| 6 | Compose writes every pixel's direct and reads four probes; the 16-wide cache find stays on the weight fallback | 1.0–1.5 ms | low | thin geometry already takes that fallback | SUN's split (walk ~2.4 ms, write ~0.4 ms); [QUIET](../experiments/QUIET-quiet-paths-compose-filter-compact.md). The pass still runs. |
| 7 | Gather and filter only woken probes, plus the verification stride | 0.6–0.75 ms | medium | a missed wake holds SH until the stride | S78P; S78R3 (still gather 0.85 → 0.09 ms); Lumen's probe filter. The stride is the check. This is not a held frame. |
| 8 | Screen-space-weighted verification | 0.2–0.5 ms of rays; it is what makes 4 and 7 safe | medium | misses land where the picture is | A-SVGF (Schied et al. 2018); Lumen feedback. In progress. |
| 9 | Hardware ray query for shadow and probe rays, on cards that have it | 2–4 ms of the post-wake light pass, if the software walk is that time; alternative to part of 5, not an extra | medium | must match the software tracer | ADR 0012; `VK_KHR_ray_query`. The 1060-class floor and macOS stay on `scene_ray`. |
| 10 | Store the filtered texels in a card atlas so a 5×5 is coherent | 4–8 ms if the filter stays on the cache; little, once 2 has moved it to an image | high | same texels, different layout | Lumen mesh cards. Values still update every frame. This is a layout, not a bake. |
| 11 | Bounce probes at half the linear resolution, direct at full resolution, spatial upsample | 1–1.5 ms across trace, gather, and filter | medium | softer bounce, sharp shadows | Lumen's screen probes are already coarser than the picture; ReBLUR's upsample. A temporal upscaler that holds last frame's picture is out. |
| 12 | Three wavelet iterations instead of five, as a measured knob | 2–4 ms, inside 1 and 2 | high as a blind switch | S79's adaptive one-iteration picture missed the bar (mean 2.8 / p99 11–13 against 1.5 / 10) | ReLAX. Useful as a profiling lever. Not the 5 ms plan by itself. |
| 13 | Run the pre-fill scouts on the async queue beside the G-buffer | 0.2–0.4 ms | low | none if the scouts stay the same rays | The world queue already overlaps cache work (`GPU_OVERLAP`). Pre-fill stays in the frame. |
| 14 | fp16 for the screen-sized bounce image in idea 2 | 0.3–0.8 ms after 2; about 0 on today's hash gathers | low | a small quantize on bounce | `shaderFloat16` is optional on the floor (ADR 0012). The 4070 has it. |
| 15 | Subgroup ballot inside a screen tile, after idea 2 | under 0.3 ms on today's launches | low | none | Vulkan subgroup operations. Shaders do not assume a subgroup size. Empty hash slots are not the 13 ms, so a ballot over the million-slot launch saves little now. |
| 16 | ReSTIR GI reservoirs on texels | none worth scheduling; S72B was reverted | high | flicker from a reused path | Ouyang et al., ReSTIR GI; [S72B](../experiments/S72B-restir-gi-texel-reuse.md). A reservoir that is last frame's chosen path is a radiance hold. |
| 17 | SHaRC's 16-sample read, or a neural radiance cache | about 0 on this frame (S78H2 measured no effect) | high for NRC | a trained field lags | SHaRC; Müller et al., NRC. NRC was ruled out as the core on 1060-class cards. A trained field is a hold. The world-radiance cache is already the hash. |
| 18 | AMD GI-1.0 / Brixelizer as a reference for the brick budget and the screen denoise | no separate millisecond until the filter moves | — | — | AMD FidelityFX GI 1.0 and Brixelizer. The piece to take is idea 2 plus a fixed update budget, not a second GI. |
| 19 | Replace the cache filter with a DDGI probe volume | would delete the 13 ms and add a probe update of unknown size | high | a different picture | Majercik et al., DDGI. Probes would still trace every frame. Not the next step. |

### How the top of the list becomes 5 ms

Take one filter, not five. Idea 2 is the launch that can cost 1.4 ms: a screen-sized (or half-resolution) spatial filter, coherent loads, rebuilt from this frame's surface-cache means. Idea 1 decides which texels of layer 1 are new inputs, so a sun tick does not mark 108 k cells. Idea 3 keeps the 25 cm cells on their running mean so they are not in that image. Ideas 10, 12, 14, and 15 are either a way to implement that filter or a lever that spends the same 13 ms.

Take the light pass in order. Idea 4 removes the kept-hit launch the owner already measured at about 5.5 ms. Idea 5 then cuts the pixel shadows that are the remaining 6 ms, down toward 1.0 ms of direct plus 0.25 ms of woken hits. Idea 9 is the other way to make those shadow rays cheaper, on this card only, and it has to match `scene_ray`.

Then the small columns: idea 6 on compose, idea 7 on gather, idea 8 so the stride looks where the picture is, idea 11 if the probe grid is still above 0.25 ms, idea 13 so the required scouts overlap the raster. The 0.40 ms cache-ray line is where pre-fill lives.

What this ordering refuses, because of the constraints: baking the bounce, replaying last frame's picture, dropping the scouts, and treating S79's half-resolution hash as evidence that a screen-space filter cannot work. S79 showed that a million-slot hash does not get cheaper when half the slots return early.

## Profiling experiments

These run on the owner's RTX 4070. Lavapipe does not time the GPU. The bench lock on that machine stays in force (`~/genos-bench.lock`). Each run is 180 frames at 1920×1080, times printed every 30 frames, so the line is a mean over that window. Read `atrous`, `light`, `trace`, `gather`, `compose`, and `cache_rays`. `shadows` repeats `light`.

`GENOS_GPU_DISPATCH=1` adds `LIGHTING_DISPATCH` with one number per launch (`index:pipeline=ms`). The seven à-trous launches are pipeline 8. The two trace launches are pipeline 2. The two light launches are pipeline 3.

### 1. Baseline, this tree, moving

Confirms the 13 / 11.5 / 2.2 / 1.5 / 0.9 split on `5b38ca20`, and splits the pipelines the owner line adds together.

```
GENOS_GPU_DISPATCH=1 cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --dynamic 50 --gpu-times 30
```

### 2. What the sun and the movers are dirtying

Same camera and lamps. One run freezes the sun, one freezes the boxes, one freezes both and holds the lamps. If à-trous falls from the 11–15 ms band to the still 2–3 ms band when the sun is frozen, idea 1 is the moving-scene wavelet. If it stays high with a frozen sun and moving lamps, the lamps are the dirty set.

```
GENOS_GPU_DISPATCH=1 cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --dynamic 50 --freeze-sun --gpu-times 30

GENOS_GPU_DISPATCH=1 cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --dynamic 50 --still-boxes --gpu-times 30

GENOS_GPU_DISPATCH=1 cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --dynamic 0 --freeze-sun --still-boxes --gpu-times 30
```

`lightingatrous` (with `GENOS_LIGHTING_DEBUG=1`) prints the dirty fraction, `list` or `dense`, and the filtered count. A moving run that prints `dense` and a filtered count above 65,536 is the model in the wavelet section.

### 3. Wavelet iterations, and lists against dense

If `atrous` scales from 5 iterations to 1 by much less than 5×, the launch and the variance/push passes dominate and idea 12 is a small lever. If it scales almost linearly, the 5×5 finds are the cost and idea 2 is aimed at the right loop.

```
GENOS_GPU_DISPATCH=1 cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --dynamic 50 --denoise-iters 5 --gpu-times 30

GENOS_GPU_DISPATCH=1 cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --dynamic 50 --denoise-iters 1 --gpu-times 30

GENOS_LIGHTING_ATROUS_DIRTY=0 GENOS_GPU_DISPATCH=1 cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --dynamic 50 --gpu-times 30

GENOS_LIGHTING_ATROUS_DIRTY=1 GENOS_GPU_DISPATCH=1 cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --dynamic 50 --gpu-times 30
```

`ATROUS_DIRTY=0` is always the dense sweep. `=1` keeps the lists until the 65,536 cap.

### 4. Direct, isolated, and against lamp count

Potato is direct and compose, no GI buffers. The `light` column there is the pixel shadow walk without kept hits. Bounce 1 adds probes and drops the cache and the wavelet. The gap from potato's `light` to high's `light` is kept hits plus hit-light. The 5-lamp run against the 100-lamp run is whether idea 5 matters on the target scene or only in the hell room. `--layout first` matches the lamp-picking plan's hell-scene question.

```
cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset potato \
  --dynamic 50 --gpu-times 30

cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset gi-1bounce \
  --dynamic 50 --gpu-times 30

cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --bounces 0 --dynamic 50 --gpu-times 30

cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --lights 100 --layout first --dynamic 50 --gpu-times 30
```

### 5. Ring wakes, on the unmerged branch

The 11.5 → ~6 ms claim. Same moving frame, legacy against rings. The branch has `--wake`. This tree at `5b38ca20` does not.

```
cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --dynamic 50 --wake legacy --gpu-times 30 --lighting-debug --wake-log moving-legacy

cargo run --release -p genos-stress -- \
  --proof --no-panel --size 1920x1080 --frames 180 --preset high \
  --dynamic 50 --wake rings --gpu-times 30 --lighting-debug --wake-log moving-rings
```

### What would confirm or kill the model

- Dispatch dump: seven à-trous launches of similar cost, and the direct launch much larger than hit-light. A single à-trous launch near 13 ms means the model is wrong about iterations.
- Frozen sun drops `atrous` into the 2–3 ms band. If it does not, idea 1 is not the moving-scene lever.
- Potato `light` near 6 ms, and high `light` near 11.5 ms, matches "pixels remain after the kept tail." Potato `light` near 11.5 ms means the kept tail was never the 5.5 ms.
- `trace` staying near 1.5 ms with `--dynamic 0 --freeze-sun --still-boxes` means the 1.5 ms is the standing probe stratum, not crossings.
- `cache_rays` under a few tenths of a millisecond means pre-fill is already inside the trace budget and the 0.40 ms line is slack. `cache_rays` near 1.5 ms means the owner "trace" number is the cache-ray batch and the probe stratum is the small one.
