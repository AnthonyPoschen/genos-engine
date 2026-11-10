# ADR 0039 cell gaps

Status: **design for review**. No runtime change. Anthony reviews this before any stage is implemented.

Accepted direction: [ADR 0039](../../adr/0039-lighting-simplification-one-directional-world-cache.md). One colour SH-L1 world cache is the bounce source of truth. Texels are a detail layer. Data flows cells → texels → pixels, never back. This note designs the three gaps [adr-0039-remaining.md](adr-0039-remaining.md) left open after the neighbour read:

1. Each world cell is stored as up to six dominant-axis copies.
2. A seam between those keys when the dominant axis flips.
3. A same-facing surface closer than half a cell leaks. The plane test does not reject it.

The thin-wall reject uses the six copies: the other side is a different entry, and a stored normal that faces away is left out. Collapsing the copies without a replacement split puts that leak back. The three gaps are one layout.

## Recommendation

Pack every live face of one spatial cell into a side pool (a **face pack**). The world-cell hash keeps today's half-cell inset, so a wall on a grid plane stays in one cell, and the world-cell branch of `lighting_cache_key_sized` stops hashing the axis. Texel keys share that function and keep the salt. Each face keeps its uniform-hemisphere rays, the exact cosine mean, the fp16 SH-L1, the sun share and sun baseline, and the relight frame. A query gathers each dominant axis whose dot with the normal is positive (one lattice on a flat wall, two on the cone, three on a corner). The face weight is `max(dot, 0)` times a sample-count ramp inside the gather. The ramp keys off faces stored in the slot: a face with one sample is ramped whenever another stored face exists, including for a pixel 0.5° off the pure axis. It does not key off how many axes that pixel dots. At a pure axis the other dots are zero, so the value is that face's mean. On the cone the adjacent faces both contribute. The opposite face stays a different record and drops out.

Same-facing surfaces inside that face are **cell residents**. The offset is the cell-relative signed distance to the geometric plane, not the dominant-axis coordinate. Each corner converts the query by its own centre, `dot(query - centre, n) - stored_offset`. One plane wins. Every corner on that plane stays in the trilinear sum, and every other plane is dropped. The lower offset breaks a tie inside that comparison. A full face refuses the new surface and the query misses, which keeps the probes and the texel rays.

A body-centred lattice is a later A/B against this cubic pack. It is not the store. Plain L1, a full-sphere trace from a surface point, a smoothstep across the old keys, a tighter plane test alone, and camera-distance cell bands are not the fix.

High pool: 262144 records × 64 B = 16 MiB, inside ADR 0039's SH envelope (the ADR text says 12–48 MB; 1,048,576 × 24 B is 24 MiB). The fat table stays 1,048,576 × 368 B (ADR 0036) plus 1,048,576 × 24 B of SH (ADR 0039), which is 392 MiB. This pool does not grow that table. Low keeps probes, a zero pool, one resident, and a 50 cm cell.

## What the code does now

World mode 3. `LIGHTING_SH_CELL` is `LIGHTING_SC_HIT`, the literal 0.25 m (`lighting_cache.glsl`). `world_cache.cell_metres` is a public setting, clamped to 0.05–4 m, and world mode 3 does not read it: `lighting_cell_size_d` returns `LIGHTING_SC_TEXEL` (0.125 m) for every distance, and `lighting_sh_gather` uses the 0.25 m constant. S57 removed distance bands because a band edge re-keyed entries and restarted them.

`lighting_axis` maps a normal to 0..5: the largest `|component|`, then the sign. Ties prefer X, then Y, then Z.

`lighting_cache_key_sized` builds the world-cell key from three things:

- an inset of half a cell along that axis, so a surface on a grid plane lands in one cell;
- `floor(pos / size - inset)`;
- a hash of that cell, the axis, and the size.

Two orientations at one place are two hash keys. Each key is a full cache entry. ADR 0036's entry is 368 B. ADR 0039 adds the 24 B fp16 SH-L1 (`lighting_sh_put`, `LIGHTING_SH_HALF`) on top of that entry. The table is 1,048,576 × 392 B = 392 MiB. The sun share and sun baseline live in `LIGHTING_SC_SUN_AT` (`x` share, `y` baseline; shown = value × ((1 − x) + x × now / y)). The estimator pair at `LIGHTING_SC_EST_AT` is per slot and is not on the world-cell path. Six copies do not multiply the allocation. They multiply live entries. A thin wall is two entries. A corner is up to three. Six is the cap, not the average. ADR 0036 records no occupancy split by axis.

