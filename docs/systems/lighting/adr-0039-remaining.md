# ADR 0039 remaining work

Accepted design: `docs/adr/0039-lighting-simplification-one-directional-world-cache.md` (Anthony, 5 Nov 2026). Sequencing after 8 Nov: land the agreed path end to end, then refine accuracy. A quality miss on the old cell picture is parked. A numerical invalid, a leak, a hang, or a missing frame wire is a functional blocker.

Columns: **functional** means the frame path does the thing. **accuracy** and **performance** are later gates. They are not reasons to keep a superseded estimator in front of a wired replacement.

## This stage

| Item | Kind | State |
|---|---|---|
| Empty fine world texel shows the one owning SH-L1 cell, tilted to its normal | functional | `surface_cache.sh_texel_detail`, default off. Compose's `lighting_world_irradiance` returns it. Denoiser off included. |
| Seed is not an own sample. Lamp change drops a stored seed, then the cell is copied again | functional | `lighting_sc_set_seed`, confidence 0.25. Cell mean, harmonics, and read stamp are not written |
| Covered texel does not also take the resample, parent, hit-patch, or probe seed | functional | Those writers stay for a miss |
| Neighbour blend | — | Not this seed. Stage 2's one-cell owner read stands. The four-cell experiment is not this setting |

Departure from ADR 0039: the ADR's texel seed is a trilinear blend. This seed follows the stage 2 owner fix (one cell, true-normal tilt). The four-cell blend is the read that stage 2 already left, and it is not turned back on here.

## Still in the frame, and what removes them

| Legacy path | Still runs when | Removal depends on |
|---|---|---|
| Resample, parent bilinear, hit patch, probe seed | Texel detail off, or the owning cell has no light, or the texel is a mover | This setting, once the cell has light. Uncovered misses stay until cells cover that texel |
| Screen-probe trace, gather, and filter | Every GI frame on Medium and above. Pixels mix probes in where world trust is below 1 | Stage 5, after slanted pixels and movers are covered. Low keeps probes until a 1060 measurement exists (ADR open question 2) |
| Slanted screen lattice | `sh_cells` on. The read is empty unless a normal component has magnitude 1, and compose keeps the probes | A later cell read that accepts a slanted normal without the claim side effect stage 2 removed. Not invented here |
| Mover texels | Object-frame entries. Texel detail does not seed them | Mover-only probe helper if cells cannot keep up (ADR decision 3). Not deleted in this stage |
| SVGF four-cell world read (`lighting_sc_wrc_now`) | Texel detail off, and a texel that already has samples whose field is stale | Texel detail on skips that read. The texel's own samples show instead |
| Direct per-pixel light, including the GI-on lamp-ray cost | Always. ADR D1 | D1 explains the ray count. Not this stage |
| ReSTIR and world-cell light lists | Not the bounce seed. ADR D2, where the direct path is approved | Its own stage. Temporal reuse is that reservoir's history for a matched depth and normal, not a new bounce cache |

## Parked (accuracy or performance, not a functional stop)

| Issue | Kind | What it waits on |
|---|---|---|
| Cache-only room blotch above 0.014. 4070 means were about 0.193 and 0.228 / 0.158 against 0.014 | accuracy | Cell convergence and the spatial read. Not another old-cell probe before the replacement path is in |
| Cell energy, linger, and lag (ADR stage 1) | accuracy | The same cells, after the path exists. Thresholds stay |
| Turn and slant absolute gates (0.403, probes-on slant) | accuracy | Stage 2 picture gates. Relative r15 is not those gates |
| Texel ray cut where the texel agrees with the cell (ADR stage 4) | performance | This seed. Contact, corner, and thin-wall gates stay |
| Probe dispatch retirement (ADR stage 5) | performance | Coverage of slanted pixels and movers, then an A/B. Deletion is a later change |
| GI-on lamp-ray cost (ADR D1) and reservoir memory (ADR D2) | performance | Those stages. The ~200 MB figure is a soft preference, not a hard gate |
| 4070 milliseconds | performance | After an accuracy gate, on the owner's machine. Lavapipe times are not that number |

Adaptive subdivision, ray feedback into cells, and memory pools are not in the accepted ADR body. They stay out.

## Proof for this stage

`cargo test -p genos-render --test texel_detail_prod` on lavapipe. The dispatch is `lighting_world_irradiance_in`. Covered red owner, lamp clear to blue, own samples, mover trust, slant left empty, denoise off, and detail off are separate rows. The room-blotch harness is unchanged: it does not set `sh_texel_detail`.
