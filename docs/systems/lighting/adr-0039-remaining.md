# ADR 0039 remaining work

Accepted design: `docs/adr/0039-lighting-simplification-one-directional-world-cache.md` (Anthony, 5 Nov 2026). Sequencing after 8 Nov: land the agreed path end to end, then refine accuracy. A quality miss on the old cell picture is parked. A numerical invalid, a leak, a hang, or a missing frame wire is a functional blocker.

Columns: **functional** means the frame path does the thing. **accuracy** and **performance** are later gates. They are not reasons to keep a superseded estimator in front of a wired replacement.

## This stage

| Item | Kind | State |
|---|---|---|
| Empty fine world texel shows the trilinear neighbour read at its true normal | functional | `surface_cache.sh_texel_detail`, default off. Compose's `lighting_screen_probe_indirect` returns it. Denoiser off included. Seed, coverage, and the screen slant use that same read |
| Seed is not an own sample. Lamp change drops a stored seed, then the cells are copied again. An empty neighbour read does not keep a marked seed on screen | functional | `lighting_sc_set_seed`, confidence 0.25. The detail mark is a negative confidence. Sun word z is the quarter scratch and is not the mark. `lighting_sc_seed` refuses a negative confidence when the neighbour read is empty, on every return, including default High denoise. Unmarked seeds stay positive. Cell mean, harmonics, and read stamp are not written by the read. The read-only compose does not need the lamp clear |
| Covered texel does not also take the resample, parent, hit-patch, or probe seed | functional | An empty fine texel this setting covers does not take resample or the hit patch, even when the neighbour read accepts nothing. The probe seed still runs for that miss, so a stale, back-facing, or off-plane cell is not copied back in. Resample and the hit patch stay when the texel is not a detail candidate: detail off, a mover, own samples, or a coarse size. The parent walk is off for every texel while texel detail is on, including a mover. A mover's trust is 0, so it shows the probes without that walk. A slant returns the cell read and does not run them |
| Covered pixel does not read screen-probe irradiance | functional | Only when that neighbour read is the displayed indirect, including a lit slant. Own samples, a parent, a resample, and a partial corner still read the probes. Place, trace, gather, and filter stay |
| Neighbour blend | functional | Eight corners, true-normal tilt. The corner weight is the trilinear weight times `dot(stored normal, query normal)`. Equal stored normals cancel that dot. Stored normals that both face the query do not, so that blend is not pure trilinear. A corner is left out when it is missing, empty, stale, back-facing (`dot <= 0`), or more than half a cell off the query plane. Left-out corners do not darken the blend. A black mean with samples does. The read does not allocate. Claim stamps only an accepted corner; seed, coverage, and slants pass claim false. Cells stay keyed by the dominant axis. The rejected four-cell lattice stays off. An axis-aligned ray hit still tilts the one owning cell. A detail candidate whose read accepts nothing does not call that hit read. SH cells on with texel detail off still does |

The six axis copies are still separate entries. The thin-wall reject uses that: the other side is a different cell, and a stored normal that faces away is left out. Collapsing those copies into one entry is not this read. A seam between dominant-axis keys is still open. A same-facing surface closer than half a cell also stays: the plane test does not reject it, so that leak is not solved here. Neither gap gets a new design in this change.

## Still in the frame, and what removes them

