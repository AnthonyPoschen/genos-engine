# ADR 0039 cell gaps

Status: **design for review**. No runtime change. Anthony reviews this before any stage is implemented.

Accepted direction: [ADR 0039](../../adr/0039-lighting-simplification-one-directional-world-cache.md). One colour SH-L1 world cache is the bounce source of truth. Texels are a detail layer. Data flows cells → texels → pixels, never back. This note designs the three gaps [adr-0039-remaining.md](adr-0039-remaining.md) left open after the neighbour read:

1. Each world cell is stored as up to six dominant-axis copies.
2. A seam between those keys when the dominant axis flips.
3. A same-facing surface closer than half a cell leaks. The plane test does not reject it.

The thin-wall reject uses the six copies: the other side is a different entry, and a stored normal that faces away is left out. Collapsing the copies without a replacement split puts that leak back. The three gaps are one layout.

## Recommendation

Pack every live face of one spatial cell into a side pool (a **face pack**). The world-cell hash keeps today's half-cell inset, so a wall on a grid plane stays in one cell, and it loses the axis salt (`lighting_axis` no longer goes into the hash). Each face keeps the hemisphere rays, the exact cosine mean, and the fp16 SH-L1 it already stores. A query gathers each dominant axis whose dot with the normal is positive (one lattice on a flat wall, two on the cone, three on a corner) and weights that face by `max(dot, 0)`. At a pure axis the other dots are zero, so the value is that face's mean and the gather is today's eight corners. On the cone the adjacent faces both contribute. The opposite face stays a different record and drops out.

Same-facing surfaces inside that face are **cell residents**, split by plane offset (High default two residents, 5 cm sheet). The read picks the closest resident inside half a sheet. It does not blend two residents. A full face refuses the new surface and the query misses, which keeps the probes and the texel rays. It does not show the other surface.

A body-centred lattice is a later A/B against this cubic pack. It is not the store. Plain L1, a full-sphere trace from a surface point, a smoothstep across the old keys, a tighter plane test alone, and camera-distance cell bands are not the fix.

High pool estimate: 262144 records × 48 B ≈ 12 MB, inside ADR 0039's 12–48 MB SH envelope. The 392 MiB slot table does not grow. Low keeps probes, a zero pool, one resident, and a 50 cm cell.

## What the code does now

World mode 3. `LIGHTING_SH_CELL` is `LIGHTING_SC_HIT`, the literal 0.25 m (`lighting_cache.glsl`). `world_cache.cell_metres` is a public setting, clamped to 0.05–4 m, and world mode 3 does not read it: `lighting_cell_size_d` returns `LIGHTING_SC_TEXEL` (0.125 m) for every distance, and `lighting_sh_gather` uses the 0.25 m constant. S57 removed distance bands because a band edge re-keyed entries and restarted them.

`lighting_axis` maps a normal to 0..5: the largest `|component|`, then the sign. Ties prefer X, then Y, then Z.

`lighting_cache_key_sized` builds the world-cell key from three things:

- an inset of half a cell along that axis, so a surface on a grid plane lands in one cell;
- `floor(pos / size - inset)`;
- a hash of that cell, the axis, and the size.

Two orientations at one place are two hash keys. Each key is a full cache entry: position, normal, mean, estimator, sun share, and 24 B of fp16 SH-L1 (`lighting_sh_put`, `LIGHTING_SH_HALF`). The table is 1,048,576 slots × 392 B = 392 MiB (368 B entry + 24 B SH, ADR 0036). Six copies do not multiply that allocation. They multiply live entries. A thin wall is two entries. A corner is up to three. Six is the cap, not the average. ADR 0036 records no occupancy split by axis.

A 25 cm relight (`lighting_cache.comp`, the `wrc_cell` arm) traces `LIGHTING_CACHE_RAYS` (16) cosine-hemisphere rays about the stored normal. The mean is the cosine-weighted irradiance. The SH sums the same rays, unweighted by cosine: `L` and `L * dir`. The harmonics never see the back hemisphere.

`lighting_sh_gather` looks up eight corners on **one** axis, the query's dominant axis. The tilt (`ShL1::tilt`, `lighting_sh_tilt`) is the stored mean plus the band-1 change from that axis to the true normal. At the axis it is the mean. `plain_l1_loses_a_quarter_of_a_narrow_source_so_the_read_keeps_the_mean` shows why: plain L1 irradiance of a narrow source straight down the normal is 0.75 of the cosine mean. The read clamps at zero. A corner is left out when it is missing, empty, stale, back-facing (`dot <= 0`), or more than half a cell off the query plane (`plane > LIGHTING_CELL_HALF * cs`, half of 0.25 m = 0.125 m). Left-out corners add no weight. `lighting_sh.rs` `neighbour_corner_weight` is the CPU twin. The rejected four-cell lattice stays off. An axis-aligned ray hit still tilts the one owning cell.

`lighting_sc_wrc_now` is the detail-off bilinear read. It has the same half-cell plane test and the same axis key.

The one-way rule is already in the read: seed, coverage, and slants pass `claim` false. `lighting_sh_gather` does not call `lighting_sh_put`. Texel detail and the ray cut do not write the cell.

