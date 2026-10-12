# Dynamic-scene performance plan (moving lamps and moving assets)

Written 2026-10-12 against `gi-v2` at e61b89d, as a code-reading evaluation. Nothing in this plan has been built or measured. Every gain below is an estimate with its reasoning, and each phase starts with a measurement that can prove the estimate wrong before any code lands.

Baseline (README "Current state", RTX 4070, stress scene, 1920x1080):

| Run | Frame time | fps |
|---|---|---|
| Still | 8.21 ms | 105 |
| Moving (`perf/moving.rhai`: dusk, lamps and boxes moving, 50 % dynamic) | 18.7 ms | 51 |

Moving-frame split, from the README sections S78B, S78K and S78P:

- Light pass: 6.60 ms in total.
  - Kept-hit relight: about 3.26 ms. Switching it off gives 3.34 ms.
  - Per-pixel direct light: about 2.74 ms. Switching it off gives 3.86 ms.
  - TDL lookup: costs 0.61 ms more than it saves. Switching it off gives 5.99 ms.
- A-trous: 4.83 ms.
- Gather: 0.95 ms.
- The rest (trace, three filter rounds, compose, compact, cache relight) is not split in any recorded moving profile.

Owner principles this plan keeps:

- Work is gated by need, and the budget is a ceiling.
- Re-trace effort follows how much results change.
- No readbacks happen in the frame loop.
- Lighting VRAM stays roughly fixed; more is acceptable if it cuts compute.
- Quality is uniform within a region.
- Do the cheap thing first and measure anything expensive.
- Every frame runs a check, and nothing is skipped on an assumption.

Each step must pass the README protocol before it is pushed:

- still and moving bisect runs (`GENOS_GPU_TIMES`, plus `GENOS_GPU_DISPATCH=1` for per-dispatch times)
- pop, and `lum a`
- VRAM
- a moving-shadow smoke test with screenshots that someone actually looks at

A step that saves no measurable time is reverted and recorded as an experiment file.

## Ranked summary

| Rank | Item | Moving gain (estimate) | Visual risk | VRAM |
|---|---|---|---|---|
| 1 | A: Light GI hits in their own compacted dispatch (wavefront) instead of inline in `lighting_direct` | 0.8-1.5 ms alone. Without it, the other caching items cannot pay off. | None: same math | +16 MiB queue |
| 2 | B: Shadow visibility for GI hits invalidated by what changed visibility, not by the lamp's light epoch | 0.8-1.5 ms on top of A | Low. Bounce shadows of moving lamps lag by at most one 12.5 cm shadow cell; pixels unchanged. | ~0 |
| 3 | D: A-trous dense mode in heavy-change frames, without scatter or stamps | 1-2 ms | Very low: an exact SVGF pass | ~0 |
| 4 | C: Direct light at the 25 cm WRC cell for hits with no fine texel (Lumen cards off screen) | 0.3-1 ms | Low: bounce input at 25 cm | 0 (slot-indexed) |
| 5 | E: Per-lamp pixel visibility reuse, exact by the existing epoch rules | 0-1.4 ms, depending on the scene (measure first) | None: exact | +8 MiB at 1080p |
| 6 | F: Screen-probe work gated by need, plus per-probe change culling | 0.5-1.5 ms (measure first) | Medium: needs a guard band and hysteresis | +32 KiB |
| 7 | G: Tracer any-hit for shadow rays, and mesh-instance bounds first | ~0 in the default stress scene; large with glTF content | None | 0 |
| 8 | H: Frame plumbing (copies, gate order, kept-thread work) | 0.1-0.3 ms | None | 0 |

Plausible combined result: moving 18.7 ms comes down to roughly 12-14 ms (70-80 fps). The still scene should not get slower, and A and B also help it a little. The spread is wide because no per-dispatch moving profile exists yet. Phase 0 fixes that first.

## Comparison with the published state of the art: where our structure deviates

| Topic | Published practice | Ours | Consequence for moving scenes |
|---|---|---|---|
| Direct light at GI hits | Lumen (Wright et al. 2022): hits read the surface cache's lit cards. Card direct light is updated under a fixed texel budget, and hits never trace lamps. MegaLights: per-pixel stochastic samples, with shadow rays traced in **separate compacted passes**. | TDL per fine texel (S75). On a miss, a new entry over budget, or no fine texel, the hit runs the whole lamp loop **inline in the same thread** (`lighting_direct.comp` 231-356). | Warps hold a mix of cache hits and misses, and every miss stalls its warp. This is why S77B (2/3 of shadow rays reused, light pass unchanged), S78M, S78O ("cost is the lighting itself") and S60A gained nothing. Items A and C. |
| Shadow cache invalidation | UE Virtual Shadow Maps invalidate only the cached pages that a moving primitive's bounds touch. | The whole lamp's epoch moves when the lamp moves or changes colour, or when any change sphere lies within its range (`lighting_cache.glsl` 3248-3277). | A moving lamp re-traces every hit shadow every frame, and a static lamp near any mover does too. Item B. |
| Per-pixel direct light | MegaLights: a fixed number of shadow rays per pixel, independent of the lamp count. | Every pixel loops over all its lamps every frame (about 5 in the moving scene, from S78Q) whenever anything moves anywhere (`lighting_pix_quiet`, 3387). | 2.74 ms. ReSTIR DI was shelved (S78Q). Item E is an exact per-lamp alternative. |
| Final gather | Lumen: screen probes for every pixel; the cache is read only at hits. GIBS: pixels shade from surfels, with no screen probes. DDGI (Majercik et al. 2021): probe classification switches off probes that cannot contribute. | Both at once: pixels read the surface-cache texels, and the full screen-probe pipeline (8160 probes x 64 rays, kept hits, gather, 3 filter rounds) runs for **every** tile. Pixels use probes only where world trust < 1. | We pay for a second final gather at full rate. Item F. |
| Denoiser | SVGF and A-SVGF (Schied 2017, 2018) filter every pixel every frame, with fixed taps on a grid. NRD ReBLUR/ReLAX adapt the radius to history. | Hashed texels: each tap is a hash lookup. Dirty-page levels (S77D), with scatter marks and work lists (S78X) that overflow under heavy change. | Change bookkeeping costs as much as the filtering itself in frames where most texels change. Item D. |
| GPU work structure | Wavefront and queue-based tracing (Laine et al. 2013, "Megakernels Considered Harmful"); Lumen and MegaLights compact trace lists by trace type. | One thread per candidate across 8.4M-thread dispatches, with the trace inline after data-dependent cache checks. | Items A and H. |
| Hash radiance cache | SHaRC: hash grid with bounded linear probing, entries evicted by age. | Same idea (1M slots, 16-slot linear search, no tombstones, so misses always scan all 16). | Misses cost about 2 cache lines. This is minor next to the items above; see D4. |