A 25 cm relight (`lighting_cache.comp`, the `wrc_cell` arm, `cache.v[3 * slot + 1].w > 0.2`) traces `LIGHTING_CACHE_RAYS` (16) rays about the stored normal. `lighting_dir` places them on a `round(sqrt(count))` grid of (cos θ, turn), uniform over the hemisphere (`lighting_hemi_dir`: `u` is the cosine). The cosine is the irradiance weight (`e += L * cos`), not the distribution. The SH sums the same rays, unweighted by cosine: `L` and `L * dir`. The harmonics never see the back hemisphere. The blend weight `wa` is `add / (t + add)`, then the dark-fill cube while `t` is under half the cap (`1 - (t / (t + add))^3`), then a floor while the light is not quiet or a mover is local, then `wa = 1` while a lamp change is settling. The arm writes the mean, the SH, and the sun word, and `return`s. It does not reach the texel estimator. `LIGHTING_CACHE_RAYS` is also the batch stride, the dispatch multiple, and the texel quarter (`LIGHTING_SC_QUARTERS = 64 / 16`). It is not only a ray count.

`lighting_sh_gather` looks up eight corners on **one** axis, the query's dominant axis. The tilt (`ShL1::tilt`, `lighting_sh_tilt`) is the stored mean plus the band-1 change from that axis to the true normal. At the axis it is the mean. `plain_l1_loses_a_quarter_of_a_narrow_source_so_the_read_keeps_the_mean` shows why: plain L1 irradiance of a narrow source straight down the normal is 0.75 of the cosine mean. The read clamps at zero. A corner is left out when it is missing, empty, stale, back-facing (`dot <= 0`), or more than half a cell off the query plane (`plane > LIGHTING_CELL_HALF * cs`, half of 0.25 m = 0.125 m). The sample gate is `v.w < LIGHTING_CACHE_HAS_W` (0.5). The first world-cell relight stores `t + add` with `t = 0`, and `add` is the run count, so a one-batch relight stores 1 and passes 0.5 on that frame. The corner then takes full `tri * facing`. `lighting_sc_shown`'s handover (`n / (n + K)`, `LIGHTING_SC_HANDOVER` 16, `LIGHTING_SC_HANDOVER_K` 2) is the texel display blend. The gather does not call it. Left-out corners add no weight. `lighting_sh.rs` `neighbour_corner_weight` is the CPU twin. The rejected four-cell lattice stays off. An axis-aligned ray hit still tilts the one owning cell.

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
- Lighting VRAM soft budget is about 200 MB. ADR 0039 estimated the SH payload at 12–48 MB; 1,048,576 × 24 B is 24 MiB, and this pool is 16 MiB. The fat table is ADR 0036's 368 B entry plus that 24 B. This note does not grow it.
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

The fat hash slot stays the header so compact and the relight list keep walking slots. It is not a stand-in mean. When the pack is on, the 24 B fp16 SH region on that slot (`LIGHTING_SH_AT` plus the paired half at `LIGHTING_SH_B_AT`) is not harmonics. Harmonics live only on the resident. Those 192 bits hold the six face heads. The index width follows the pool cap, not the default count of 262144. The largest power-of-two pool that stays inside 48 MiB at 64 B is 524288 records (32 MiB), which is 19 bits, and 262144 is only 18. A head is a presence bit plus 19 index bits (120 bits for six heads). 524288 is 2^19, so an index needs 19 bits and has no spare pattern for "empty". The presence bit is the empty mark. The remaining bits stay unused. Stage 4 can raise the count to that cap without widening the head.

`lighting_sc_wrc_now` on the pack-on path stays single-axis. It loads the one face whose axis is `lighting_axis(n)` and reads that face's mean. It does not run the multi-axis gather. Detail-off bilinear stays per axis. One writer updates that face's mean, SH, sun share, and sun baseline together (`sh_ls` scales the mean and the harmonics, and the sun word, the way the slot path does today).

A resident record is 54 B, stored aligned to 64 B:

| Field | Bytes | Why |
|---|---|---|
| Cosine mean, fp32 RGB | 12 | Same range as `cache.v[3 * slot + 2]` |
| Sample count | 4 | The `w` the gather ramps on |
| Cell-relative plane offset, fp16 | 2 | Signed distance from the cell centre to the geometric plane. An absolute fp16 plane is a 3 cm ulp at 32 m (fp16 ulp at 2^5 is 2^-5) |
| SH-L1, fp16 | 24 | The ADR 0039 payload, per resident |
| Sun share and sun baseline, fp32 × 2 | 8 | `LIGHTING_SC_SUN_AT` `x` and `y`. Two faces do not share one share |
| Relight frame | 4 | The stamp `lighting_sc_stale` uses. One header stamp cannot stale one face and not the other |

54 B aligns to 64 B. The 10 B pad holds the next-resident link: a has-next bit and a 19-bit index, the same width as the head. The link is not in the fat slot. Six heads plus six second indices would be 240 bits and do not fit the 192-bit SH region. 262144 × 64 B = 16 MiB. 524288 × 64 B = 32 MiB, which is the cap the 19-bit field is sized for.

The lit-from point stays `lighting_sc_centre` on that resident's plane. Empty faces take no record.

Rays stay uniform on the face's hemisphere, from that resident's point. The cosine remains the irradiance weight. The sample domain does not become a sphere. The `wa` path is the world-cell arm's path, copied onto the resident: plain `add / (t + add)`, dark fill, the change floor, and `wa = 1` while settling. The ray loop (direction, cosine weight, SH sums in registers) is the part that is reused. The stores are not. `lighting_tex_put`, `lighting_sh_put`, the sun word, and the relight frame index a slot in the fat table. A pool index is not a slot. Those four writes are retargeted at the resident. The estimator arrays stay per fat slot. The world-cell arm returns before them, so a face does not grow an estimator pair. There is no second relight shader. There is a retargeted store.

Read: for each dominant axis with `dot(axis, n) > 0`, the eight corners of **that axis's inset lattice**. From stage 2, the query has already chosen one winning plane (below). Corners on any other plane are dropped before the sum. A kept corner contributes

`max(dot(axis, n), 0) * trilinear * ramp(n) * that resident's tilt`.

`ramp` is inside this gather. It is not `lighting_sc_shown`. It is a property of the face stored in the slot, not of how many axes this pixel dots.

- The slot stores only this face, and `n >= 1`: `ramp = 1`. A lone wall still shows its mean on the first relight, which is today's `w = 1` accept.
- The slot stores any other face: `ramp(n) = clamp((n - 1) / LIGHTING_SC_HANDOVER, 0, 1)` with `LIGHTING_SC_HANDOVER = 16`. A face with `n = 1` has ramp 0 on every query that reads it. A pixel on the pure axis and a pixel 0.5° off that axis use the same ramp. Counting positive dots splits them: the pure-axis pixel has one positive dot and takes the young face at full weight, and the pixel 0.5° off has two and takes that face at ramp 0, so it shows the other face's colour for one frame. That is the step at the wall/bevel join. Each later sample raises that face by 1/16 of its dot weight. It reaches full weight at `n = 17`.

Weights renormalize. A missing face, a stale face (its own relight frame), and a ramp of 0 add nothing and do not darken. At a pure axis every other dot is 0, the other walks are skipped, and the value is that face's mean (`assert_eq!(sh.tilt(e, axis, axis), e)`). On the cone, `normalize(0, 1, 1)` gives +Y and +Z `dot = 0.707`. The tie itself is classified +Y, because `|y| == |z|` takes the `a.y >= a.z` arm. A hair toward +Z classifies +Z. The sweep is that neighbourhood. The opposite face is `dot <= 0` and stays out. The pack-on gather keeps `abs(dot(stored - p, n)) > 0.5 * cell` through stage 1. Removing it there would accept a second same-facing surface in the same band the test already fails to split, before a resident exists. Stage 2 deletes the test on the pack-on path. The resident match replaces it. The pack-off path keeps the test until stage 5.