### Gap 1, six copies

ADR 0039 decision 1 is one entry per cell, SH-L1 serving any angle. The key still hashes `lighting_axis`. The thin-wall test in `texel_detail_prod` depends on that: the other side resolves a different key, and the facing test drops a stored normal that faces away. `lighting_axis` gives the boundary to the earlier axis, so the normals stored under one axis sit inside an open cone and their facing dot stays positive. The facing test does not separate same-axis surfaces.

### Gap 2, the seam

The seam is the cone where the largest component changes. `lighting_axis` is a step. `lighting_sh_gather` then rebuilds `an` from that step and looks up only that family. The inset flips with it, so the spatial index flips in the same step.

Example, cell 0.25 m. A normal a hair off `normalize(0, 1, 1)` moves the inset from +Y to +Z. `floor(pos / size - inset)` changes family. The eight corners of the new family are not the eight corners of the old one, and the axis salt means they are not the same entries. A bevel, a cylinder, or a sphere steps from one converged mean to another. A normal that jitters across the cone pops. There is no weight on the second axis.

### Gap 3, the same-facing leak

The plane test rejects a stored point more than 0.125 m off the query plane. It has two holes.

**Same slot.** Inset for +Z is `(0, 0, 0.5)`. The index is `floor(z / 0.25 - 0.5)`. `z = 0` and `z = 0.10` both floor to `-1`. One entry, one mean, one stored point. The plane test never sees a second candidate. The front surface shows the back surface's light.

**Neighbour inside the threshold.** The +Z split between index `-1` and `0` is `z = 0.125`. Planes at `0.124` and `0.126` are different entries, 2 mm apart, and `0.002 < 0.125`, so both pass and the blend mixes them. `lighting_sc_wrc_now` does the same.

Both normals face the same way, so `dot > 0`. Tightening the constant does not split a shared slot.

Texels are the ADR's detail layer for contacts, corners, and walls thinner than a cell. `lighting_compact_hold_texel_rays` skips a fine texel's rays when the live gather agrees with itself. A leaked cell that is smooth agrees with itself, so the cut holds the wrong light. Bounce rays read the cell. The one-way rule stops the texel from repairing that cell. The split has to be in the cell.

## Constraints

From ADR 0039, ADR 0012, ADR 0036, S57, and this review:

- Accuracy is against the path-traced reference. A stage is not allowed to be less accurate than the stage before it on any reference scene.
- No estimator whose only job is to hide a step. The weight has to be the reconstruction ADR 0039 already chose (exact mean at the face axis, SH tilt off that axis).
- No pop and no flicker. A normal jittering across 45° stays continuous. A claim does not depend on camera order for its value once both surfaces exist.
- Stored light does not depend on the camera (ADR 0012). The camera may decide residency and update order. It does not decide cell size, sheet, or which lattice is read.
- Near-camera error matters most. The oracle weight is `ref_err_near` (`1 / max(distance, 0.5 m)`).
- Lighting VRAM soft budget is about 200 MB. SH was estimated at 12–48 MB and must stay inside that envelope. The 392 MiB table is ADR 0036's problem; this note does not grow it.
- Memory may be spent to save rays or to keep a second surface. Lookup cost is recorded and does not reject an option.
- Body-centred layouts and tight packing are candidates.
- Pool size, cell size, resident count, sheet, and ray count are settings (public value, MCP through the existing lighting commands, debug menu). Low is the coarse end. Low keeps screen probes until a GTX 1060 measurement exists (ADR 0039 open question 2).
- Rays stay in the hemisphere of the face they light, from that resident's point. A full-sphere trace from a surface point shoots through the surface.
- The rejected four-cell lattice stays off. Direction bins stay rejected (ADR 0039). Radiance cascades stay removed (ADR 0012).

## Gap 1 — six dominant-axis copies

The job: one spatial entry that still answers every face, still exact at an axis-aligned normal, still separate on the two sides of a thin wall.

### Option A1. Drop the axis. One slot, one hemisphere, tilt the rest

The first claimer owns the only hemisphere. Other faces extrapolate with tilt.

| | |
|---|---|
| Memory | Fewer live slots where axes used to collide. Allocation stays 392 MiB until the pool knob shrinks the table. SH stays 24 B on the slot. |
| Compute | One 16-ray relight instead of one per live axis. The gather stays eight lookups. |
| Accuracy | Fails the axis mean for every face that did not claim. A floor and a wall that share the quantized cell extrapolate each other. Opposite faces share one mean. The facing test can turn the unclaimed side into a miss (probes) rather than a blended leak. A miss is not the reference. |
| Failure | Who claimed first depends on what the camera saw. That is the claim-order bug flag 16384 fixed for the lit-from point. Here it would choose the hemisphere. ADR 0012. |

Rejected.

### Option A2. One slot, full-sphere rays, plain L1 irradiance

Trace the sphere into one SH-L1. Read `ShL1::irradiance` at the true normal. Drop the tilt.