| Legacy path | Still runs when | Removal depends on |
|---|---|---|
| Resample and the hit patch | Not a detail candidate: texel detail off, a mover, own samples, or a coarse size | An empty fine texel with texel detail on does not run these, even when the neighbour read accepts nothing. The probe seed is the miss. SH cells on with texel detail off still runs the hit patch. A mover is not seeded. A slant returns the cell read and does not run these writers |
| Parent walk | Texel detail off | While texel detail is on the walk does not run, including for a mover. The mover still shows the probes because its trust is 0. The walk is not put back |
| Probe seed | The neighbour read is not what the pixel shows: a miss on a covered texel, own samples, a parent, a partial corner, a mover, detail off, Low | A covered texel with an accepted corner does not take it |
| Screen-probe irradiance read in compose | The displayed indirect is not the neighbour read. That includes a miss, an empty slant, a mover, own samples, a parent, a resample, a partial corner, detail off, and Low | Covered pixels, including a lit slant, skip this read. Trust 1 is not the test |
| Screen-probe place, trace, gather, and filter | Every GI frame on Medium and above that still has an uncovered pixel, an empty slant, or a mover. Low has no world cache | Stage 5, after movers are covered and the remaining probe readers are gone. Not a global probe disable. Low keeps probes until a 1060 measurement exists (ADR open question 2) |
| Mover texels | Object-frame entries. Texel detail does not seed them, and compose still reads the probes | Mover-only probe helper if cells cannot keep up (ADR decision 3). Not deleted in this stage |
| SVGF per-axis world read (`lighting_sc_wrc_now`) | Texel detail off, and a texel that already has samples whose field is stale | Texel detail on skips that read. The texel's own samples show instead |
| Direct per-pixel light, including the GI-on lamp-ray cost | Always. ADR D1 | D1 explains the ray count. Not this stage |
| ReSTIR and world-cell light lists | Not the bounce seed. ADR D2, where the direct path is approved | Its own stage. Temporal reuse is that reservoir's history for a matched depth and normal, not a new bounce cache |

## Parked (accuracy or performance, not a functional stop)

| Issue | Kind | What it waits on |
|---|---|---|
| Cache-only room blotch above 0.014. 4070 means were about 0.193 and 0.228 / 0.158 against 0.014 | accuracy | Cell convergence and the spatial read. Not another old-cell probe before the replacement path is in |
| Cell energy, linger, and lag (ADR stage 1) | accuracy | The same cells, after the path exists. Thresholds stay |
| Turn and slant absolute gates (0.403, probes-on slant) | accuracy | Stage 2 picture gates. Relative r15 is not those gates |
| Texel ray cut where the texel agrees with the cell (ADR stage 4) | performance | This seed. Contact, corner, and thin-wall gates stay |
| Probe dispatch retirement (ADR stage 5) | performance | Coverage of slanted pixels and movers, then an A/B. The compose read is already skipped for covered axis pixels. Deletion of place, trace, gather, and filter is a later change |
| GI-on lamp-ray cost (ADR D1) and reservoir memory (ADR D2) | performance | Those stages. The ~200 MB figure is a soft preference, not a hard gate. D2 is the existing ReSTIR path, not a second reservoir |
| 4070 milliseconds | performance | After an accuracy gate, on the owner's machine. Lavapipe times are not that number |

Adaptive subdivision, ray feedback into cells, and memory pools are not in the accepted ADR body. They stay out.

## Proof for this stage

`cargo test -p genos-render --test texel_detail_prod` on lavapipe. The dispatch is `lighting_screen_probe_indirect`, which compose calls. One test compiles that function with `LIGHTING_SC_WRITES` false and reads slots the writing compile planted, with no lamp clear. Rust `lighting_change_head` writes head 3.w, including bit 17, and the shader copies that word. A covered owner skips the probe read. Two planted corners blend by their trilinear weights. A missing corner stays out, and a black corner with samples darkens. A back-facing corner and a corner past half a cell off the plane stay out. The other side of a thin wall reads its own cell. A stale cell stays in memory and the pixel shows the probe. A slant shows the true-normal tilt and skips the probe read. One covered corner beside an own-sample corner does not skip. An unmarked parent with sun scratch 40 stays the parent. A marked seed whose owner is then emptied, after sun word z is cleared, shows the probe. A read-only compose with a marked green seed and an empty owner shows the probe colour `(1, 2, 3)` and leaves the stored seed. A mover, Low, detail off with an empty cache, and a miss keep the probes. Detail off with SH cells on, no probe, and a cell the parent and resample bilinear leave out (fractional sample count) shows that cell through the hit read. The same cell marked stale with texel detail on shows the probe. `diag[0]`, `diag[1]`, and `diag[2]` are displaying-compose skip, evaluate, and probe-irradiance counts for that one dispatch. Maintenance does not add. `cells_only` is not what this fixture counts: `covers()` ignores that toggle. The dispatch does not present a swapchain image. Zero presents is not a full frame. The room-blotch harness is unchanged: it does not set `sh_texel_detail`, and its 0.014 gate stays parked.