| | |
|---|---|
| Memory | Pool, not a ×6 stripe on the 1M table. Six always-on SH copies would be 6 × 24 B × 1,048,576 = 144 MiB, past the ADR 0039 envelope. A pool of live residents does not. High default 262144 × 64 B = 16 MiB. The fat slot stays a header. Its 24 B SH region holds the six 19-bit heads when the pack is on. An extra face is 64 B, not another 392 B slot. |
| Compute | Relight count matches live faces, which is today's live axis copies. A flat wall is eight hash lookups, the same as today. The cone walks a second lattice (16) and a corner a third (24), each probe up to `LIGHTING_CACHE_SEARCH` (16). That cost is listed and does not reject the option. A cell that already holds both faces still needs the second walk when the other face was claimed on the other inset. |
| Accuracy | Axis-aligned surfaces keep today's mean, so the energy gate has the same estimator it has now. Slanted surfaces use the tilt they already use, from the face that actually sampled that hemisphere. Thin walls stay two records. |
| Failure | A seventh orientation in one cell cannot happen: there are six axes. Pool exhaustion refuses the claim. The query misses (probes, texel rays). It does not alias onto another face. A face with `n = 1` has ramp 0 when the slot stores another face, on the pure axis and 0.5° off it, so the arrival frame does not step. Leaving that face at full `tri * facing` once `w >= 0.5` is the pop this ramp exists to remove. |

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
| Failure | The two copies converge on different frames. Without the gather ramp, the young side enters at full weight on its first relight (`w = 1`). The ramp above is what keeps that frame still. The inset lattices still differ, so this blends two grids. It does not delete the six copies, and gap 3 is untouched. Shipping it and then packing deletes it. |

Useful as a fallback experiment if the pack's key change fails the thin-wall gate. Not the recommended build.

### Option B2. The face-pack read (A3)

Same weights as B1. The axis id is gone from the key, so two faces that share an inset cell share a header and do not drift as two fat entries. Faces that the inset still separates stay on their own cells. The second walk reads those face records. That is B1's walk against the pack.

The inset stays in the key. Dropping it would put a grid-plane wall on a cell boundary, and a millimetre of hit jitter would split its samples. The pure-axis gather is that inset lattice, which is the lattice the claim wrote, so it finds the cell. The cone adds the other axis's inset lattice. Stage 1 plants a wall on a grid plane and walks the normal through the cone in 0.5° steps. A miss on that wall fails the stage.

| | |
|---|---|
| Memory | The A3 pool. |
| Compute | Eight lookups on a flat wall. Sixteen on the cone, twenty-four on a three-axis corner. Same figure as B1. A shared inset cell stores both faces on one header instead of a second fat entry. |
| Accuracy | Exact at the axis, because the other weights are zero and the tilt-at-axis identity holds. Across the cone, a face with `n = 1` contributes only through `ramp` when the slot already stores another face. A slot with one stored face stays at `ramp = 1`. The positive-dot count does not choose the ramp. |
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

Not the fix. The pack-off gather keeps `0.5 * cell`. The pack-on gather keeps it through stage 1 as well. Stage 2 deletes it when the resident match replaces it. Stage 1 does not widen the same-facing band. Shrinking the constant still does not split a shared slot.

### Option C2. Put a plane quantum in the hash key

Add `floor(pos[axis] / sheet)` to the key. High sheet 5 cm. A 10 cm gap becomes two entries. The query knows its plane, so the lookup count stays eight.

| | |
|---|---|
| Memory | One extra fat slot per sheet crossing. A bookcase in one column is several full entries, not 64 B residents. That fights the packing in A3. |
| Compute | Same eight lookups. |
| Accuracy | Parallel axis-aligned gaps larger than a sheet split correctly. |
| Failure | A ramp's axis coordinate changes by about `cell * sin(tilt)` across the cell. At 45° and 25 cm that is about 18 cm, several 5 cm sheets, so one ramp seams and stores duplicate samples. A key that depends on the shading normal seams a smooth cylinder the same way. Hysteresis on the sheet index sticks, then pops when it lets go. |

Rejected as a hash salt. The sheet belongs inside the cell, as C3.

### Option C3. Residents inside the face (recommended)

Each face holds up to `R` residents, linked by the next-resident field. High default `R = 2`. High sheet default 0.05 m, so the attach radius is 0.025 m.

