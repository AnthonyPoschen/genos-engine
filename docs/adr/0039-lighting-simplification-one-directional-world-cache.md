# 0039: Lighting simplification: one directional world cache, texels as detail, retire screen probes

Status: **Proposed** (5 Nov 2026). No renderer code changes until Anthony accepts it.

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

Each stage is its own PR, behind a setting, A/B against the stage before it. 4070 ms is reported for every stage. Nothing is deleted until its replacement passes. The direct-light track (D1, D2) runs in parallel with the bounce track.

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

## Open questions for Anthony

1. **SH-L1 precision:** RGB fp16 (~24 MB at 1M slots, coloured bounce by direction) or luminance-only direction on the existing RGB (~12 MB, colour doesn't vary with direction)? Proposal: RGB fp16.
2. **Low:** should Low get a coarse cell layer and lose its probes too, or keep probe-only GI until there are real 1060 numbers? Proposal: keep probes on Low until measured.
3. **Movers:** if cells can't follow movers fast enough, is a mover-only probe helper acceptable, or must movers be solved in the cells (e.g. object-local cells)?
4. **Order:** stage 1 (ADR 0037) blocks everything. Do you want stage 2 prototyped in parallel on the box (gated the same way) to de-risk it, or strictly in order?
5. **Budget:** do this ADR's 12–48 MB count against the 200 MB soft budget now, or only after ADR 0036 and reservoir trimming land?