| | |
|---|---|
| Memory | One slot, 24 B SH. Occupancy falls. Allocation unchanged. |
| Compute | 16 rays over 4π is half today's hemisphere density, or 32 rays if the density stays. The ray count is a setting either way. Eight lookups. |
| Accuracy | A uniform sky on one hemisphere is a special case L1 reconstructs exactly: irradiance `π (1 + cos θ) / 2`, so the lit side is `π` and the back is `0`. A narrow lamp is the case the CPU test guards: plain L1 returns 0.75 of the cosine mean at the normal. The energy gate is 0.9 of reference. Lamp bounce fails it. Off-axis, the L1 lobe is a low-pass, so a bright side tints angles that are not the exact back. The zero clamp hides the negative ring and does not put the energy back. |
| Failure | The rays start on a surface. The back hemisphere runs through the wall into the other room. That is a new leak, built into the sample domain. DDGI can trace a sphere because the probe sits in free space. These cells do not. |

Rejected. The 0.75 result is why the tilt exists. The sample domain is why a surface cell stays a hemisphere.

### Option A3. Face pack in a side pool (tight packing)

The hash key for a world cell stays `floor(pos / size - inset)` with `inset = 0.5 * cell` along the dominant axis of the geometric normal, plus the size. That inset is what keeps a grid-plane wall in one cell. The axis id is not hashed. Two faces that fall in one quantized cell are two records on that cell, not two fat slots.

A 5 cm wall can already land both sides in one index once the salt is gone (front `z = 0.13` insets to 0, back `z = 0.08` insets to 0). The face record is what keeps those sides apart. Removing the salt without the pack would merge them.

The fat hash slot stays the header so compact and the relight list keep walking slots. The header stores the pool indices of its faces (at most six). It is not a stand-in mean. `lighting_sc_wrc_now` on the pack-on path loads the face whose axis is `lighting_axis(n)` and reads that face's mean. Detail-off bilinear stays per axis. One writer updates the face mean and the SH together, including the sun/sky rescale (`sh_ls`).

A face record is the exact cosine mean (fp32, 12 B, same range as `cache.v[3 * slot + 2]`), the sample count (4 B), the plane offset (fp16, 2 B), and the fp16 SH-L1 (24 B). Aligned 48 B. The lit-from point stays `lighting_sc_centre` on that face's plane. Empty faces take no record.

Rays: 16 cosine-hemisphere rays about that face's axis, from that face's point. Same rays as today, no new sample domain. The SH blend alpha stays the mean's alpha, including the settling arm that takes the round whole. The scheduler walks the header's index list and runs the existing relight arm once per face.