The stored offset is the signed distance from the cell centre to the geometric plane: `dot(pos - cell_centre, geometric_normal)`. A shading normal and a normal map do not move it. A flat plane, including a 45° ramp, has one constant. The ~18 cm figure (`0.25 * sin(45°)`) is how far the dominant-axis coordinate walks across the cell. That coordinate is not the offset. Storing it would make one ramp miss two 2.5 cm radii (they cover at most 10 cm) and either seam or attach the wrong resident, which is the C2 failure. An absolute world distance in fp16 is the wrong unit as well: at 32 m the ulp is about 3 cm, coarser than the sheet.

A hit attaches to the resident within the radius. A hit outside every radius claims a new resident when the face is under `R`. There is no temporal hysteresis. An exact tie (two residents at the same distance) keeps the lower offset. It does not average, and it does not follow which resident was claimed first.

The read converts each corner by that corner's own centre. The distance from the query to a resident at that corner is `dot(query - centre, n) - stored_offset`, where `centre` is the corner's cell centre and `stored_offset` is that resident's cell-relative plane offset. The winning plane is the smallest of those distances. An equal distance keeps the lower offset, and that rule is only the tie break inside this comparison. It is not a sort of the stored offsets before the distance is computed.

Once that plane has won, every corner on it stays in the trilinear sum. Corners on any other plane are dropped. A corner is not dropped because its own centre would have preferred a different resident, and it is not kept because it lost the plane test by half a cell. If each corner kept its own nearest plane, the 2 mm pair would still blend. A query outside every radius misses. Dropped corners add no weight and do not darken the winner. The masked bevel gate checks this: a 45° ramp is one plane, each of the eight corners is converted by its own centre, and all eight stay.

Relight origin is that resident's `lighting_sc_centre`. Rays stay uniform on the face hemisphere. The cosine is the irradiance weight. The scheduler walks the resident list. The ray loop is the world-cell loop. The mean, SH, sun word, and relight frame are stored on the resident, not through the slot-indexed writes.

Ray cut: `lighting_compact_hold_texel_rays` does not hold when the gather's face has a second live resident, even if the colours match. The cell is not a complete description of that texel. One resident and an agreeing field still hold, which is today's cut.

| | |
|---|---|
| Memory | `R` scales how many of the 262144 records are live, not the 392 MiB table. The cap is 16 MiB either way. Typical rooms use one resident per face; the second is the leak case. Average occupancy is the stage 0 counter. The cap is the budget, not the average. |
| Compute | One offset compare per resident per query, then a corner keep-or-drop. Extra rays only for a live extra resident. Ray count per resident is `surface_cache.cell_rays`, High 16. |
| Accuracy | The 10 cm pair and the 2 mm pair each keep a mean, and the trilinear sum sees only the winner. A single flat wall, and a 45° ramp, stay one resident. Bounce rays read that resident, so the leak does not survive into the next bounce. Near the camera a miss still has texel rays and probes. Far from the camera, `R` full and a miss is the coarse answer; `ref_err_near` weights that down. |
| Failure | A third surface with `R = 2` is refused. The picture misses. It does not show resident 1's light on surface 3. A surface that jitters around the radius boundary can open two residents. The tie break is the lower offset, so the two do not swap. The one-wall fixture and the ramp fixture expect a resident count of 1. A moving panel is already a mover: detail-on reads the pack at the current pose, `claim` false, and does not allocate. Object-local texel keys stay salted and are not residents. |

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
| Cell rays | `surface_cache.cell_rays` | 16 (4×4) | 9 (3×3) | `GENOS_LIGHTING_SURFACE_CACHE_CELL_RAYS` |
| Body-centred lattice | `surface_cache.sh_bcc` | off | off | `GENOS_LIGHTING_SURFACE_CACHE_SH_BCC` |

The High column is the stage-5 end state. Until that stage, `sh_face_pack` stays off and `residents` stays 1, including on High.

`surface_cache.ray_budget` stays the dispatch cap. `cell_pool` is a power of two. Zero pool forces the pack off. `residents` clamps to 1..6. `sheet_metres` clamps to 0.01..cell size.

`surface_cache.cell_rays` is the ray count of one world-cell resident relight. It is a perfect square in 1..64. High is 16. Low is 9. 4 is the coarser square Low may use. 8 is rejected: `lighting_dir` sets the grid side to `uint(sqrt(count) + 0.5)`, which is 3 for 8, and the loop only traces 8 of those 9 cells.

The constant `LIGHTING_CACHE_RAYS` (16) keeps three other jobs, and the knob does not take them:

- batch stride (`j * LIGHTING_CACHE_RAYS`, and the ray index in `lighting_cache.comp`);
- dispatch size (`(entries * LIGHTING_CACHE_RAYS + 63) / 64`, and the same multiple on the grad cap);
- texel quarters (`LIGHTING_SC_SET_RAYS` 64 divided by 16).

End to end, the knob is a `LightingSettings` field, a `set_key` name, MCP through the existing lighting commands, a debug-menu control, `--cell-rays` (the argument wins), and `GENOS_LIGHTING_SURFACE_CACHE_CELL_RAYS` only when the argument was omitted. The world-cell arm loops `cell_rays` times and calls `lighting_dir` with that count. The world-cell list's ray product uses `cell_rays`. Texel lists, the batch stride, and `LIGHTING_SC_QUARTERS` stay on 16. A resident relight that is not a multiple of the batch stride is dispatched as its own group sized from `cell_rays`, not by pretending the resident is a fat slot of 16.

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
| Young face enters at full weight while another face is stored | the gather ramp, keyed off stored faces in the slot. The arrival fixture (stage 1) expects a zero step on the frame `n` first hits 1, on the bevel and 0.5° off the pure axis. `lighting_sc_debug_size` still shows the count. `lighting_sc_shown` is not this ramp. |
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

`cell_gaps` is not in that list. It does not exist until stage 0 adds it. `room-blotch` stays parked on its 0.014 gate. This note does not retune it. `cells_only` keeps meaning "25 cm cells, no texels, no probes".

### Stage 0 — fixtures and occupancy, no behaviour change

There is no `cell_gaps` test today. This stage adds one beside `texel_detail_prod`. The dispatch is the production gather (`lighting_sh_gather` / `lighting_screen_probe_indirect`), writing compile plants slots, read checks colours. Same rules as `texel_detail_prod`: no swapchain image, not a frame time.

Cases, setting off, asserting **today's** behaviour so the bug is pinned:

- Two axis copies at one place (floor and wall) resolve two keys.
- A normal on each side of `normalize(0, 1, 1)` resolves two different keys. That is the seam.
- `z = 0` and `z = 0.10`, both +Z, one key. `z = 0.124` and `z = 0.126`, two keys, and the plane test accepts both.
- The other side of a thin wall still reads its own cell (already in `texel_detail_prod`).

CPU twins in `lighting_sh.rs` for the face weight and the resident pick, graded the way `neighbour_read` is graded. They are called by the later stages. Stage 0 adds them and does not wire them to the shader.

Occupancy counter for axis copies, printed in the GI dump, so stage 4's pool cap replaces the 262144-record estimate. The record stays 64 B. 16 MiB is 262144 × 64 B, not a measured occupancy.

Oracle scenes added to `examples/stress/scripts/lighting_oracle.rhai`. They do not change old scenarios. `slant_mid_err` is not their grade. That metric crops the middle half of `slant` / `slant-day`. Most of those pixels are the axis-aligned floor; `scripts/lighting/slant_mid_split.py` classes a wall as `other` and treats a floor-only reading of the gate as a failed premise, which is a different claim from "the crop covers a ramp". It does not. The new scenes carry a mask, and the score is `ref_err_near` on the mask:

- `bevel`: a 45° strip between a floor and a wall, camera close, path-traced reference. The mask is the pixels whose geometric normal lies within 10° of the cone (`||n_y| - |n_z||` small, and both larger than `|n_x|`). The floor outside that band is not in the mask.
- `gap`: two +Z planes 10 cm apart, camera close, and a far repeat. The mask is the pixels on those two planes, split so each plane is scored on its own. A blend of the two colours fails the near plane.

Gate: old tests green. New tests show the three gaps. No shader behaviour change.

Deletes: nothing.

### Stage 1 — face pack and the weighted read (gaps 1 and 2)

Setting `surface_cache.sh_face_pack`, default off. Requires `surface_cache.sh_cells`. Residents stay 1. Sheet is unused.