## Phase 0: measure before changing anything (no visual change)

These steps are measurement only. Each is a debug-only commit or no commit at all.

### 0.1 Moving per-dispatch profile (no code)

Run the moving bisect scene with `GENOS_GPU_TIMES=600 GENOS_GPU_DISPATCH=1`, and do the same for the still scene. Record every dispatch's time, in particular:

- trace pass 2 (probe rays, kept re-traces, fills)
- direct pass 3, in mode 1 and as dense rays
- gather pass 4
- filter pass 5, all three rounds
- cache pass 6
- the seven a-trous pass 8 dispatches, each one separately
- compose pass 7
- compact pass 0

The README has no moving split beyond the light pass and a-trous. Every later estimate is re-based on this table. Commit the table as `experiments/DP0-moving-profile.md`.

### 0.2 Counters (debug only, `LIGHTING_DIAG`)

The `diag[64]` slots are printed by `trace_sc` (`lighting_gpu.rs` 1133). Slots 40-56 are used; 0-39 and 57-63 are free. Turn `LIGHTING_DIAG` (`lighting_cache.glsl` 135) to true only in a measurement build. Add these counters with `lighting_dg(i, 1u)`:

| Slot | Counted where | Meaning |
|---|---|---|
| d0 | `lighting_direct.comp`, just before `trace_occluded` (343), `idx >= pixels` | shadow rays traced by GI hits |
| d1 | same, `idx < pixels` | shadow rays traced by pixels |
| d2 / d3 | `lighting_shadow_get` (3285): returned >= 0 / -1, for `li < LAMP_ALL - 1` | hit shadow cache hit / miss |
| d4 | `lighting_direct.comp` 235, `ts == ~0u` | GI hits with no fine texel (full lamp loop) |
| d5 | `lighting_direct.comp` 259 branch failing its budget test | GI hits over the TDL budget (full lamp loop) |
| d6 | pixels, inside the lamp loop: lamp `li`'s epoch did not move this frame | (pixel, lamp) pairs that item E could reuse |
| d7 | pixels, inside the lamp loop | (pixel, lamp) pairs in total |
| d8-d12 | `lighting_atrous.comp` 13: `cnt` per level 1-5 (`atomicMax`, one thread) | work-list size per level, and overflows (> 65536) |
| d13 | `lighting_svgf_variance` 2857: recomputed | stage-0 texels recomputed |
| d14 | `lighting_svgf_iter` 2941: `moved` | level outputs that scattered |
| d15 | `lighting_compose.comp` 75: `world.w < 0.999` | pixels that read probes |
| d16 | compose: tiles with at least one such pixel (per-tile flag in the work buffer, counted in a later pass, or estimated offline from the dump) | tiles that need probes |
| d17 | `lighting_cache_find_sized` (565) returning `~0u` with `claim == false` | hash lookups that miss |
| d18 | the same function returning a slot | hash lookups that hit |

For d6: per-lamp "moved this frame" is `ckeys.key[LIGHTING_SHADOW_EPOCH_AT + 8u*li]` compared with a copy taken at the start of `lighting_shadow_epochs()`. Add that copy in the free word `at + 6u` (words `at + 6`, `at + 7` of each 8-word lamp block are unused today; see 3256-3275).

Also run the existing S78S change counters (`GENOS_LIGHTING_SC_TRACE=1`, `perf/chg.py`). `CHG_KEPT_TDL_REUSE / CHECK / LAMPS` (slots 6-8) already give the direct-light source split of the kept hits.

### 0.3 Divergence test (debug only; decides whether item A is the lever)

Add `const uint DP_DIVERGE = 0u;` to `lighting_direct.comp`. Rebuild once per value. Under each value, a GI hit (`idx >= pixels`) skips `trace_occluded` and takes `shadow_lit = true`:

1. every hit
2. a random half, `lighting_hash(idx) & 1u`
3. a coherent half, `lighting_hash(idx >> 6u) & 1u`: the whole 64-thread group decides together

Compare the direct-pass dispatch time in the moving scene. The picture is wrong under this toggle; never push it on.

How to read the result:

- If (2) saves far less than half of (1) while (3) saves about half, divergence is confirmed and item A is the main lever.
- If (2) and (3) save the same, the cost is ray count. Then rank B and C above A, and A becomes optional.

## Item A: light GI hits in their own compacted dispatch

### Problem

The light pass is one dispatch over `w*h + 2*kept` threads (`lighting_gpu.rs` 1433, 1451). Pixels come first. Kept probe hits (8160 probes x 64 rays x 8 strata = 4.18M) and fill slots (another 4.18M) follow. With a cache round, light-cache rays get a second dispatch (1452).

A GI-hit thread runs these steps in order:

1. Gates: 172-213.
2. TDL lookup and signature: 231-268, including a 16-slot hash search and a lamp-epoch hash loop.
3. Either one TDL read, or a lamp loop with shadow-cache reads and inline `trace_occluded` calls (309-356).

Neighbouring kept rays of one probe stratum go in 64 directions and land on unrelated texels and cells. TDL hit or miss, shadow cache hit or miss, and lamp count therefore vary thread by thread. A warp finishes only when its slowest thread does. If two thirds of the threads are served from caches but one thread in each warp traces 5 rays, the warp costs about 5 rays.

Recorded evidence that this is the bottleneck:

- S77B: 2/3 of shadow rays reused, light pass unchanged at 8.6 ms.
- S78O: the texel lookup removed, about 0.06 ms; "cost is the lighting itself".
- S78M (one TDL owner per texel) and S60A (picks on probe hits): no gain.
- S78K and S78J: kept relight about 3.26 ms for about 1.08M relit hits, about 3.0 ns a hit. Pixels take about 2.74 ms for 2.05M pixels, about 1.3 ns a pixel, and each pixel traces all of its ~5 lamps. A relit hit therefore costs about 2.2x a pixel, even though many hits are served by the TDL and never trace.

### Proposed change

Split hit lighting into classify, queue, and light. This is wavefront tracing as in Laine et al. 2013, and the structure MegaLights and Lumen use for their trace lists.

1. In `lighting_direct.comp`, after the TDL decision (line 268) and only for `idx >= pixels`, check whether all of the hit's needed shadow terms are already known: either TDL reuse, or every lamp with `w > 0` has `lighting_shadow_get >= 0`.
   - If yes, finish inline as today. This path is coherent and cheap.
   - If no, append `uvec2(idx, tdl)` to a hit-light queue and return. `tdl` carries the 0x80000000 / 0x40000000 flags and the texel slot.
   - Use one `atomicAdd` per thread on a counter in `ckeys`. Subgroup aggregation would need SPIR-V 1.3; see note A-n1.
2. A new pass, the hit-light pass, is the same `lighting_direct.comp` compiled with `-DLIGHTING_HIT_QUEUE`. One thread per queue entry runs the hit's lamp loop and its write (the code from line 285 to the end). It takes `idx` and `tdl` from the entry instead of repeating the classification. It never runs the TDL budget `atomicAdd` (259) or the gates again, because their side effects already happened.
   - The file keeps a single `trace_occluded` call site in each compiled variant, as `lighting_common.glsl` 243 requires.
3. A tiny args step writes the indirect dispatch size from the queue counter. Reuse the S70A mechanism: pass 0 with `LIGHTING_ARGS` already writes `LIGHTING_DENSE_*` args at `LIGHTING_ARGS_AT`; add a third triple.
4. Order in `lighting_passes` (`lighting_gpu.rs` 1423-1468):
   - Without a round: … direct pass 3 → args → hit-light pass → gather pass 4 ….
   - With a round: direct (mode 1) → direct (dense rays) → args → hit-light pass. One queue serves both direct dispatches.
   - Reset the counter in the args step after it is read. Each frame then starts empty with no host work.
5. Queue capacity: `LIGHTING_HIT_QUEUE_CAP = 2^21` entries (16 MiB of `uvec2`). When the queue is full, the thread lights inline as today. The budget stays a ceiling, and nothing is dropped.

### Paper / reference

- Laine, Karras, Aila, "Megakernels Considered Harmful: Wavefront Path Tracing on GPUs", HPG 2013.
- MegaLights (Epic, UE 5.5; Advances in Real-Time Rendering 2025): separate sample, trace and shade passes over compacted lists.
- Lumen (Wright et al. 2022): screen-probe traces are compacted by trace type before tracing.

### Expected gain

If Phase 0.3 shows strong divergence, the queued hits run at near-full SIMD occupancy, and the cached hits leave the queue entirely. Ray memory incoherence remains: scattered rays still miss cache. Kept relight could fall from about 3.26 ms to 1.8-2.4 ms, a saving of 0.8-1.5 ms. The light-cache dense rays gain a little too. The still scene gains little, because only about 0.15M kept hits are rewritten per frame (S78S). The extra dispatch, barrier and args step cost about 0.02-0.05 ms there.

### Risk to visuals

None by construction: the math is the same and runs in the same frame. The only order-dependent element is which hits win the TDL budget's `atomicAdd`, and that is already nondeterministic.

### VRAM

+16 MiB queue, plus 12 bytes of args. Put it in a new buffer, or grow `cache_keys`. If `cache_keys` grows, also grow its size where `lighting_gpu.rs` allocates it and its binding range.

### Measure

- direct-pass time plus hit-light-pass time, against the old direct-pass time, in the moving and still runs
- the queue length (diag slot)
- an unchanged picture: pixel-exact compare of a still frame, using the S78S dump's per-pixel direct and kept-light CSVs from before and after

### Steps (one commit each)

1. **A1**: Add the queue buffer region and constants (`LIGHTING_HIT_QUEUE_AT`, `LIGHTING_HIT_QUEUE_CAP`, a counter word) and its allocation. Nothing uses them yet. `cargo build --workspace` passes.
2. **A2**: `build.rs`: let `compile()` take extra `-D` defines. Add a second compiled variant `lighting_direct_queue.spv` with `-DLIGHTING_HIT_QUEUE`, and a pipeline slot (`pipes[9]`). It is not dispatched yet.
3. **A3**: Args-step support: the queue's indirect args are written and the counter is reset. Dispatch the hit-light pass, which returns immediately because the queue is still empty. Measure the overhead of the empty pass in the still scene; it should be at most 0.02 ms.
4. **A4**: Enqueue light-cache-ray hits only: `cache_pick` and the dense dispatch, the smaller population. Compare the dump and the times.
5. **A5**: Enqueue kept and fill hits. Measure moving and still, then record `experiments/DP-A-hit-light-queue.md`.
6. **A6** (optional, only if A5 shows the queued pass itself diverging on lamp count): one thread per (hit, lamp-with-a-miss) shadow ray, with a per-hit reduction pass. Do this only if measurement asks for it.

