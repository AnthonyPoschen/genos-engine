# 0039: Lighting simplification: one directional world cache, texels as detail, retire screen probes

Status: **Accepted** (approved by Anthony Poschen, 5 Nov 2026).

Builds on ADR 0012 (GI), ADR 0013 (local invalidation, ~200 MB lighting VRAM), ADR 0036 (cache sized per preset), ADR 0037 (cache convergence, linger and lag; PR #147) and the ReSTIR + world-cell light lists proposal (PR #164). Evidence: `/home/box/shared/ge-notes/lighting-challenge-5nov.md`, checked against master 067aa856.

All numbers are measured on the 4070 at 2560x1440 unless marked **est**.

## Context

Anthony's question (5 Nov): the layers seem to repeat the same work, and the coarse world layer looks too blotchy to be a useful fallback. Reading the code confirms it.

### Today: three independent estimates of the same bounce light

| Layer | What it is | Rays | What it keeps |
|---|---|---|---|
| **25 cm cells** ("WRC") | Entries in the hashed surface cache (`lighting_cache.glsl`), keyed by cell × one of 6 axis normals | 16 hemisphere rays per relight | One cosine-weighted RGB irradiance. Each ray's direction, radiance and distance is thrown away. |
| **12.5 cm texels** | The same hash table at half the cell size; what pixels show | Their own 16-ray relights from nearly the same points | Same single value per axis. Surfaces not axis-aligned (normal component < 0.999) get no texels. |
| **Screen probes** | One per 16 px tile (Low: 32 px), 64 rays (Low: 16) | A third set of rays every frame | Irradiance SH per probe, 3 × 3 filtered |

- Ray hits read **one** 25 cm cell with no interpolation, and only the axis copy nearest the hit normal.
- New texels are seeded from a single number (cell bilinear, parent or probe), then trace a full set of their own.
- Probes cover what the cache can't: slanted surfaces, movers, seeds for new texels, and Low (which has no world cache at all).
- Direct light is per pixel with shadow rays. With GI off it costs 4.3 ms. With GI on the same pass costs 13.4–14.3 ms, because pixels trace ~4.8 lamp shadow rays each (17.66M a frame). That ~10 ms is unexplained and looks like a bug, not the price of per-pixel direct.

### Why the cache-only picture looks blotchy (measured)

1. **Too dark.** Cells reach only 0.70–0.84 of reference energy (ADR 0037 patch probe): the settled freeze, young-only rounds and a capped 128-sample mean that keeps dark early samples.
2. **Coarse.** Nearest-cell reads with the normal snapped to 6 axes. Converged, a 25 cm cache scores blotch 0.021 against 0.014 at 12.5 cm (E11).
3. **Noisy.** Young cells (fewer than 16 sets) are already used as seeds.

### Measured lighting cost today (4070, 1440p stress, ms)

| Bucket | ms |
|---|---|
| Lighting total, GI on | 33.6–37.3 |
| Light pass (pixel direct + shadows + kept-hit light) | 13.4 |
| À-trous | 9.0 |
| Compose | 3.9 |
| Probe trace | 3.1 |
| Other GI (cache rays, gather, filter, compact) | ~4 **est** |

## Decision

Go from three bounce estimates to **one**, plus per-pixel direct light.

1. **The 25 cm cells are the single source of truth for bounce light.** Each cell stores incoming light by direction as **SH-L1**, accumulated from the rays it already traces (no new rays). One cell then serves any surface angle, so the six axis copies collapse to one entry per cell. Reads blend across neighbouring cells (trilinear) and evaluate the SH at the surface's **true normal**. Stored light still never depends on the camera (ADR 0012).
2. **Texels become a detail layer, not a second estimate.** A new texel starts from the cell SH at its own position and normal. It traces its own rays only where it needs more than the cell gives: residual disagreement with the cell, contacts and corners, and walls thinner than a cell. In open areas most texel rays go away.
3. **Screen probes are retired** once directional cells cover their jobs: slanted surfaces, seeds and movers. If movers can't be covered (cells don't relight fast enough around a moving object), probes shrink to a **mover-only helper** instead of being deleted.
4. **Direct light stays per pixel.** First the GI-on lamp-ray blowup is fixed. Then lamps are picked by ReSTIR with world-cell light lists (ReGIR), as proposed in PR #164. Caching direct light was tried three times (S72E, S77B, S74B) and failed; Anthony's t378 rule stands: the cache feeds bounce rays only, the screen stays per pixel.