World-cell claims with the pack on keep the inset. The world-cell branch of `lighting_cache_key_sized` stops hashing the axis. The texel branch of that function still hashes it. A face that shares that inset cell is another resident on the header. Reads use the A3/B2 weight, including `ramp`, and `ramp` follows the faces stored in the slot. The pack-on gather keeps the half-cell plane test. Residents do not exist yet, so removing the test would widen the same-facing band. `claim` false still does not allocate. Seed, coverage, slants, and movers stay claim false. `lighting_sc_wrc_now` on this path stays single-axis: the face for `lighting_axis(n)`, not the weighted gather. It keeps the plane test too. One writer updates that face's mean, SH, sun share, and sun baseline (`sh_ls`).

Lavapipe gate, on the stage 0 `cell_gaps` test once it exists:

- Axis-aligned gather equals the old per-axis mean (the tilt-at-axis identity), including a slot that stores only one face with `n = 1` (`ramp = 1`).
- Cone sweep, 0.5° steps, both faces already mature: the max step is within the Lipschitz of the two tilts (the band-1 term is linear in the normal), not a step the size of the other room's colour. A wall planted on a grid plane stays on one face through the sweep. A miss of that wall fails the stage.
- The half-cell plane test stays on. A second same-facing plane is the stage 2 fixture. This stage does not accept a wider band than pack off.
- Arrival: on the 45° bevel, the second stored face's count goes from 0 to 1 in one relight during the sweep. That frame's gather equals the one-face gather, because the ramp follows the stored faces and the new face's ramp is 0. The next relight moves the colour by at most one step of `ramp` (1/16 of that face's dot weight), not the whole colour difference.
- Arrival, 0.5° off the pure axis, at the wall/bevel join. The slot already stores the wall face. The bevel face's first sample is `n = 1`. The pixel 0.5° off the wall axis and the pixel on the wall axis agree: the new face contributes 0. The off-axis pixel does not take it at full `tri * facing` for one frame.
- Thin wall: each side reads its own face. The back face's dot is ≤ 0. Sun share and the relight frame are per face: a stale face is left out while the other face still contributes.
- `texel_detail_prod`, `texel_ray_cut`, `probe_retire` green with the pack off, and green with the pack on for the cases they already plant (one face, one plane).
- Oracle, pack on against pack off: `slant` and `slant-day` `slant_mid_err` no worse (that crop is not the seam); `turn` `ref_err_near` no worse; `bevel` masked `ref_err_near` lower than pack off; `gap` is not this stage's leak gate. Energy, linger, and lag stay the ADR stage 1/2 gates when `cache_energy` is run (`cargo test --release -p genos-stress --test cache_energy -- --ignored --nocapture --test-threads=1`). That command is the owner's machine. A lavapipe miss of the picture is a fail; a lavapipe time is not. Merging this stage before those three gates is the exception in "Order against ADR 0039".

Deletes, after that gate, in its own commit: nothing that the off setting still runs. The axis salt and `lighting_sh_gather`'s single-axis `an` stay for pack off.

### Stage 2 — residents (gap 3)

Setting `surface_cache.residents` greater than 1, default 1. Requires the pack. High preset moves to 2 only in stage 5.

Lavapipe gate:

- Planted 10 cm pair: each plane reads its own colour. Planted 2 mm pair across the lattice split: the same.
- One flat wall, including a wall on a grid plane: one resident, sample count matches a single-resident run.
- A 45° ramp: one resident. The masked bevel crop checks the corner conversion: each corner uses its own centre, `dot(query - centre, n) - stored_offset`, and all eight corners of that one plane stay. The crop does not gain a sheet seam. `slant_mid_err` is not this check.
- The 2 mm pair: one winning plane. Every corner on that plane stays in the sum. Other planes are absent. An equal distance keeps the lower offset, and that tie break is inside the per-corner comparison, so every corner names the same plane.
- `bevel` masked `ref_err_near` still beats pack-off.
- `gap` masked `ref_err_near` on the near plane is lower than stage 1. The far repeat may tie.
- Ray cut: a second live resident on that face does not hold. The existing hold cases in `texel_ray_cut` still hold. Cell words stay unwritten.
- Thin wall still splits by face, not by resident.
- A third surface with `R = 2` misses and does not take resident 1's colour. `resident_refused` increments.
- Mover, detail on: still `lighting_sh_read` at the current pose, claim false, no new resident.

Deletes, after that gate: `plane > LIGHTING_CELL_HALF * cs` on the pack-on path in `lighting_sh_gather` and in `lighting_sc_wrc_now`. The resident match replaces it. Stage 1 left the test in place. The pack-off path keeps it. `neighbour_corner_weight` gains the one-winning-plane pick and keeps the old half-cell reject for the pack-off path.