Note A-n1: `build.rs` invokes `glslc` without `--target-env`, which means SPIR-V 1.0 with no subgroup ops. Use plain atomics in A. Moving the compile to `--target-env=vulkan1.1` would enable `subgroupBallot` aggregation, but change it only as its own measured step: the NVIDIA compiler has hung on this tracer before (README, "v2 became a separate frame path").

## Item B: hit shadow visibility follows what changed visibility

### Problem

`lighting_shadow_epochs()` (`lighting_cache.glsl` 3248-3277) moves lamp `i`'s single epoch when any of these happens:

- its position moves, at any distance
- its colour changes, which does not change visibility
- its range changes
- any change sphere lies within its range

The hit shadow cache (`lighting_shadow_get` 3285, `put` 3300) and the TDL signature (`lighting_lamps_sig` 3329) both key on this epoch.

In the moving scene, a moving lamp therefore invalidates every hit shadow in its range every frame. So does a static lamp with any moving box in range. Every relit kept hit in range re-traces that lamp (S78I: "at dusk moving lamps dominate wherever they reach"; S78D/S78K: "moving lamps relight the kept hits").

The hit shadow cache is already quantized to a 12.5 cm receiver cell (`lighting_shadow_cell` 3279-3283). Exact per-frame visibility for a lamp that moved 2 cm is therefore precision the cache's own resolution cannot show.

### Proposed change

Split each lamp's state into two epochs:

- **Light epoch** (today's word `at`): unchanged rules. It feeds `lighting_lamps_sig` (TDL), `lighting_wrc_lamp_changed`, and the pixel signatures. A moving lamp's light is still recomputed. That costs analytic `lamp_light` ALU only, with no rays.
- **Visibility epoch** (new word): read only by `lighting_shadow_get` / `lighting_shadow_put`, which only GI hits use (`li = LAMP_ALL` for pixels, `lighting_direct.comp` 338). It moves when:
  - the lamp's range changes, or the lamp turns on or off
  - the lamp is more than `LIGHTING_SHADOW_MOVE = LIGHTING_SC_TEXEL` (12.5 cm) from the position where its visibility epoch last moved (an anchor kept per lamp)
  - a whole-world change happens
  - a change sphere within range: unchanged rules in B1-B3, refined in B5
  - a verify failure (below)

  It does not move on colour changes.
- **Verify, as required by "never skip on an assumption"**: one shadow-cache hit in `LIGHTING_SHADOW_VERIFY = 16` (by `lighting_hash(cell tag ^ frame)`) is traced anyway, which happens in the queued hit-light pass after A. If the traced answer differs from the cached one, the entry is overwritten. In addition, a per-lamp mismatch counter in `ckeys` is incremented. When the counter passes a small threshold (for example 8 a frame), the next `lighting_shadow_epochs()` moves that lamp's visibility epoch and resets the counter. This is the S76 check-then-escalate pattern. It stays GPU-resident, with no readback.
- **B5 (later)**: per-entry change-sphere test instead of a whole-lamp bump, for static lamps near movers (the VSM page idea):
  - Keep a ring of the last `H = 4` frames' change spheres (4 x 64 `vec4` = 4 KiB, copied from the change block in `lighting_shadow_epochs()`).
  - Keep per lamp and per ring frame a 64-bit mask of the changes within the lamp's range (128 lamps x 4 x 2 words).
  - An entry stores the frame it was written. On `get`:
    - if the visibility epoch matches and the entry is at most `H` frames old, test the segment from the cell centre to the lamp against only the masked spheres of the frames since it was written (`lighting_seg_dist2`, `lighting_trace.comp` 40)
    - an entry older than `H` frames is valid only if the lamp had no in-range change since it was written (a per-lamp "last in-range change frame" word)
  - With this, the change-sphere rule no longer moves the visibility epoch. It is only a fallback when the ring overflows.

### Paper / reference

- UE5 Virtual Shadow Maps, cached-page invalidation by primitive bounds: only pages a moving object touches are re-rendered.
- Our S76 confirm-then-escalate and S74A shadow reuse, whose own paper basis is VSM cached pages.

### Expected gain

Take a lamp at 1 m/s and about 50 fps. It moves 2 cm a frame, so its visibility epoch moves once every 6 frames instead of every frame. Its hit shadow rays drop about 6x. Static lamps near movers keep invalidating until B5.

On top of A, if half of the kept-hit shadow terms belong to moving lamps, hit shadow rays drop by about 40 %. That saves 0.5-1 ms. B5 adds 0.3-0.5 ms where static lamps sit near movers.

Without A, the gain is much smaller, because hits and misses still mix inside warps. This is why B follows A.

### Risk to visuals

- Pixels (direct shadows) are untouched.
- Bounce light from a moving lamp sees occluders that lag by at most 12.5 cm of lamp travel, about one shadow-cache cell. The bounce then passes through the probe filter (3 rounds) and the a-trous levels, which blur far more than 12.5 cm.
- Run the `lampret` and `flick` gi-check tests and the moving-shadow smoke test.
- B5 risk: a sphere ring that misses a change whose bounds were not listed (whole-world changes already force a bump).

### VRAM

- B1-B4: 0. Use a new 128-lamp x 8-word block in `ckeys` for the visibility epoch, its anchor xyz, and the mismatch counter. Each lamp's existing block has only 2 free words, so it cannot hold these.
- B5: about 8 KiB, plus one word per shadow entry (today's entry `vec4` has its `w` free, line 3306).

### Measure

- d0 (hit shadow rays) and d2/d3 (shadow cache hit rate), from Phase 0
- the hit-light-pass time
- the verify mismatch rate (a new diag slot)
- `lampret`, `flick`, the moving-shadow smoke screenshots, pop

### Steps

1. **B1**: Add the visibility-epoch block. `lighting_shadow_epochs()` writes it with **the same rules as today**: it moves whenever the light epoch moves. `shadow_get` and `shadow_put` switch to it. There should be no behaviour change; confirm with an identical dump.
2. **B2**: Colour-only changes no longer move the visibility epoch. Fire and flicker cases gain.
3. **B3**: Lamp travel tolerance with an anchor.
4. **B4**: The verify trace plus the escalation counter. Push B3 and B4 together only if B3 alone shows an artefact. Otherwise B4 is its own commit, and it must land before B3 is pushed, per the principle that nothing is skipped without a check.
5. **B5**: Per-entry change-sphere ring test (static lamps near movers). Measure on its own.

## Item D: a-trous in heavy-change frames

### Problem

The seven SVGF dispatches (`lighting_gpu.rs` 1459-1465) each run over 1M slot threads. Levels 1-5 run their S78X work list when it has at most `LIGHTING_ATROUS_LIST_CAP = 65536` entries (`lighting_atrous.comp` 12-19). Otherwise they fall back to the full sweep and check per-slot stamps.

Under moving lamps most filtered texels move by more than `LIGHTING_DIRTY_EPS` (1 %), so the bookkeeping does work that informs nothing. For each changed texel:

- **Stage 0** (`lighting_svgf_variance` 2792-2868): 24 neighbour lookups to scatter, plus 48 lookups and writes to stamp the 7x7 spatial-variance readers (2861-2866). A young texel's spatial estimate adds another 48 lookups (2823-2840).
- **Levels 1-5** (`lighting_svgf_iter` 2869-2953): 8 lookups for the g3x3 variance and 24 for the 5x5 taps, plus 24 lookups, 24 `atomicExchange` and up to 24 `atomicAdd` for the scatter (`lighting_svgf_scatter` 2750, `lighting_svgf_mark` 2729). Level 5 adds 4 more for the push stamps.

Each lookup (`lighting_sc_cell_slot` 2603) reads the texel's position and normal (32 B), computes the key, and searches 16 consecutive slots. A miss always scans all 16 (`lighting_cache_find_sized` 568-579), because frees leave no tombstones.

So in a frame where nearly everything changes, about 40 % of the lookups and all the atomics are change tracking whose answer is "everything".

### Proposed change

1. **Dense mode per level, decided on the GPU.** Keep last frame's list count per level. Copy each counter to a "prev" word when it is reset; the reset is wherever `lighting_atrous_list(k)` is zeroed each frame (grep the reset in `lighting_compact.comp` or the args pass).
   - If last frame's level-k count was more than `LIGHTING_ATROUS_DENSE = 0.5 x` the active-texel count (or simply overflowed the cap), level k runs dense this frame:
     - every active texel is recomputed (`lighting_svgf_is_dirty` is bypassed)
     - `lighting_svgf_scatter` and `lighting_svgf_mark` are skipped for level k+1
     - stage 0, in dense mode, skips the 7x7 stamp loop (2861-2866), because every spatial estimate is redone anyway
   - Hysteresis: leave dense mode only after 2 frames below 0.3 x.
   - Level 5's push stamps (2944-2949) are still written, because the push stays change-driven.
   - The active-texel count is a counter the variance stage already passes through. Add an `atomicAdd` per active slot in stage 0, or reuse an existing live count if one exists.
2. **Optional D4, after D1-D3 measure: a cached neighbour index for the g3x3 taps.** The 8 step-1 neighbours are the same for all 5 levels and the push's ±1 taps. Store 8 slot ids per slot (32 MiB for 1M slots).
   - Validate an id with one read: `ckeys.key[id] == expected key`. The expected key is cheap ALU from the cell.
   - On a mismatch, do the full find and rewrite the id.
   - This saves 7 of 8 hash searches for those taps at levels 2-5.
   - Do it only if Phase 0 d17/d18 and the D3 profile show lookups dominating.
3. **Not proposed**: list-driven indirect a-trous (S78A was rejected because of atomic order and locality), and per-slot level skipping by variance (too little evidence of gain).

### Paper / reference

- Schied et al. 2017 (SVGF) filter every pixel every frame; the dense path is that.
- Dammertz et al. 2010 (a-trous).
- The dirty-page scheme (S77D) is our deviation. Dense mode returns to the paper's behaviour exactly when the deviation has nothing to save.

### Expected gain

In frames where everything changes, dense mode removes about 40 % of hash lookups and all the mark atomics. Moving a-trous could fall from 4.83 ms to about 3-3.5 ms. Still frames are unaffected, because they stay on lists. D4 might add 0.3-0.8 ms if lookups dominate.

### Risk to visuals

Dense levels are exact SVGF, while dirty mode holds changes under 1 %. A texel can step by up to 1 % when the mode switches. The hysteresis keeps switches rare. Watch pop at the switch frames.

### VRAM

- D1-D3: 5-10 words.
- D4: +32 MiB, which is within "more VRAM is fine if it cuts compute"; record it.

### Measure

- the seven a-trous dispatch times from 0.1
- d8-d14
- pop and `lum a` in the moving and still runs
- a screenshot of a dusk moving-lamp frame, dense against dirty

### Steps

1. **D1**: Persist the previous per-level list counts and the active count. No behaviour change.
2. **D2**: Dense mode for levels 1-5 (bypass the dirty check and skip the scatter). Measure.
3. **D3**: Dense stage 0 (skip the 7x7 stamps and scatter marks when level 1 is dense next). Measure.
4. **D4** (optional): Neighbour-id cache for the step-1 taps.

## Item C: direct light at the WRC cell for hits with no fine texel

### Problem

The TDL path (`lighting_direct.comp` 235-268) only looks up a **fine** texel, with `lighting_cache_find_sized(pos, n, false, LIGHTING_SC_TEXEL, pos)`. Fine texels exist only where pixels saw the surface (`lighting_svgf_active` 2634). Probe and cache rays often hit surfaces behind the camera, off screen, or occluded. Those hits fall through to the exact loop with every lamp shadow-traced.

`LIGHTING_PROBE_PICKS` is false (`lighting_cache_slots.glsl` 19), so there are no picks. With about 5 lamps per point, those hits pay about 5 shadow rays. Lumen covers off-screen surfaces with cards, so hits never trace lamps.

### Proposed change

When the fine lookup misses, look up the 25 cm WRC cell slot with `lighting_cache_find_sized(pos, n, false, 0.25, pos)`, the same size `lighting_svgf_push` uses (2986, 3002). Then use `LIGHTING_TDL_AT + slot` exactly like a fine texel:

- same signature, check, budget and escalation code
- origin at the cell's stored centre and normal (`cache.v[3*slot].xyz`, `cache.v[3*slot+1].xyz`), since `lighting_sc_texel_world` assumes a fine texel

The TDL slot block is already indexed by slot, so WRC slots have entries that are unused today. This needs no new memory.

### Paper / reference

Lumen (Wright et al. 2022), surface-cache direct lighting: cards cover meshes whether or not they are on screen.

### Expected gain

Phase 0 d4 gives the share of hits with no fine texel. If a third of relit kept hits take the full loop, C replaces about 5 lamp evaluations and up to 5 shadow rays with one read. After A and B that saves about 0.3-1 ms. The TDL budget (32768) is shared, so watch d5. A separate WRC budget of the same size may be needed.

### Risk to visuals

The direct light feeding off-screen bounces is resolved at 25 cm instead of per hit point. Sharp lamp shadows on off-screen surfaces blur by one cell before they become bounce light, which the probe filter and a-trous blur much more anyway. Run the `room` and `turn` gi-check tests, because they cover off-screen surfaces coming into view.

### VRAM

0.

### Measure

- d4 and d5
- `CHG_KEPT_LAMPS` dropping
- hit-light-pass time
- room, turn, pop, brightness

### Steps

1. **C1**: `lighting_sc_cell_world(slot, out p, out n)` helper. No behaviour change.
2. **C2**: WRC fallback for the TDL lookup in `lighting_direct.comp`, behind `const bool LIGHTING_TDL_WRC`. Measure.
3. **C3** (only if d5 shows budget starvation): a separate WRC TDL budget counter.

## Item E: per-lamp pixel visibility reuse (exact)

### Problem

S78J reuses a pixel's direct light only in fully quiet frames (`lighting_pix_quiet` 3387: no change and no mover anywhere). In the moving scene, every pixel therefore traces every lamp every frame: about 2.05M x 5 shadow rays, about 2.74 ms. That includes lamps that did not move and whose range holds no change.

Earlier attempts were rejected for good reasons:

- S74B, S78N: a segment test against change bounds, plus verify.
- S77B: per-pixel shadow reuse with check and escalate, which tore moving shadow edges.
- S78Q: ReSTIR DI, shelved with more noise.

### Proposed change

Reuse at **lamp** granularity, under the same exactness rule S78J already relies on. A lamp's epoch moves when the lamp moves or when any change touches its range (3263-3267). So a lamp whose epoch did not move this frame has exactly the same visibility as last frame from an unchanged surface point.

- Per pixel, store a signature over the surface point (1 mm), the normal, the sidedness and the lamp list **indices**, without epochs. Also store a 32-bit visibility mask over the lamp list's entries and the frame written.
- In a frame where the signature matches and the record is from frame − 1:
  - lamp k whose epoch did not move this frame (Phase 0's moved-this-frame word) takes its visibility bit
  - moved lamps are traced
  - `lamp_light` (analytic) is recomputed for all lamps, so colour and intensity changes are exact
- S78J's fully quiet path stays as it is (no lamp loop at all).
- Coherence comes for free: neighbouring pixels share a lamp list and the same per-lamp decisions. Unlike S77B's per-pixel cache hits, whole warps skip or trace the same lamp.

### Paper / reference

- MegaLights / ReSTIR (Bitterli et al. 2020) address many lights stochastically. That path is shelved (S78Q).
- This is the deterministic temporal-visibility-reuse special case, the same rule as UE's static shadow caching, applied per lamp.

### Expected gain

Proportional to f = d6/d7 from Phase 0: the moving-frame share of (pixel, lamp) pairs whose lamp is static with no change in range. Expected gain ≈ f x 2.7 ms x (share of the time spent in shadow rays).

If the moving scene's lamps all move or all have movers in range, f is about 0, and **E should not be built**. A typical game scene, with a few moving lamps and local movers, gets most of it. Fire and flicker colour changes are covered as soon as B2's colour split is also applied to the pixel-side rule.

### Risk to visuals

None. The reuse is exact under the epoch rules, the same guarantee S78J has run on since it was pushed. Still run the moving-shadow smoke test, given the S77B history.

### VRAM

Today a pixel holds 8 B (S78J: signature and light, 7.9 MiB at 1080p). Adding the mask and frame makes it 16 B: +7.9 MiB at 1080p, fixed per resolution.

### Measure

d1 (pixel shadow rays), d6/d7, the direct-pass time, moving-shadow screenshots, and a pixel-exact compare against the baseline in a moving frame (it must be identical).

### Steps

1. **E0**: Decide from d6/d7. Build only if f is above about 0.2 in the moving run or in a representative game scene.
2. **E1**: Per-pixel record layout grows to 16 B. Write the new fields; nothing reads them yet.
3. **E2**: Per-lamp reuse in non-quiet frames. Compare pixel-exact output in a moving frame; it must be identical.

## Item F: screen-probe work gated by need

### Problem

Every tile gets a probe every frame (`lighting_place.comp` 9-58). Each probe gets:

- 64 rays per frame (`lighting_gpu.rs` 1432)
- kept-hit re-traces (`lighting_trace.comp` 117-125) and relights (`lighting_direct.comp` 172-213)
- a one-workgroup gather (1434)
- three filter rounds (1435-1437)

Pixels read probes only where the surface cache's trust is below 1 (`lighting_compose.comp` 70-75). That means:

- slanted normals under flag 2048 (`lighting_world_irradiance_in` returns trust 0, 2394-2396)
- texel holes (trust scaled by `min(4 wsum, 1)`, 2501-2503)
- object texels with few sets (2489-2493)
- stale texels when the estimator is off (2487)
- first-reveal seeds (S72M/N, 2462-2473)

Flat static walls, floors and ceilings under settled texels never show their probe. Yet in moving frames those probes still re-trace crossing kept rays and relight every kept hit in range of a moving lamp.

There is a second waste in the trace pass. Every kept hit of 7 strata (3.66M threads) runs `lighting_kept_crossed` (`lighting_trace.comp` 53-79): it rebuilds its ray and loops over up to 64 change spheres, per ray.

### Proposed change

1. **Need mask.** Compose writes a per-tile flag (1 bit per tile, about 1 KiB at 1080p) when any pixel of the tile has `world.w < 0.999`, or `lighting_px_probes` was consumed as a seed.
2. **Active set.** Next frame's place pass marks a probe active when any of these holds:
   - its tile, dilated by `LIGHTING_PROBE_NEED_DILATE = 2` tiles, was needed in the last `LIGHTING_PROBE_NEED_HOLD = 8` frames (one full stratum cycle)
   - it lies within 2 tiles of the screen edge (camera motion guard band)
   - its tile was disoccluded: the probe moved more than its plane test allows, which place already computes for holding
3. **Dormant probes**: `pp.w = 0` in `work.v[lighting_probe_at(i)]`.
   - Trace, direct, gather and filter already skip `pp.w < 0.5` probes (`lighting_filter.comp` 39, 69; `lighting_compose.comp` 48).
   - Their kept hits stay in memory.
   - When a probe becomes active again, the fill mechanism (`lighting_fill_ray`, `lighting_common.glsl` 428) traces its missing strata in the same frame, as it does today for new probes.
   - Make sure a probe's stale strata are treated as foreign or empty, so it does not resume kept hits from frames ago. Use the S78K `until` mechanism: set `until = frame + STRATA` on wake.
4. **Per-probe change cull** (also applies when F1-F3 are not built). Before the per-ray crossing loop, test the change spheres once per probe and stratum against a bound: the probe's position plus the maximum kept-hit distance of that stratum.
   - Store that maximum, a float per probe and stratum (8160 x 8 x 4 B = 255 KiB), when the stratum is traced (`lighting_trace.comp` 168-178).
   - The rays of a stratum are consecutive threads, so the early-out is warp-coherent.

### Paper / reference

- DDGI probe classification (Majercik, Marrs, Spjut, McGuire 2021, "Scaling Probe-Based Real-Time Dynamic Global Illumination for Production", JCGT): probes that cannot contribute are switched off and not updated.
- Lumen traces only screen probes that are placed on visible G-buffer surfaces.

### Expected gain

Proportional to the dormant share, from d16 against the 8160 tiles. If 40 % of tiles are dormant after dilation, F1-F3 save 40 % of the probe pipeline. After A and B that pipeline is roughly 2.5-3.5 ms moving, so the saving is about 1-1.4 ms. F4 saves 0.1-0.4 ms, depending on the number of change spheres.

If the moving scene's movers and slanted surfaces cover most tiles, F1-F3 save little. **Decide from d16**.

### Risk to visuals

This is the main risk in this plan.

- A dormant probe's neighbours lose it from the filter's 2/4/8-probe reach (`lighting_filter.comp` 63), so edge probes of an active region get fewer neighbours, which is a patchwork risk.
- Mitigations: the 2-tile dilation, the 8-frame hold, the screen-edge band, and keeping `LIGHTING_PROBE_NEED_DILATE` as a single constant to tune against the gi-check `room` blotch and grid redraw tests.
- Waking takes a frame. For one frame, a first-reveal texel falls back to the S72L hit-patch and parent path instead of the probe seed.

### VRAM

+1 KiB need mask, +255 KiB stratum maximum distances.

### Measure

- d15 and d16
- trace, direct, gather and filter times
- gi-check `room` (blotch, grid redraw), `turn`, `approach`, and the moving-object `spin`
- pop

### Steps

1. **F1**: Per-probe change cull (F4 above). This step is independent of gating and carries no visual risk. Measure.
2. **F2**: Need mask written by compose (no consumer yet). Dump it as an image to look at.
3. **F3**: Place pass reads the mask with dilation, hold, edge band and the wake rule, behind `const bool LIGHTING_PROBE_GATE`. Measure, and look at the screenshots.
4. **F4**: Tune the dilation against the gi-check tests. Record the experiment.

## Item G: tracer any-hit and instance culling

### Problem

`trace_occluded` (`lighting_common.glsl` 254-262) calls the closest-hit `trace_ray`:

- `scene_ray` walks the occupancy grid (`scene_rays.glsl` 363-451) and keeps testing cells after a hit when the hit lies past the cell exit (426).
- It then tests the floor and roof planes (466-486).
- It then runs `mf_trace` (`mesh_field.glsl` 218-227) over **every** mesh-SDF instance. `mf_trace_one` loads the full 32-word record (`mf_inst`, 51-70) before the bounds slab test (167), and marches up to `FIELD_STEPS = 96` steps.

A shadow ray needs any hit, not the nearest one. `lighting_hit_glow` in the trace pass also loops over all instances (`lighting_trace.comp` 25). The default stress scene has no mesh instances unless `--gltf` is given (`examples/stress/src/main.rs` 240), so this does not show in the benchmark. It scales linearly with the content.

### Proposed change

1. An `any_hit` parameter through `scene_ray_masked`, `occ_walk`, `occ_test_list` and `mf_trace`:
   - test the floor and roof planes first
   - return on the first opaque hit in `[t0, t1]`
   - skip `mf_trace` entirely if the analytic part already hit
   - `trace_occluded` passes true
   - keep a single call site per shader (`lighting_common.glsl` 243)
2. `mf_trace_one`: read the 6 bounds words (12-14, 16-18, as `mf_meets` does at 248-250), run the slab test, and only then load the rest of the record.
3. Later, only if `--gltf` scenes show `mf_trace` in the profile: a uniform grid or BVH over instance bounds, so a ray visits only nearby instances.

### Paper / reference

Standard any-hit shadow rays (Vulkan `gl_RayFlagsTerminateOnFirstHitEXT`, the same flag for hardware ray queries). The Lumen mesh-SDF tracing paths cull SDF instances by bounds per tile or ray before marching.

### Expected gain

About 0 to 0.2 ms in the stress scene. With N glTF instances, the shadow-ray cost of `mf_trace` drops from N full record loads to N bounds reads plus the few instances actually hit.

### Risk to visuals

None (same occlusion answer).

### VRAM

0.

### Measure

Moving and still runs with and without `--gltf`, plus the direct and hit-light dispatch times.

### Steps

1. **G1**: Bounds-first `mf_trace_one`. In `lighting_hit_glow`, test the emission word (`glow == 0u`) and the bounds before `mf_inst(i)` loads the record (`lighting_trace.comp` 24-29).
2. **G2**: `any_hit` through the analytic walk and the planes.
3. **G3**: `any_hit` early-out in `mf_trace`.
4. **G4** (only on a measured need): an instance grid.

## Item H: frame plumbing

1. **H1: gate before reconstruction.** In `lighting_direct.comp` 139-158, a kept-hit thread loads its record and rebuilds its stratum ray (`lighting_stratum_ray`, 153) before the stratum gate at 178-184, which sends about 75 % of kept threads home. Move the `s != cur && s != cur + STRATA/2` test above the reconstruction. The `urgent && near_change` case needs `pos`, so keep it after; `urgent` is off by default (`GENOS_LIGHTING_URGENT`, `lighting_gpu.rs` 1370). Saves about 67 MB of reads and 3M ray rebuilds per frame, roughly 0.05-0.15 ms.
2. **H2: compose feeds FXAA directly.** The frame does these copies:
   - colour image → `lighting.out` (`lighting_gpu.rs` 1382)
   - `lighting.out` → colour image (1592)
   - colour image → `aa_src` (`aa_gpu.rs` 496)
   - `aa_dst` → colour image (514)

   Let FXAA read `lighting.out` when lighting ran. That drops two full-frame copies (2 x 8 MiB of traffic each way at 1080p), roughly 0.05-0.1 ms.
3. **H3: barriers.** Every dispatch is followed by a full compute→compute barrier (`lighting_gpu.rs` 1585). The probe-ray trace and the dense cache-ray trace (1449-1450) write disjoint ranges, and so do the two direct dispatches (1451-1452). Drop the barrier between each pair, so they overlap on the GPU. This is small and only measurable as an A/B.
4. **H4: compose LastUsed writes.** `lighting_world_irradiance_in` writes `cache.v[3*slot].w` and `WORLD_AT.z` for each of the 4 taps of every pixel (2418, 2435), so about 16M scattered partial writes a frame land on the same few lines. Write only when the stored frame differs (read, compare, then write). Measure; it may be nothing.

Each H step is one commit, measured, and reverted if it gives no measurable gain.

## Implementation order (cheaper-model friendly)

Each line is one commit that builds (`cargo build --workspace`) and keeps the workspace warning-free. Each is measured with the README protocol before the next one starts, and each gets an experiment file in `experiments/` with numbers and screenshots.

1. 0.1 profile (doc only) → 0.2 counters (measurement build) → 0.3 divergence test (measurement build, never pushed on).
2. Gate: if 0.3 confirms divergence, do A1 → A6. Otherwise skip to B, and put A after C.
3. B1 → B2 → B4 → B3 → B5.
4. D1 → D2 → D3 (→ D4 if the profile asks).
5. C1 → C2 (→ C3).
6. H1 → H2 → H3 → H4 (independent; any time).
7. E0 decision → E1 → E2.
8. F1 → F2 → F3 → F4.
9. G1 → G2 → G3 (→ G4), together with the first glTF-content benchmark.

## What not to retry (already measured)

- List-driven indirect a-trous (S78A).
- A single TDL owner per texel (S78M), kept hits remembering their texel (S78O), and a noise-aware TDL check (S78L): each was cheap, and none gained. If A confirms divergence, re-test S78O and S78M after A, because divergence would explain their null results.
- Per-pixel shadow reuse with check and escalate (S77B: torn edges), and pixel reuse by segment test (S74B, S78N). Item E is per lamp and exact instead.
- ReSTIR DI for pixels (S78Q, shelved). It stays the many-lamp answer (the 100-lamp room) if E's f is low there.
- Graded lamp change (S78I).

## System-file note

This plan changes no goal, intent, game use or limit of the lighting system. The README's "Deviations" list gains entries only when a step lands:

- B: "hit shadow visibility tolerates 12.5 cm of lamp travel, with a 1-in-16 verify"
- C: "WRC-cell direct light for hits with no fine texel"
- D: "dense a-trous under heavy change"
- F: "probes dormant where no pixel reads them"