### One-way data flow (hard rule)

**Cells → texels → pixels. Never back.**

- Cell rays read cells (that is how later bounces arrive) but never texels or probes.
- Texels may read cells; cells never read texels.
- While probes still exist, they may seed texels but never feed cells.
- A seed is **low-confidence**: it never counts as a full sample in the capped mean, and the lamp-change event clear drops it.

Reason: ADR 0037 round 13 found old light chained across five histories feeding each other, and no tuning passed the linger and lag gates. A loop between layers re-creates that.

## Stages, each gated on the path-traced reference first

**Validation rules:**
- Accuracy against the path-traced reference is the primary gate. No stage may be less accurate than the stage before it, on any reference scene.
- Validation is headless on lavapipe only (no GPU access assumed) until a stage passes its accuracy gates. Only then is it benchmarked on the 4070.

Each stage is its own PR, behind a setting, A/B against the stage before it. 4070 ms is reported for every stage once its accuracy gates pass. Nothing is deleted until its replacement passes. The direct-light track (D1, D2) runs in parallel with the bounce track.

| # | Stage | Gates (all on the oracle unless stated) |
|---|---|---|
| 0 | **References in place**: camera-turn, slanted-surface (turned boxes, ramps), thin-wall, many-movers (S4), linger and lag scenes, plus a cache-only blotch view | Scenes and metrics run on the box and the 4070 before stage 1 code |
| 1 | **ADR 0037 convergence fix** (no settled freeze on stale cells, no young-only starvation, mean that forgets dark early samples) | Cell energy **≥ 0.9** of reference; linger ≤ 5 % at 120 frames; lag ≥ 90 % at 300 frames |
| 2 | **SH-L1 cells**, blended reads at true normal | Blotch **≤ 0.014** (cache-only view, i.e. 25 cm cells reach today's 12.5 cm score); turn error **≤ master 0.403**; slanted-surface error ≤ probes-on today; energy still ≥ 0.9; VRAM within the estimate below |
| 3 | **Texels seeded from cell SH** | Fill time shorter than today; linger and lag gates still pass; no energy drop; no new pops |
| 4 | **Texel ray cut A/B**: skip a texel's rays where it agrees with the cell SH | Contact, corner and thin-wall errors no worse than no-skip; linger and lag still pass; ms saving measured |
| 5 | **Probes off A/B**, then delete | Turn, spin, approach, A→B→A, many-movers, slanted and linger/lag all ≤ probes-on. Pass: delete probes in a separate PR. Fail on movers only: shrink to a mover-only helper and re-run |
| D1 | **GI-on lamp-ray blowup** | Lamp rays per pixel GI on vs off explained; light pass with GI on **≤ 5 ms**; near and turn error unchanged |
| D2 | **ReSTIR + ReGIR cells** (PR #164) | As PR #164: turn error ≤ 0.403 (ReSTIR-off), replaces the old pass |

Stage 1 comes first because richer data on a cache that is 20–30 % dark still looks bad.

## VRAM (soft budget ~200 MB for lighting)

| Item | Estimate |
|---|---|
| SH-L1 per cell, RGB fp32 (12 floats, 48 B) | **est** 48 MB at 1M slots, 24 MB at 512K |
| SH-L1 RGB fp16, or luminance-only L1 direction on top of the existing RGB (3 values) | **est** 12–24 MB at 1M slots |
| Axis copies removed (6 per cell → 1) | **est** saving, not yet sized: depends on how many axis copies are live per cell |
| Screen probes removed (SH, kept strata, ray buffers) | Not yet measured; not counted as a saving |
| Today's cache table | 368 MiB at 1M slots (ADR 0036); already over budget without ADR 0036's per-preset sizing |
| Today's ReSTIR reservoirs at 1440p | **est** ~236 MB (PR #164); over budget on its own |

So SH costs **est 12–48 MB**. It fits only once ADR 0036's per-preset sizing lands and the reservoir memory from PR #164 is trimmed. This ADR doesn't fix the budget on its own; it must not make it worse beyond those figures.

## Cost estimates

All **est**, to be replaced by stage measurements.

### 4070, High, 1440p

| Change | Effect |
|---|---|
| SH accumulation in cell relights | +0.1–0.3 ms (math on rays already traced) |
| Blended 8-cell reads at hits and seeds (from 1) | +0.3–0.8 ms (more hash lookups) |
| Texel ray cut | −1 to −2 ms in open areas, little near contacts |
| Probes retired (trace 3.1 ms + gather/filter share) | −3 to −4 ms |
| D1 lamp-ray fix | −8 ms (13.4 → ≤ 5 ms gate) |
| Net lighting total | ~20–25 ms, from 33.6–37.3 ms |

À-trous (9 ms) may shrink with a smoother input, but that is not assumed.

### Low on a GTX 1060

- No 1060 numbers exist; every figure here is scaled from the 4070 at roughly 4–5× slower (fp32 and bandwidth), which is only a rough guide.
- Low today is **probe-only GI** (16 rays, 32 px tiles, 1 bounce, no world cache, max 4 lamps). Retiring probes on Low means Low needs cells, which it doesn't have today.
- A Low cell layer (coarser cells, e.g. 50 cm, luminance-only L1, small slot count under ADR 0036) is **est** 4–10 MB and 1–3 ms on a 1060. Today's Low probes are **est** 2–4 ms on a 1060. Roughly a wash; it has to be measured.
- Until it is, Low keeps its probes (see open question 2).

## Risks

- **Contact and corner darkening lost at 25 cm.** The texel residual exists for this. If the ray-cut rule is too keen, corners go flat. Stage 4 gates it.
- **Thin-wall leaks.** Blended reads across a wall thinner than a cell bleed light through it, which is already an open leak. Blending must reject cells on the far side of the surface (normal and visibility weighting); thin-wall is a gate in stages 2 and 4.
- **SH-L1 ringing.** L1 is smooth and can go negative on high-contrast light. Reads clamp to ≥ 0; strong directional bounce near one lamp may look softer than the reference.
- **Feedback loop returns.** Any shortcut that lets texels or probes feed cells re-creates ADR 0037's linger. The one-way rule is checked in review and by the linger gate.
- **Movers.** Cells may not relight fast enough around a moving object; that is why the fallback is a mover-only probe helper, not deletion.
- **Budget.** VRAM is already over the soft budget without ADR 0036 and reservoir trimming.
- **Low.** The 1060 cost of cells versus probes is unknown.
- **Stage 1 is unsolved.** ADR 0037 has not passed its linger and lag gates in 18 rounds. Everything after stage 1 waits on it.

## Rejected

- **Store each ray's hit (ReSTIR GI style)** in cells: S72B was reverted for correlated low-frequency blotches, and it needs visibility checks.
- **Direction bins** (e.g. 8 octahedral): more memory than SH-L1 for detail the 25 cm cell can't use.
- **Neural radiance cache:** rejected in ADR 0022.
- **Caching direct light in texels:** blurs shadows to 12.5 cm, breaks specular, and lags on every moving lamp (S72E, S77B, S74B).
- **Keep three layers and tune them:** each tuning round so far has traded linger for lag; the duplication is the problem.

## Consequences

- One place owns bounce light, so lamp changes, invalidation (ADR 0013) and convergence (ADR 0037) are fixed in one layer instead of three.
- Slanted surfaces get cache light, not only probe light.
- `docs/systems/gi-design.md` and the lighting README are updated when each stage lands, not before.
- The probe passes (place, trace, gather, filter) leave the frame graph at stage 5, or shrink to movers.

## Decisions on the open questions (Anthony, 5 Nov 2026)

1. **SH precision:** colour (RGB) SH-L1 in fp16, ~24 MB **est** at 1M slots.
2. **Low:** keeps its screen probes until real GTX 1060 numbers exist. Retiring probes applies to Medium and above first.
3. **Movers:** a mover-only probe helper is acceptable if cells can't keep up with moving objects.
4. **Order:** stage 2 may be prototyped in parallel on the box, but merges only after stage 1 passes its gates.
5. **Budget:** SH memory counts against the ~200 MB lighting soft budget once ADR 0036's per-preset sizing and the reservoir trimming have landed.

## Addendum A (5 Nov 2026): stages 2b and 2c, cell layout, lighting memory

Status: **Proposed**. Adds two stages between stage 2 and stage 3, a cell-layout choice, and a lighting-memory setting. Research and sources: `/home/box/shared/ge-notes/cache-ray-feedback-research.md`. The CPU reference is `crates/cache-ref`. Measured numbers live in that crate's report and the PR, not here. Every byte count below is **est** until the crate or a GPU stage measures it.

### Principles

- **One-way stays.** Cells are built from their own rays only; texels and pixels only read cells. Letting texel rays feed cells ("lend rays") was studied and is **parked**: it is not part of any stage.
- **Memory is the constraint, ray and lookup ALU are not.** Prefer compact encodings (shared exponents, quantised gradients, octahedral or relative packing) even when decode costs more. Lookup cost is reported as counters (points fetched, ALU ops per lookup) and never disqualifies a variant on its own.
- **Accuracy against the path-traced reference stays the primary gate.** No stage may be less accurate than the stage before it on any reference scene (as above).

### Stage 2b: in-cell position data

Each cell also records **where** its own rays start and how far they go, so a read can correct the cell value for the reader's position:

| Field | What | Compact size (**est**, RGB engine) | How a read uses it |
|---|---|---|---|
| Ray-start centroid μ | mean origin of the cell's rays, offset from the cell point | 3 × u8 = 3 B | Anchor for the gradient (the value is placed where it was measured, not at the cell point) |
| Clamped L0 gradient ∇ | least-squares slope of the L0 term over ray starts | 3 × i8 relative to L0 (luminance gradient, applied to all channels) = 3 B | `E(p) = E + ∇·(p − μ)`, clamped so it cannot go negative and to `L0 / max(R, h/2)` (Ward/Heckbert gradient limit, Křivánek clamping) |
| Mean hit distance R | harmonic mean of the cell's ray lengths | 1 B (log-encoded) | Gradient clamp; later a validity weight |
| Axis key (replaces the side bit) | dominant normal axis and its sign (6 classes, as today's cache), hashed into the key | 0 B per cell (a cell touched from several classes uses one entry per class) | A read only blends entries of its own class. The 1-bit side key of the first draft let a wall face and the floor beside it share a cell; on the CPU reference it raised the thin-wall leak instead of lowering it |
| Irradiance scale k | running mean of the cell's measured cosine irradiance over its SH-L1 irradiance at the mean normal, clamped to [¼, 4] | 1 B (log-encoded) | Reads multiply by k. SH-L1 under-reads light that arrives from one hemisphere only (E = ¼∫L + ½∫L cos θ instead of ∫L cos θ), so plain SH-L1 cells lose 17–33 % of the energy on the reference scenes |
| Gradient variance | running variance of the per-frame gradient estimate | 1 B (log-encoded) | Wiener shrink of the gradient, `s = |∇|² / (|∇|² + var / n)`, so a noisy young gradient does not raise pop |

With SH-L1 in a shared-exponent layout (L0 RGB as RGB9E5, L1 as i8 relative to L0, **est** 13 B), a 2b cell is **est ~22 B** (13 + 7 position + 2 for k and the gradient variance) against 24 B for plain fp16 SH-L1; with a 12 B key, count and age, **est 34 B** against 36 B per entry. The accumulation side (this frame's sums) is transient and not in that figure.

**Read visibility (optional, counted not assumed).** A read may drop a neighbour cell when a short ray from the reader to that cell's centroid is blocked (the stage 4 "reject cells on the far side" rule, applied early). It costs one short occlusion ray per neighbour read (8 cubic, 4 BCC). On the CPU reference it halves the thin-wall leak but does not remove it: lattice points that sit inside a 5 cm wall own surface on both sides. Subdivision (2c) or stage 4 has to finish that.

**Departure in the CPU reference.** After a lamp change the reference holds the history cap at the clear value for 24 frames (`recover_frames`), so stale bounces flush. This stands in for ADR 0037's convergence fix; without it linger at +120 is ~8 %.

**Gates (CPU reference first, then lavapipe):** on every reference scene, against plain SH-L1 at the same layout and memory:
- blotch lower;
- leak no higher (thin-wall dark face);
- energy ≥ 0.9 of reference and within 1 % of plain SH-L1's energy (see the reading note below);
- linger ≤ 5 % at 120 frames and lag ≥ 90 % at 300 frames (as stage 1);
- pop (99th-percentile frame-to-frame change in steady state) no worse than plain;
- no negative irradiance anywhere.

Reading note (5 Nov, after the first CPU run): plain SH-L1 is itself 17–33 % low, so "within 1 % of plain" and "leak no higher than plain" compare against a biased baseline. The gates are kept as written and reported literally; the PR also reports each config's leak in excess of its own energy bias. Accuracy against the reference stays the primary gate.

**Must-fail variants (negative controls).** If either passes, the harness is too weak and every other result is void:
- F3, texel feedback: texels push their own error back into the cell. Must fail linger at +120 frames (> 5 %).
- F2, skewed origins without 1/q weights: choosing ray origins in proportion to a hardness score (skew 8 and 64) and averaging them unweighted. Must show a hard-spot bias beyond 3 seed standard deviations of uniform selection, and the 1/q-weighted control must remove it.

### Stage 2c: sum-based subdivision with hysteresis

Cells split into 8 children near the camera (projected size) and where their own data says light varies across the cell (`|∇|·h / L0` above a threshold, with enough samples). State is kept as **sums and counts**, so a merge is exact addition. A split seeds each child from the parent's 2b model evaluated at the child (centroid + gradient), as a **low-confidence seed** (never more than a few samples' weight; lamp-change clear drops it first, as the seed rule above). Children trace extra rays only while below a minimum sample count. Readers cross-fade parent → child by the child's sample count, so a split does not pop.

Hysteresis: split and merge thresholds with a gap (distance band and data band), plus a minimum dwell in frames after any change.

**Gates:** blotch close to static 12.5 cm cells (within 10 %) at lower memory; pop at the split frame ≤ 1 steady-state σ; zero split/merge toggles over 600 frames of a camera dolly across the threshold; with a deliberately skewed origin distribution, seeded children's energy within 1 % of reference behaviour; fewer catch-up rays than zero-start children; linger and lag unchanged.

### Cell layout: cubic or body-centred cubic (BCC)

The BCC lattice is the 3D analogue of the triangular lattice. For volumetric signals it needs 29.3 % fewer samples than a cubic lattice for the same reconstruction (Theußl, Möller, Gröller, *Optimal regular volume sampling*, IEEE Visualization 2001: the same accuracy as a Cartesian grid with 29.3 % fewer samples for isotropic band-limited volumes; Entezari, Van De Ville, Möller, *Practical box splines for reconstruction on the body centered cubic lattice*, TVCG 2008: reconstruction filters for BCC). Our signal lives on surfaces, not in a volume, so the gain has to be measured. The crate compares both layouts **at equal memory** and **at equal accuracy** (memory each needs to reach the same blotch, leak and energy). BCC reads are linear interpolation over the 4 points of the BCC tetrahedron containing the reader (fewer fetches than trilinear's 8, more ALU); subdivision of a BCC point is a BCC lattice at half spacing (each point's Voronoi cell is a truncated octahedron, which does not nest, so children are assigned by nearest point rather than exact containment). On a surface the gain need not carry over: at equal volume density a plane cuts more BCC Voronoi cells than cubic ones, so BCC can need more entries for the same surface (first CPU sweep, contact room: 31–38 % more). The comparison therefore uses entry counts measured on the scenes, not lattice density. The layout is decided by the crate's numbers before stage 2c starts on the GPU.

### Lighting memory setting

The fixed cell pool becomes its own **lighting memory** value, separate from texture and model quality, so a player on a 3, 4 or 6 GB card can trade lighting quality against model quality. It follows `docs/engine/settings-principles.md` (a key on `LightingSettings`, on the MCP catalogue and in the debug menu) and replaces ADR 0036's fixed per-preset slot count with a byte budget the presets default (placeholders: ~150 MB Low to ~500 MB High). The defaults are picked from the crate's accuracy-vs-memory sweep (64, 128, 256 and 512 MB equivalents), then confirmed by ADR 0036's measured occupancy on the GPU.

### Tuning order and low-spec knobs

Tune on the 4070 first, but design the low-spec knobs in now: **pool size (lighting memory), cell size and ray counts are parameters, never hard-coded** (settings values per `docs/engine/settings-principles.md`, presets only set defaults), so Low presets for 3–6 GB cards can be tuned later without a redesign. `crates/cache-ref` takes the same three as parameters.