### Stage 3 — BCC A/B

Setting `surface_cache.sh_bcc`, default off. Requires the pack. Four tetrahedral taps, same face and resident rules. Spacing set so the nearest neighbour is `world_cell` (cube edge `world_cell * 2 / sqrt(3)`).

Gate: masked `bevel` `ref_err_near` and `turn` `ref_err_near` strictly better than the cubic pack, with no thin-wall or masked `gap` regression. `slant_mid_err` is reported and is not the decision. If the masked gates are not better, the deletion commit removes the BCC key and the tetrahedral loop, and the setting is rejected in this note. If they are better, the deletion commit removes the cubic eight-corner loop **inside the pack read only**. `lighting_sc_wrc_now` is not that loop.

### Stage 4 — knobs, presets, measured pool

Wire the gather's cell size to `world_cache.cell_metres` (High stays 0.25). That size is the `cs = LIGHTING_SH_CELL` constant, and `LIGHTING_SH_CELL` is `LIGHTING_SC_HIT` (0.25). Preset Low: pack off, pool 0, residents 1, cell 0.50 m, `cell_rays` 9, probes unchanged. `crates/scene/tests/lighting_settings.rs` round-trips the new keys, including "argument wins over env", and rejects a `cell_rays` that is not a square.

The occupancy counter from stage 0 sets the High pool: peak live residents × 1.5, rounded up to a power of two, and the bytes (count × 64) stay inside 48 MiB. If the measured peak does not fit, High residents drop to 1 before the fat table grows. 16 MiB is the default cap (262144 × 64 B) until this stage replaces the count.

Gate: settings tests. A pool of 0 matches stage 0's pictures on the planted fixtures. Overflow increments `cell_pool_overflow` and misses. `lighting_settings` preset Low still has `world_cache.enabled = false` and does not allocate the pool.

Deletes: the gather's `cs = LIGHTING_SH_CELL` / `LIGHTING_SH_CELL = LIGHTING_SC_HIT` literal. The 0.25 m value remains the High default of `world_cell`. `lighting_cell_size_d` returning `LIGHTING_SC_TEXEL` (0.125) stays. That return is the texel size.

### Stage 5 — default on, then delete the old world-cell key

Medium and High default the pack on and residents to 2. Low stays off. One release keeps the off switch.

After that is the default and the oracle gates still hold, one commit deletes:

- the axis salt on the world-cell branch of `lighting_cache_key_sized` (the half-cell inset stays; the texel branch of that function still hashes `lighting_axis`);
- `lighting_sh_gather`'s single-axis `an` lookup;
- the off-path plane blend in `lighting_sh_gather` and `lighting_sc_wrc_now`;
- the CPU half-cell reject in `neighbour_corner_weight` once every caller uses the resident pick;
- any mean or SH left on the fat header once every reader loads the face record. The inset and `lighting_axis` for texel keys stay.

Does not delete: `lighting_axis` (texel keys, debug views, and detail-off bilinear still use it); screen-probe place, trace, gather, and filter (ADR stage 5 is a different track; Low keeps probes); texel detail, the ray cut, or the one-way rule; the tilt; the 0.75 plain-L1 test.

## Order against ADR 0039

ADR 0039 decision 4: stage 2 (SH-L1 cells) may be prototyped in parallel, and it merges only after stage 1 passes energy ≥ 0.9, linger ≤ 5 % at 120 frames, and lag ≥ 90 % at 300 frames.

This note's stages are default-off and would merge before that gate. That is an exception to decision 4. Anthony accepts it or drops it.

- Accept: the default-off commits may merge. They still do not become the Medium/High default, and they still do not delete the world-cell axis branch, until `cache_energy` passes those three numbers.
- Drop: the commits stay off the branch that ADR 0039 stage 1 has to land on. Nothing in this note merges until that gate is green.

The pack does not retune the estimator. It stores the same mean and the same SH with the same `wa`, including dark fill, the change floor, and `wa = 1` while settling.

## What this note does not do

- No engine code, no shader, no settings field in this change.
- It does not reopen direction bins, cached direct light, or a texel→cell feedback path.
- It does not claim a 4070 millisecond number.
- It does not move the room-blotch 0.014 gate.
