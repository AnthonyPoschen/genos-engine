# Moving and changing lamps (design, for review)

Status: **Proposed** (18 Nov 2026). Design only. No shader, no setting, no renderer change in this PR. Nothing here is accepted until Anthony Poschen reviews it.

Anthony, 18 Nov 2026: scale the response to the size of the change. A slow-moving lamp has immediate direct and shadow impact and little GI impact. A large colour or brightness change has immediate GI and direct impact. The hard part is telling the right world-cache cells, giving them the right number of new rays, and dropping the right old samples, without a pop.

Picture rules that stay: the path-traced reference is the grade, no look hack, no flicker and no pop, near-camera error matters most. Lighting VRAM stays near the ~200 MB soft budget. The stress scene target stays about 100 fps on an RTX 4070 at 1440p (10 ms). This design does not claim that target. It adds no pass.

Screen probes are gone (ADR 0039). "Rays" below are world-cache fill rays. They are not probes.

## 1. What already does this job

| Piece | What it does today | What this design changes |
|---|---|---|
| Lamp ring (`lighting_wrc_note_lamps`) | 16 entries × 10 words: old place, new place, `max(old range, new range)`, brightest channel, frame, keep. Keep is 1 when the lamp stays within 1 mm and only colour or range changes, and 0 when it moves or goes out. | Same ring. Word 9 stops being a 0/1 keep and stores the radiance magnitude below. |
| `lighting_lamp_delta` | At a point, sums `brightest × \|1/d²_new − 1/d²_old\|` for ring entries whose ball reaches the point. Callers only test `≥ 0`. A colour change with no move sums to about 0. | Callers use the number. The colour term below is added, because a hue or brightness change has no geometric delta. |
| `lighting_lamp_noted` | True for 120 frames after any note, ignoring reach. While it is true, every world cell's fill commit sets `blend = 1` (`lighting_fill_commit`). | Deleted as a picture switch. A cell outside both balls is not a lamp change. |
| F2 fill | One ray per chosen origin, deposited that frame, capped by `fill_budget`. | Unchanged. |
| F3 water-fill | `priority = closeness × (remaining error + change_weight × lambda)`. Lambda is the same-seed check, then it decays. | Lambda also takes the cell's lamp error, defined below. The formula stays. |
| F4a | A low-confidence seed into other cells of the same screen tile. Not an own sample. A share from before the lamp stamp stays home. | Unchanged. |
| Direct history, [PR #292](https://github.com/AnthonyPoschen/genos-engine/pull/292) | Header word 37 is the previous ball, word 38 is the changed-lamp bitset, word 39 is "any lamp changed" and only an early-out. History drops only where the previous or the current range ball reaches the shaded point (`pos + n × 0.02`). The mean keeps 12 samples (new weight 1/13), shorter when the unshadowed target steps by 8 log codes (~9%) or visibility flips. Range 0 is an empty ball, so colour-to-0 fades the previous ball and does not touch pixels that ball missed. | Left as it is. Direct is not redesigned here. |
| Mover texels, [PR #288](https://github.com/AnthonyPoschen/genos-engine/pull/288) | An object-attached pool and an O(1) mover table. Moving geometry does not salt the world hash and does not build a hitch list of overlapped cells. | The lamp analogue: the ring is the record. Cells ask the ring. No list of dirty cells. |
| [Localized invalidation](localized-invalidation-design.md) | A lamp change record is old and new position, intensity, and reach. Priority uses `Δ(I/d²)` inside reach. A lamp move raises priority. It does not flush the lamp. | This is that rule, with a magnitude, on the fill that replaced the old scheduler. |
| ReSTIR DI ([plan](restir-di-plan.md), ADR 0039 D2) | One reservoir per pixel. Off is the labelled exact sum. `LampShadeGate::Exact` stays that sum. World-cell light lists stay out. | No second direct estimator. |

The binary note is the bug. A 1 mm move and a colour flip both set the ring, and the noted flag then replaces every world-cell mean for 120 frames. Direct light already stopped doing the screen-wide version of that (PR #292). GI has not.

## 2. Per-lamp magnitude

Computed once per changed lamp, in the note that already writes the ring. Two numbers in `[0, 1]`.

`span = max(range_prev, range_now, 0.25 m)`

`m_move = clamp(|p_now − p_prev| / span, 0, 1)`

Radiance uses Rec.709 luminance `Y` of `color.rgb`, and the max channel, so a hue swap with the same luminance still counts:

`m_rad = max( |Y_now − Y_prev| / max(Y_now, Y_prev, ε) , max_c |c_now − c_prev| / max(|c_now|, |c_prev|, ε) )`

Range to 0, or a new lamp, is `m_rad = 1`. A range shrink uses the radiance term on the way out and the geometric term for the shell that left the ball.

GI does not treat a creep as a teleport:

`m_gi = max(m_rad, m_move²)`

| Event | `m_move` | `m_rad` | `m_gi` |
|---|---:|---:|---:|
| 5 cm step, range 8 m | 0.006 | 0 | 0.00004 |
| 2 m step, range 8 m | 0.25 | 0 | 0.06 |
| Jump by a full range | 1 | 0 | 1 |
| Colour flip, same place | 0 | 1 | 1 |
| Brightness ×2, same place | 0 | 0.5 | 0.5 |
| 1.5% dim, same place | 0 | 0.015 | 0.015 |

`m_gi` is the lamp's own size. The cell still scales it by how much of that lamp it actually sees (next section). A slow lamp can matter on the surface next to it, where `1/d²` moves a lot, and nowhere else.

Below `1/64` at the cell, the change is one code and the cell is left alone. That is the cheap check. A still lamp writes no ring entry, as today.

Direct does not read `m_gi`. Its response is the PR #292 ball test and the 12-sample cap. `m_move` and `m_rad` are stored so a dump can show why a lamp was noted. Word 9 holds `m_rad`. `m_move` is recovered from the two positions and the range already in the entry.

## 3. Which cells

A cell asks the ring. The ring does not walk the cache. Sixteen entries, the same walk `lighting_lamp_delta` already does. This is the mover-pool rule (PR #288): an O(1) record, no hitch list of overlapped cells. PR #288's table stores only the current sphere; the lamp ring already stores both places, and both are used.

The influence volume is the union of the previous and current range balls, the same union as PR #292. The test is the cell's bounds, not only its centre: expand each ball by half the cell's own size (25 cm world cell, 12.5 cm fine child). A coarse cell is included when its box meets the ball. A fine child uses its smaller box. A lamp change does not create a finer cell and does not change cell size (S57).

A cell outside both expanded balls has lamp error 0, whatever `m_gi` is.

At a cell centre `p`, for each ring entry that reaches it and is newer than the cell's mean commit:

`geom = brightest × |1/d²_new − 1/d²_old|` (today's term; 0 outside that entry's ball)

`colour = m_rad × max(1/d²_new, 1/d²_old) × Y_now` (0 outside the ball; this is the term a pure recolour is missing)

`expected = sum of (geom + colour)` over those entries

`lam = clamp(expected / max(cell luma, ε), 0, 1)`

`lam` is the fraction of the cell's stored light that this change can move. It is the weight F3 already multiplies by `change_weight`.

Level of detail is closeness, which F3 already has: `1 / max(distance, prox_min)`. Near cells win the budget. Far cells inside the ball keep the same `lam` and lose the tie. They are not given a second, smaller ray count on top of that.

Bounce past the ball is not a second volume. A big colour change updates the cells the lamp lights directly, on this frame's budget, nearest first. The next fill ray that hits one of those cells reads the new value (the cache read at the hit, already the bounce). Surfaces the lamp never reached move when that read changes, not because the ring marked the whole room. The old comment that bounce reaches past the range, so every texel is due, is the line this design removes. If a colour flip still leaves near bounce late after the gates in section 7, the follow-up is the fill design's longer paths (F5), not a bigger ball.

More than 16 lamps changing in one frame already collapses to one infinite-range stand-in. That stand-in keeps `m_rad = 1`, so an overflow is a full response, which matches today's "unknown amount".

## 4. How many new rays

F3 water-fill is unchanged:

`priority = closeness × (remaining error + change_weight × lambda)`

When the ring reaches the cell and the mean is older than that entry, `lambda = max(check lambda, lam)`. The existing decay then drops it as new rays agree. A cell with `lam < 1/64` does not take this max.

The budget stays `fill_budget` (a ceiling, not a target). The spread-out floor stays, so a quiet wrong cell is still found. No new dispatch.

What that buys:

- A slow lamp raises `lam` only where `Δ(1/d²)` is real, usually the near shell. Those cells outrank a settled neighbour by a little. They take a few rays. The rest of the cache does not.
- A colour flip sets `lam` near 1 on cells inside the ball. Closeness spends the ceiling on the near ones until their remaining error falls, then the budget idles back to the floor.
- A 2× brightness step sits between the two. Owed work is about half a full refill of the cells in the ball, still capped, nearest first.

F4a may seed other cells in the same tile from a path traced after the stamp, under the gates it already has. That is how a tile moves together without a ray per cell. A low `lam` cell does not need the seed. A shared seed still does not count as an own sample.

The texel ray cut (ADR 0039) today treats any reaching lamp change as a miss, so a 1.5% dim cannot freeze. Under this design the miss follows `lam`, not the boolean. `lam < 1/64` may hold. `lam` at or above that drops the hold so the texel takes new rays. `settle_dim` (1.5% and 10%) still has to reach the new level. If the 1.5% row stays at the old brightness, the floor is too high and the change does not ship.

## 5. Old samples, without a pop

Nothing is zeroed. The stored mean stays the published value (only finished values).

Replace `if (lighting_lamp_noted()) blend = 1` with a keep that matches the direct 12-sample fade at a full change, and matches the ordinary fill when nothing changed:

`kept = lerp(ray_cap, 12, lam)`

`blend = max(ordinary pace, n / (kept + n))`

| `lam` | Kept samples | What the picture does |
|---|---|---|
| `< 1/64` | the full cap | no lamp work |
| small (slow move, or a few percent of brightness) | close to the cap | the mean steps by about that fraction; no extra fade |
| ~0.5 | halfway toward 12 | partial replace, still one MSME step |
| ~1 (colour flip, lamp off, jump across the range) | 12 | first new sample weighs 1/13, the same step as direct history in PR #292 |

A full change does not cut to one ray and does not cut to black. The first frame moves by about a thirteenth of the gap, then the water-fill supplies the rest. A cell revealed later runs the same test against its old commit, so it does not show pre-change light.

Direct and GI stay split on purpose:

| Event | Direct (PR #292, unchanged) | GI (this design) |
|---|---|---|
| Slow move | Immediate inside the union of the balls. 12-sample history. Fresh shadow rays, near the camera first, inside the existing direct budget. | `m_gi` tiny. A few rays where `Δ(1/d²)` exceeds 1/64. Mean kept. |
| Fast move or a jump | Same balls. The cap shortens when the unshadowed target steps by 8 log codes or the shadow flips. | `lam` from the geometric term, up to 1 in the shell the lamp entered or left. 12-sample keep at the high end. |
| Colour flip | Word 38 bit. Fade inside the current ball. Colour-to-0 fades the previous ball only. | `m_rad = 1`. `lam` near 1 inside the ball. 12-sample keep. Nearest cells first. |
| Brightness ramp | Same fade. A step under ~9% may not clear the mean. | `lam` equals the relative step. 1.5% stays under the floor. A large step follows the colour row, scaled. |

Pixels and cells outside both balls keep their history on both paths. Direct does not wait for GI. GI does not flush because word 39 was set.

## 6. Memory and per-frame cost

No new buffer. The ring is already 16 × 10 words. Word 9 changes meaning (`m_rad` instead of 0/1 keep). That is 0 MB against the ~200 MB soft budget.

Per frame, on the path that already runs:

| Work | Cost |
|---|---|
| Note | One pass over the lamp list on the cache thread that already notes. A still scene writes nothing. |
| Cell query | The 16-entry walk already inside the fill. A few more arithmetic ops. No live-table scan. |
| Rays | Still `≤ fill_budget`. A colour flip spends that ceiling on near cells in the ball, then stops. A slow lamp spends the floor plus a thin near shell. |
| Direct | Unchanged. The bitset walk runs only when word 39 is set (PR #292). |

Estimates, not measurements. A 5 cm step on an 8 m lamp is under the GI floor except beside the lamp: on the order of the spread floor, not a refill. A colour flip is bounded by `fill_budget`, not by the old settle list. The 1,470,464-ray hold spike in the [fill design](screen-tile-fill-design.md) (flag off, lamp 3, 11 Nov) is the behaviour this removes.

No new pass, so the settled lighting cost is not moved. The 11 Nov 4070 hall was about 15.2 ms of lighting at 1440p with no cache rays, already over the 10 ms stress frame. Direct, compose, and the denoiser still own that gap. Lavapipe is the correctness check. It is not a frame time.

## 7. What is deleted, and what stays

Deleted only after the gates in section 8 pass, in the implementing PR, not in this one.

| Superseded | Replaced by |
|---|---|
| `lighting_lamp_noted` forcing `blend = 1` on every world cell | per-cell `lam`, keep lerped to 12 |
| Keep as 0 or 1, with any move past 1 mm meaning "drop it all" | `m_rad` in word 9, and `lam` from section 3 |
| Whole-cache settle for 120 frames while any ring entry is live (`lighting_lamp_settling` listing every cell; the "bounce past range, every texel is due" rule) | cells outside the balls stay on the ordinary fill. The 120-frame window only keeps the ring entry readable for a cell visited late |

Left in place: the ring and `lighting_lamp_delta`'s geometric term, F2, F3 (formula, decay, ceiling, spread floor), F4a, PR #292 words 37/38/39 and the 12-sample direct fade, the PR #288 mover pool, world-cell light lists staying out, `LampShadeGate::Exact`.

The 12 Oct [dynamic perf plan](dynamic-perf-plan.md) is an older frame (probes still in the picture, ReSTIR shelved). Item B there (do not re-trace every hit shadow because a lamp epoch moved) is the same intent. This note is the world-cache side. It does not reopen that plan's dispatch list.

## 8. Gates

Lavapipe first, headless, one lamp in the hall, error against the path-traced reference with the existing near weight `1 / max(distance, 0.5 m)`. Frame-to-frame pop uses the metric the settle scripts already write. The 4070 run is a later session, after these pass. Frozen picture gates do not move (blotch 0.014, cell energy 0.9, linger 0.05, lag 0.9, turn 0.403, contact / corner / thin wall).

| Case | Pass |
|---|---|
| Slow move. Displacement per frame ≤ 1% of range, held for 120 frames. | GI near error stays within +0.01 of the still-lamp curve. Direct matches PR #292: no salt outside the balls, wake grain under 1.2. Pop ≤ 1.2× the frozen baseline. Cache rays stay at the still-scene fill, plus the spread floor. |
| Fast move. The PR #292 walk (0.9 m/frame) or a jump by about one range. | Direct: the PR #292 gates (wake grain, and `ref_err` 0.05 by frame 160 when the lamp leaves). GI: near error inside the balls reaches the still floor within 120 frames. Outside the balls, the picture matches the still frame within noise. No frame steps by more than the 12-sample fraction (~1/13 of the gap). Peak rays ≤ `fill_budget`. The 1.47M spike does not return. |
| Colour flip, lamp stays. | Inside the ball, direct and GI both move on the note frame: error against the new reference starts falling immediately, and the first step is ≤ 1/13 of the gap. Outside the ball, unchanged within noise. |
| Brightness. `settle_dim` at 1.5% and 10%, plus one step to ×2 and one to 0. | 1.5% does not lift `lam` over 1/64 and still reaches the new level (the dim is not frozen). 10% and ×2 follow the colour row, scaled by `m_rad`. Colour-to-0 matches PR #292 on direct and the full-keep row on GI inside the previous ball only. |

Knockout, so a wired no-op fails: force `lam = 1` on a 1 mm move and the slow-move GI row goes red. Force `lam = 0` on a colour flip and the colour-flip row goes red.

## 9. Probes, names, networking, departures

Failure modes and the probe for each, on `debug/lighting` when this is implemented. No new probe in this PR.

| Failure | Probe |
|---|---|
| A creep refills the cache | rays this frame against the still fill; `lam` heatmap |
| A colour flip does not reach the near cells | `lam` inside the ball, near error vs the new reference |
| A pop on the first new sample | frame-to-frame near delta, ceiling ~1/13 of the gap |
| Salt or a frozen mean outside the balls | the PR #292 grain tests; GI crop outside the balls |
| A 1.5% dim frozen by the ray cut | `settle_dim` |
| Noted flag still replacing every cell | `blend` max with the noted bit forced; the knockout above |

Tunable, one setting when implemented, not in this PR: the `1/64` floor, CLI argument, env mirror only when the argument is omitted, MCP, debug menu (`docs/engine/settings-principles.md`). `fill_budget` and `change_weight` stay the knobs they are. No new public name beyond the glossary line below.

Glossary, to land with the implementing PR, not this one: **Lamp change magnitude** (SCHED). The per-lamp pair `m_move` (position delta over range) and `m_rad` (relative colour and brightness). It scales GI keep and fill lambda. A Virtual Shadow Map invalidates every cached page of a moved light; this does not. The change record in the glossary already has old and new position and reach. This is the scale on that record.

Networking:

- Replicated state: none. The ring and the cell means stay on the client that shades.
- Authority: that client.
- Prediction / rollback: none.
- Determinism: the note is a function of the lamp snapshot. `lam` is a function of the cell centre, the cell size, and the ring.
- Headless server: a server that does not shade does not note lamps and does not fill.

Departures, for the lighting README when the code lands, not in this PR:

- UE Virtual Shadow Maps: a moved or recoloured light invalidates all of that light's pages. We raise `lam` inside the union of the balls, in proportion to `Δ(I/d²)` and `m_rad`. ADR 0013 already chose priority over a flush. The departure from ADR 0013's rev 2 is the keep count (12 at a full change, the full cap at none) instead of a priority-only nudge.
- The current shader: any note sets `blend = 1` on every world cell for 120 frames. Replaced by section 5.
- ReSTIR DI (Bitterli et al. 2020): no change. The temporal cut is the PR #292 reach test, already named there.
- A-SVGF (Schied, Peters, and Dachsbacher, HPG 2018, eq. 7): the quiet-field step in ADR 0039 stays. This design does not add a screen-space fade on top of the cache mean.

Out of scope: the sun, the flame, mover-versus-lamp shadow volumes (the invalidation design and the PR #288 pool), pitch and roll on movers, and any claim on the 100 fps stress target.