Read: for each dominant axis with `dot(axis, n) > 0`, the eight corners of **that axis's inset lattice**. A corner contributes `max(dot(axis, n), 0) * trilinear * that face's tilt` when the face is present. Weights renormalize. A missing face adds nothing and does not darken. At a pure axis every other dot is 0, the other walks are skipped, and the value is that face's mean (`assert_eq!(sh.tilt(e, axis, axis), e)`). On the cone, `normalize(0, 1, 1)` gives +Y and +Z `dot = 0.707`. The tie itself is classified +Y, because `|y| == |z|` takes the `a.y >= a.z` arm. A hair toward +Z classifies +Z. The sweep is that neighbourhood. The opposite face is `dot <= 0` and stays out.

| | |
|---|---|
| Memory | Pool, not a ×6 stripe on the 1M table. Six always-on SH copies would be 6 × 24 B × 1M ≈ 144 MB and would miss the 12–48 MB envelope. A pool of live faces does not. High default 262144 × 48 B ≈ 12 MB **est**. See the knob table. The fat slot stays a header (pool indices only). Extra faces cost 48 B instead of another 392 B slot. |
| Compute | Relight count matches live faces, which is today's live axis copies. A flat wall is eight hash lookups, the same as today. The cone walks a second lattice (16) and a corner a third (24), each probe up to `LIGHTING_CACHE_SEARCH` (16). That cost is listed and does not reject the option. A cell that already holds both faces still needs the second walk when the other face was claimed on the other inset. |
| Accuracy | Axis-aligned surfaces keep today's mean, so the energy gate has the same estimator it has now. Slanted surfaces use the tilt they already use, from the face that actually sampled that hemisphere. Thin walls stay two records. |
| Failure | A seventh orientation in one cell cannot happen: there are six axes. Pool exhaustion refuses the claim. The query misses (probes, texel rays). It does not alias onto another face. A young face (under the existing sample gate, `LIGHTING_CACHE_HAS_W`) stays out of the blend and fades in with the sample-count handover already in `lighting_sc_shown`, so the first samples do not pop the cone from one colour to another. |

This is the recommendation. It is the ADR 0039 store: SH-L1 per sampled hemisphere, faces packed instead of six fat entries, texels still downstream.

### Option A4. Body-centred lattice, one sample, no face pack

A body-centred cubic lattice puts two sites in a cube of edge `A`. The nearest-neighbour distance is `(√3 / 2) A`. Matching today's 25 cm spacing means `A ≈ 0.289 m` and about 83 sites per cubic metre against 64 for the cubic grid, about 1.3× density **est**. The read is four tetrahedral weights instead of eight trilinear corners.

| | |
|---|---|
| Memory | About 1.3× the live cells for the same spacing, or the same count with about 9% coarser spacing. Empty space is not stored: sites are still claimed by surfaces, not a dense volume. A dense volume would blow the budget. |
| Compute | Four lookups instead of eight. Lookup cost is not the reason to pick it. |
| Accuracy | Interpolation isotropy improves. The axis key, the seam, and the same-facing leak are untouched if the key still hashes the axis and the record is still one hemisphere. |
| Failure | Tetrahedra that cross a wall blend the two rooms unless the face test rejects the far vertex. That reject is A3's facing weight, which A4 does not contain on its own. |

Candidate for the spatial layout **on top of A3**, as a gated A/B. Not a substitute for the pack. Default stays the cubic eight-corner gather. The four-cell lattice stays rejected; tetrahedral BCC is a different layout and does not revive it.

### Option A5. Eight octahedral direction bins

ADR 0039 rejected direction bins: more memory than SH-L1 for detail a 25 cm cell cannot resolve. More bins make the seam more frequent. Not revived.

## Gap 2 — the seam between dominant-axis keys

The job: a normal that crosses the cone gets a continuous value, exact at each axis, from samples that belong to those hemispheres.

### Option B1. Read every positive-dot axis and weight by that dot

Keep today's six keys. `lighting_sh_gather` repeats the eight-corner walk for each axis with `dot(axis, n) > 0` (one on a flat wall, two on a cone, three on a corner). Weight `max(dot, 0)`. Drop the rest. At a pure axis the extra walks have weight 0 and the value matches today, including the inset lattice of that axis.

| | |
|---|---|
| Memory | None. The six copies stay. |
| Compute | Up to 24 hash lookups (3 × 8) where three axes meet, each walking `LIGHTING_CACHE_SEARCH` (16). A flat wall stays at 8. Listed, not a veto. |
| Accuracy | Continuous across the cone if both axes have samples. Exact at the axis. The weight is the same facing dot the neighbour read already uses, not a band glued over the step. Each contribution is that face's tilt, which is the ADR reconstruction. |
| Failure | The two copies converge on different frames. A young side is left out (same rule as an empty corner) and fades in by sample count, otherwise the cone shimmers as the two means catch up at different rates. The inset lattices still differ, so this blends two grids. It does not delete the six copies, and gap 3 is untouched. Shipping it and then packing deletes it. |

Useful as a fallback experiment if the pack's key change fails the thin-wall gate. Not the recommended build.

### Option B2. The face-pack read (A3)

Same weights as B1. The axis id is gone from the key, so two faces that share an inset cell share a header and do not drift as two fat entries. Faces that the inset still separates stay on their own cells. The second walk reads those face records. That is B1's walk against the pack.

The inset stays in the key. Dropping it would put a grid-plane wall on a cell boundary, and a millimetre of hit jitter would split its samples. The pure-axis gather is that inset lattice, which is the lattice the claim wrote, so it finds the cell. The cone adds the other axis's inset lattice. Stage 1 plants a wall on a grid plane and walks the normal through the cone in 0.5° steps. A miss on that wall fails the stage.

| | |
|---|---|
| Memory | The A3 pool. |
| Compute | Eight lookups on a flat wall. Sixteen on the cone, twenty-four on a three-axis corner. Same figure as B1. A shared inset cell stores both faces on one header instead of a second fat entry. |
| Accuracy | Exact at the axis, because the other weights are zero and the tilt-at-axis identity holds. Continuous across the cone when both faces have samples. A young face stays out and fades in by sample count. |
| Failure | A read does not allocate (`claim` false stays false). A lattice with no face leaves that weight out and does not darken the other. A miss of every face leaves the probes, as today. |

Recommended. It is the read half of A3, not a second design.

### Option B3. One full-sphere SH, six axis means as anchors

Store one sphere SH plus six exact means. Blend the three nearest means inside the octant. Use the SH only as a warp.

| | |
|---|---|
| Memory | 6 × 8 B of means + 24 B SH = 72 B per occupied cell if the means are fp16 RGB plus a count. Smaller than six fat slots. Larger than one face of A3. |
| Compute | One sphere relight. Ray count is a setting (16 or 32). |
| Accuracy | Exact at the six axes if the blend is a partition of unity that is 1 at an axis. Inside the octant it is another low-pass, similar to L1, and the texel layer is allowed to refine that. |
| Failure | Same sample-domain leak as A2: sphere rays from a surface point enter the other room. The sphere SH and the hemisphere means disagree, so a tilt that adds them double-counts. |

Rejected because of the sample domain.

### Option B4. A smoother step on the old key

A `smoothstep` on `|n_a| - |n_b|` across a band, blending the two existing gathers.

| | |
|---|---|
| Memory | None. |
| Compute | Sixteen lookups inside the band. |
| Accuracy | The weight exists to hide the key switch. It is not the hemisphere integral and not the tilt. The band edges are two new kinks unless the derivative matches, and it will not match two unrelated means. |
| Failure | Normal noise inside the band flickers. The two copies still converge apart, so the blend shimmers. Gap 1 and gap 3 remain. |

Rejected.

## Gap 3 — same-facing surface closer than half a cell

The job: two surfaces that face the same way and sit less than half a cell apart keep two bounce estimates. A single flat wall stays one estimate. The read never averages the two.

### Option C1. Shrink the plane threshold

Replace `0.5 * cell` with a smaller constant, for example 2 cm.

| | |
|---|---|
| Memory | None. |
| Compute | None. |
| Accuracy | Splits the 2 mm neighbour case when the gap exceeds the new constant. Does nothing for the same-slot case (`z = 0` and `z = 0.10` are one entry). |
| Failure | A bevel or a displacement larger than the constant rejects the corner and falls through to the probes: a dark step on a surface that is one surface. The stored point of a flat wall is on the plane (`lighting_sc_centre`), so flat walls survive. The shared slot does not. |

Not the fix. The constant stops being a blend permission once residents exist. Until then it stays `0.5 * cell`.

### Option C2. Put a plane quantum in the hash key

Add `floor(pos[axis] / sheet)` to the key. High sheet 5 cm. A 10 cm gap becomes two entries. The query knows its plane, so the lookup count stays eight.

| | |
|---|---|
| Memory | One extra fat slot per sheet crossing. A bookcase in one column is several full entries, not 48 B records. That fights the packing in A3. |
| Compute | Same eight lookups. |
| Accuracy | Parallel axis-aligned gaps larger than a sheet split correctly. |
| Failure | A ramp's axis coordinate changes by about `cell * sin(tilt)` across the cell. At 45° and 25 cm that is about 18 cm, several 5 cm sheets, so one ramp seams and stores duplicate samples. A key that depends on the shading normal seams a smooth cylinder the same way. Hysteresis on the sheet index sticks, then pops when it lets go. |

Rejected as a hash salt. The sheet belongs inside the cell, as C3.

### Option C3. Residents inside the face (recommended)

Each face holds up to `R` residents. High default `R = 2`. A resident is claimed when a hit's plane offset along the face axis is farther than half the sheet from every resident on that face. High sheet default 0.05 m. The offset is `dot(pos, face axis)` from the geometric normal's dominant axis, so a shading normal or a normal map does not move it. Hits inside the radius attach. There is no temporal hysteresis.

The read, after the face weight, keeps the resident whose offset is closest to the query and within half a sheet. Two residents are never averaged. A query outside every radius misses. Left-out residents add no weight and do not darken the ones that remain.

Relight origin is that resident's `lighting_sc_centre`. Hemisphere about the face axis. The scheduler visits residents with the same per-slot relight it has; a resident is a pool index the existing `lighting_cache.comp` arm can address. No second relight shader.

Ray cut: `lighting_compact_hold_texel_rays` does not hold when the gather sees a second live resident on the query's face inside the gathered cells, even if the colours match. The cell is not a complete description of that texel. One resident and an agreeing field still hold, which is today's cut.

| | |
|---|---|
| Memory | `R` scales the pool, not the 392 MiB table. High `R = 2`, 262144 records, ≈ 12 MB **est** if the pool is the cap. Typical rooms use one resident per face; the second is the leak case. Average occupancy has to be measured (the stage 0 counter). The cap is the budget, not the average. |
| Compute | A few offset compares per corner. Extra rays only for a live extra resident: the rays that surface needs and does not get today. Ray count per resident is the cell-ray setting, default 16. |
| Accuracy | The 10 cm pair and the 2 mm pair each keep a mean. A single flat wall stays one resident because every hit falls in one radius. Bounce rays read the resident, so the leak does not survive into the next bounce. Near the camera a miss still has texel rays and probes. Far from the camera, `R` full and a miss is the coarse answer; `ref_err_near` weights that down. |
| Failure | A third surface with `R = 2` is refused. The picture misses. It does not show resident 1's light on surface 3. A surface that jitters around the radius boundary can open two residents and split samples. The radius is a Voronoi attach, and the offset comes from the geometric plane, so a stable triangle stays put. The fixture for one wall must show a resident count of 1. A moving panel is already a mover: detail-on reads the pack at the current pose, `claim` false, and does not allocate. Object-local texel keys stay salted and are not residents. |

Recommended.

### Option C4. Chebyshev depth moments

One mean depth and one variance per cell. Weight or reject by the Chebyshev bound. DDGI (Majercik et al. 2019) and GIBS depth moments do this. The glossary already marks that test as not built.

| | |
|---|---|
| Memory | 8 B per cell **est** (two fp32), or 4 B fp16. Small. |
| Compute | A few flops per corner. No extra lookup. |
| Accuracy | A reject, not a second irradiance. If both surfaces already share one mean, rejecting the front shows the probes and leaves the stored mean mixed for whoever is accepted. One Gaussian on a bimodal depth is the usual leak. |
| Failure | Used as a multiply, the bound darkens the corner. The rule is that a left-out corner adds no weight and does not darken. Used as a hard reject, the first frames have zero variance and either accept everything or reject everything, which flickers as samples arrive. |

Not the split. A later stage may try it as a soft reject **inside** a resident if the sheet radius still leaks, behind its own gate. It is not on the path below.

### Option C5. A finer cell near the camera

`world_cache.cell_metres` exists and is unwired for this cache. A uniform 12.5 cm cell quarters the leak distance and quarters nothing below 6.25 cm. A cell size that follows the camera is the S57 band edge: re-key, restart, pop. ADR 0012.

| | |
|---|---|
| Memory | Four times the cells at half the edge, if the same area is covered. The pool cap has to move with it or claims miss. |
| Compute | Four times the candidate cells. |
| Accuracy | Shrinks the bug. Does not close it. |
| Failure | Camera-following sizes pop. A uniform finer size is a Low/High knob, not a leak fix. |

Wire the existing setting (stage 4). Do not use it as the design.

### Option C6. A visibility ray from the query to the stored point

One deterministic ray along the segment. A hit in between rejects the cell.

| | |
|---|---|
| Memory | None. |
| Compute | One ray per accepted corner per shaded point, and again at every bounce hit. That is a probe budget spent to detect a case the resident already records. Stochastic rays flicker. |
| Accuracy | Same-slot: the stored point may be the other surface, and the segment through the air hits nothing before the endpoint, so the ray accepts the leak. A ray that rejects the surface which does not own the point turns that surface into a miss. A miss is not its colour. |
| Failure | Cost on the bounce. No memory of the second surface, so the next frame traces the ray again. |

Rejected as the store. The overflow path of C3 is already a miss; it does not need a ray to decide that.

## Knobs

All of these are `LightingSettings` fields, `set_key` names, MCP through `get_lighting_settings` / `set_lighting_settings`, and debug-menu controls. An environment variable applies only when the matching argument was omitted. The argument wins when both are set. Defaults below are High. Preset Low is the coarse column. Medium sits on the High numbers until a measurement says otherwise. Ultra may raise the pool. The soft budget is the ceiling (ADR 0036 sizes the fat table; this pool is the SH envelope).

| Setting | Key | High | Low | Env mirror |
|---|---|---|---|---|
| Face pack | `surface_cache.sh_face_pack` | off until stage 5 | off | `GENOS_LIGHTING_SURFACE_CACHE_SH_FACE_PACK` |
| Spatial cell | `world_cell` (already) | 0.25 m, and the shader reads it | 0.50 m | already `world_cell` |
| Pool records | `surface_cache.cell_pool` | 262144 | 0 (pack stays off) | `GENOS_LIGHTING_SURFACE_CACHE_CELL_POOL` |
| Residents per face | `surface_cache.residents` | 2 | 1 | `GENOS_LIGHTING_SURFACE_CACHE_RESIDENTS` |
| Sheet | `surface_cache.sheet_metres` | 0.05 | ignored at 1 resident | `GENOS_LIGHTING_SURFACE_CACHE_SHEET_METRES` |
| Cell rays | `surface_cache.cell_rays` | 16 (`LIGHTING_CACHE_RAYS`) | 8 | `GENOS_LIGHTING_SURFACE_CACHE_CELL_RAYS` |
| Body-centred lattice | `surface_cache.sh_bcc` | off | off | `GENOS_LIGHTING_SURFACE_CACHE_SH_BCC` |

The High column is the stage-5 end state. Until that stage, `sh_face_pack` stays off and `residents` stays 1, including on High.

`surface_cache.ray_budget` stays the dispatch cap. `cell_rays` is the rays per resident per relight. `cell_pool` is a power of two. Zero pool forces the pack off. `residents` clamps to 1..6. `sheet_metres` clamps to 0.01..cell size. `cell_rays` clamps to 1..64 and stays a square so the existing stratified pattern still applies; 16 and 8 are the preset values.

Launch arguments on the lighting debug binary, each documented in `--help`: `--face-pack`, `--cell-pool`, `--residents`, `--sheet-metres`, `--cell-rays`, `--sh-bcc`. `--cell` already mirrors `world_cell` if the binary has that argument; if it does not, the implementing stage adds `--cell` with the same rule (argument wins, env is the mirror).

## How the pieces map

Glossary "How to compare a new technique":

| Piece | Role | Our term | Verdict |
|---|---|---|---|
| Face pack | STORE | SH cells, cache entry | extend: one spatial entry, faces in a pool |
| Resident | STORE | cache entry | extend: a second surface inside the face |
| Sheet | VIS | visibility / leak test | replace the half-cell blend permission for same-facing pairs |
| Dot-weighted tilt | STORE | SH cells | equivalent: same tilt, more than one face |
| Body-centred lattice | STORE | SH cells | extend, A/B only |
| Chebyshev moments | VIS | visibility / leak test | new, not on this path (DDGI, GIBS) |
| Plain L1 / sphere rays | SAMPLE | cache ray | rejected: 0.75 lamp, rays through the surface |
| Direction bins | STORE | SH cells | rejected by ADR 0039 |

## Probes

Every failure mode and every knob above is readable. The implementing stage adds them to the lighting command catalogue. Counters land on `gstats` next to the diag words the neighbour read already uses, and the GI dump prints them per cell.

| Failure or knob | Probe |
|---|---|
| Pool exhaustion | counter `cell_pool_overflow`. The picture misses. The dump names the cell. |
| Resident refusal (`R` full) | counter `resident_refused`. Must stay equal to overflow-free leaks: a refused query does not take another resident's mean. |
| Residents and faces live | `residents_live`, `faces_live`. Stage 0 records today's axis-copy occupancy the same way, so the pool cap is measured rather than estimated. |
| One wall split into two residents | resident count in the GI dump. The one-wall fixture expects 1. |
| Cone seam | the cone-sweep fixture: max step between 0.5° samples across 45°. |
| Same-facing leak | the 10 cm fixture and the 2 mm fixture, two colours, each surface reads its own. |
| Young face left out | existing sample-count debug (`lighting_sc_debug_size`). The handover is the sample count, not a switch. |
| Ray cut held a second resident | `diag[38]` / `diag[39]` stay the hold counters. The new case expects hold false. |
| Cell size, pool, residents, sheet, rays, pack, BCC | the settings themselves, MCP and the debug menu. |
| Face mean and SH disagree | one writer, and the dump flags a face whose mean and SH were stored on different frames. It is a bug if it fires. |

Pictures and metrics stay under `./genos-debug/<kind>/` with `--out`. No path under `$HOME`.

## Networking

The implementing commit copies these lines into the lighting system record.

- Replicated state: none. The face pack and the residents are GPU cache on the client that draws. Settings replicate with the other lighting settings.
- Authority: the client that shades.
- Prediction / rollback: none.
- Determinism: the same geometry, the same settings, and the same claim order produce the same residents. The read selects by plane offset and face axis, not by resident index, so the value does not depend on which surface was seen first once both exist. Pictures are not cross-device deterministic.
- Headless server: a server that does not shade does not allocate the pool and does not run the relight.

## Stages

Each stage is behind its setting, default off, and is not allowed to be less accurate than the stage before it. Lavapipe is the correctness gate. Lavapipe times are not a result. 4070 milliseconds are the owner's machine, after the accuracy gate, and are not claimed here. Nothing in the old path is deleted until that stage's gate passes. The deletion is its own commit after the gate. The off setting stays through the next stage so a miss can be compared.

Existing proofs that must stay green at every stage, pack and residents off:

- `cargo test -p genos-render --test texel_detail_prod`
- `cargo test -p genos-render --test texel_ray_cut`
- `cargo test -p genos-render --test probe_retire`
- `cargo test -p genos-render --lib lighting_sh` (the CPU tilt, the 0.75 plain-L1 guard, `neighbour_read`)

`room-blotch` stays parked on its 0.014 gate. This note does not retune it. `cells_only` keeps meaning "25 cm cells, no texels, no probes".

### Stage 0 — fixtures and occupancy, no behaviour change

Plant the gaps in a new `cell_gaps` test beside `texel_detail_prod`. The dispatch is the production gather (`lighting_sh_gather` / `lighting_screen_probe_indirect`), writing compile plants slots, read checks colours. Same rules as `texel_detail_prod`: no swapchain image, not a frame time.

Cases, setting off, asserting **today's** behaviour so the bug is pinned:

- Two axis copies at one place (floor and wall) resolve two keys.
- A normal on each side of `normalize(0, 1, 1)` resolves two different keys. That is the seam.
- `z = 0` and `z = 0.10`, both +Z, one key. `z = 0.124` and `z = 0.126`, two keys, and the plane test accepts both.
- The other side of a thin wall still reads its own cell (already in `texel_detail_prod`).

CPU twins in `lighting_sh.rs` for the face weight and the resident pick, graded the way `neighbour_read` is graded. They are called by the later stages. Stage 0 adds them and does not wire them to the shader.

Occupancy counter for axis copies, printed in the GI dump, so stage 4's pool cap replaces the 12 MB estimate.

Oracle scenes added to `examples/stress/scripts/lighting_oracle.rhai`, scored by the existing `ref_err_near` / `slant_mid_err` path. They do not change old scenarios:

- `bevel`: a 45° strip between a floor and a wall, camera close, path-traced reference.
- `gap`: two +Z planes 10 cm apart, camera close, and a far repeat.

Gate: old tests green. New tests show the three gaps. No shader behaviour change.

Deletes: nothing.

### Stage 1 — face pack and the weighted read (gaps 1 and 2)

Setting `surface_cache.sh_face_pack`, default off. Requires `surface_cache.sh_cells`. Residents stay 1. Sheet is unused.

World-cell claims with the pack on keep the inset and drop the axis salt. A face that shares that inset cell is another pool record on the header. Reads use the A3/B2 weight. `claim` false still does not allocate. Seed, coverage, slants, and movers stay claim false. `lighting_sc_wrc_now` on this path loads the face for `lighting_axis(n)`. The face mean and the SH have one writer, and the sun/sky rescale (`sh_ls`) hits both.

Lavapipe gate:

- Axis-aligned gather equals the old per-axis mean (the tilt-at-axis identity).
- Cone sweep, 0.5° steps: the max step is within the Lipschitz of the two tilts (the band-1 term is linear in the normal), not a step the size of the other room's colour. A wall planted on a grid plane stays on one face through the sweep. A miss of that wall fails the stage.
- Thin wall: each side reads its own face. The back face's dot is ≤ 0.
- `texel_detail_prod`, `texel_ray_cut`, `probe_retire` green with the pack off, and green with the pack on for the cases they already plant (one face, one plane).
- Oracle, pack on against pack off: `slant`, `slant-day` `slant_mid_err` no worse; `turn` `ref_err_near` no worse; `bevel` `ref_err_near` lower than pack off. Energy, linger, and lag stay the ADR stage 1/2 gates when `cache_energy` is run (`cargo test --release -p genos-stress --test cache_energy -- --ignored --nocapture --test-threads=1`). That command is the owner's machine. A lavapipe miss of the picture is a fail; a lavapipe time is not.

Deletes, after that gate, in its own commit: nothing that the off setting still runs. The axis salt and `lighting_sh_gather`'s single-axis `an` stay for pack off.

### Stage 2 — residents (gap 3)

Setting `surface_cache.residents` greater than 1, default 1. Requires the pack. High preset moves to 2 only in stage 5.

Lavapipe gate:

- Planted 10 cm pair: each plane reads its own colour. Planted 2 mm pair across the lattice split: the same.
- One flat wall, including a wall on a grid plane: one resident, sample count matches a single-resident run.
- A ramp in `slant` does not gain a sheet seam: `slant_mid_err` no worse than stage 1.
- `bevel` still beats pack-off.
- `gap` `ref_err_near` lower than stage 1, near camera. The far repeat may tie.
- Ray cut: a second live resident on that face does not hold. The existing hold cases in `texel_ray_cut` still hold. Cell words stay unwritten.
- Thin wall still splits by face, not by resident.
- A third surface with `R = 2` misses and does not take resident 1's colour. `resident_refused` increments.
- Mover, detail on: still `lighting_sh_read` at the current pose, claim false, no new resident.

Deletes, after that gate: the blend permission `plane > LIGHTING_CELL_HALF * cs` inside `lighting_sh_gather` and the same test in `lighting_sc_wrc_now`, **on the residents-on path only**. The off path keeps the test. `neighbour_corner_weight`'s half-cell reject gains the resident twin and keeps the old reject for the off path.

### Stage 3 — BCC A/B

Setting `surface_cache.sh_bcc`, default off. Requires the pack. Four tetrahedral taps, same face and resident rules. Spacing set so the nearest neighbour is `world_cell` (cube edge `world_cell * 2 / sqrt(3)`).

Gate: `bevel` and `turn` `ref_err_near`, and `slant_mid_err`, strictly better than the cubic pack on the oracle, with no thin-wall or `gap` regression. If they are not better, the deletion commit removes the BCC key and the tetrahedral loop, and the setting is rejected in this note. If they are better, the deletion commit removes the cubic eight-corner loop **inside the pack read only**. `lighting_sc_wrc_now` is not that loop.

### Stage 4 — knobs, presets, measured pool

Wire `LIGHTING_SH_CELL` to `world_cache.cell_metres` (High stays 0.25). Replace the `LIGHTING_CACHE_RAYS` literal on the world-cell arm with `surface_cache.cell_rays`. Preset Low: pack off, pool 0, residents 1, cell 0.50 m, cell rays 8, probes unchanged. `crates/scene/tests/lighting_settings.rs` round-trips the new keys, including "argument wins over env".

The occupancy counter from stage 0 sets the High pool: peak live faces × 1.5, rounded up to a power of two, and the bytes stay inside 48 MB. If the measured peak does not fit, High residents drop to 1 before the fat table grows. The 12 MB figure is an estimate until this stage replaces it.

Gate: settings tests. A pool of 0 matches stage 0's pictures on the planted fixtures. Overflow increments `cell_pool_overflow` and misses. `lighting_settings` preset Low still has `world_cache.enabled = false` and does not allocate the pool.

Deletes: the world-mode-3 path that ignores `world_cache.cell_metres` for the SH cell size. The 0.25 m literal remains the High default of the setting, not a second size in the shader.

### Stage 5 — default on, then delete the old world-cell key

Medium and High default the pack on and residents to 2. Low stays off. One release keeps the off switch.

After that is the default and the oracle gates still hold, one commit deletes:

- the axis salt inside `lighting_cache_key_sized` for world cells (the half-cell inset stays);
- `lighting_sh_gather`'s single-axis `an` lookup;
- the off-path plane blend in `lighting_sh_gather` and `lighting_sc_wrc_now`;
- the CPU half-cell reject in `neighbour_corner_weight` once every caller uses the resident pick;
- any mean or SH left on the fat header once every reader loads the face record. The inset and `lighting_axis` for texel keys stay.

Does not delete: `lighting_axis` (texel keys, debug views, and detail-off bilinear still use it); screen-probe place, trace, gather, and filter (ADR stage 5 is a different track; Low keeps probes); texel detail, the ray cut, or the one-way rule; the tilt; the 0.75 plain-L1 test.

## Order against ADR 0039

ADR 0039 stage 1 (convergence, energy, linger, lag) is still the accuracy gate in front of picture claims. The pack does not retune that estimator. It stores the same mean and the same SH with the same blend alpha. Stages here can land behind a default-off setting before that gate is green. They do not become the default, and they do not delete the axis key, until energy, linger, and lag pass.

## What this note does not do

- No engine code, no shader, no settings field in this change.
- It does not reopen direction bins, radiance cascades, cached direct light, or a texel→cell feedback path.
- It does not claim a 4070 millisecond number.
- It does not move the room-blotch 0.014 gate.
