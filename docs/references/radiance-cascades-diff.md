# Radiance cascades: paper versus our field

The gather now follows section 8. This note is the record of the gap before that change.

This note is for the person who will change the gather. The picture is the GPU gather in `crates/render/shaders/light.comp` and the shade in `crates/render/shaders/scene.frag`. The CPU field in `crates/render/src/field.rs` is the older check. It is not the picture.

Names used here are the ones in `CONTEXT.md`. A radiance field is the light split into distance ranges. A radiance cascade is one of those ranges. A world probe is our farthest range. A ray is not a cascade.

## 1. Sources read

Opened and used:

- Working paper source, Alexander Sannikov, *Radiance Cascades: A Novel Approach to Calculating Global Illumination* [WIP]. <https://github.com/Raikiri/RadianceCascadesPaper/blob/main/RadianceCascades.tex>. Downloaded `RadianceCascades.tex` (718 lines). Equation labels below are the tex labels. The PDF numbers are different.
- Working paper PDF, <https://github.com/Raikiri/RadianceCascadesPaper/blob/main/out_latexmk2/RadianceCascades.pdf>. Downloaded. 36 pages, dated 28 Apr 2025 in the file metadata. Page numbers below are that PDF.
- Osborne and Sannikov, arXiv:2408.14425, <https://arxiv.org/abs/2408.14425> and <https://arxiv.org/pdf/2408.14425>. The abstract page and the HTML were opened. The HTML fetch was cut off in section 2.3. The PDF downloaded (21 pages). Citations below are the PDF.
- Index <https://radiance-cascades.com/>. It lists the two papers above, the holographic paper (not used), and Yaazarai’s GM Shaders notes. A Shadertoy on that page is not used as a source.
- Yaazarai, *GM Shaders Guest: Radiance Cascades 2*, <https://mini.gmshaders.com/p/radiance-cascades2>, 13 Jul 2024. Opened for the interval-overlap formula. The working paper does not contain that formula.
- Our decision: `docs/adr/0008-lighting-uses-radiance-cascades.md`.
- Our notes: `docs/lighting.md`, `docs/systems/lighting.md`. Where those notes disagree with the shaders, the shaders win. The disagreements are marked below.
- Words: `CONTEXT.md` entries Radiance field, Radiance cascade, World probe.

Tex sections used: Introduction (no previous frame), The Penumbra Condition, The Penumbra Hypothesis, Radiance Interval, Merging radiance intervals, Radiance cascades, Calculating indirect lighting, Analysis, Radiance 3d, Screen space probes with world space radiance intervals, Limitations.

## 2. The paper’s interval

Sannikov defines the opaque interval, PDF p. 11 eq. (7), tex `eq:def_radiance_opaque`. Only the segment `p + t ω̂` with `t ∈ [a, b]` contributes. `R` is the nearest opaque surface.

```
L_{a,b}(p, ω̂) = L_out(R(p + a ω̂, ω̂), −ω̂)   if |R(p + a ω̂, ω̂) − p| < b
              = 0                               otherwise
```

The same page, eq. (9), sets the transparency of that segment to 0 on a hit and 1 on a miss. The volume form, eq. (8), integrates emission with optical depth. We do not use it.

Osborne writes the same object as a pair, PDF p. 4 eq. (9): `R_{a,b}(p, ω̂) = [I_{b→a}, τ_{b→a}]`. The merge, eq. (11), is `I_{c→b} + exp(−τ_{c→b}) I_{b→a}`. A miss is not a second copy of the near color. The farther intensity is multiplied by the transmittance of the near segment.

Our `gather_interval(origin, dir, t0, t1)` in `light.comp` lines 430–497 has the same shape and extra terms:

```
gather(origin, dir, t0, t1) =
    black, marked hit                         if the origin is inside a solid
    surface(hit) + floor_along(t0, t_hit)     if an occluder has t in [max(t0, 0.002), t1)
    floor_along(t0, t1), marked miss          otherwise
```

`surface` is the hit albedo times arriving light (`light.comp` lines 484–494). Arriving light is `lights_at` plus, after pass 0, the previous pass of this build (`light.comp` lines 478–481). `lights_at` is the analytic lamps plus the fire (`light.comp` lines 234–244).

Terms the opaque interval does not have:

- `floor_along` (`light.comp` lines 399–422). On pass 3 only, two samples of floor emission are averaged into the interval. A miss in eq. (7) is 0. This term is not 0.
- The 0.55 / 0.68 shares, or a material reflectance (`light.comp` lines 489–494). The paper’s `L_out` is the rendering equation, not these constants.
- A 2 cm push along the hit normal before the lamp test (`light.comp` line 477).
- An origin inside a solid forced to a black hit (`light.comp` lines 431–432). The paper’s interval is not defined that way. The effect matches a hit with `L_out = 0`, so the farther range is dropped. That part is in the spirit of β = 0.

Terms the interval has that we drop:

- Transparency β, or Osborne’s optical depth τ. We store a boolean `found` and then throw the direction away. Nothing in the texel is β. Alpha is 1, or 0 only as a debug sentinel (`light.comp` lines 537–539).
- The direction. Eq. (7) is a value at one `ω̂`. We return the color and immediately average it (`light.comp` lines 529–537).

The three ranges meet and do not overlap: `[0, near_end]`, `[near_end, far_end]`, `[far_end, world_end]` (`light.comp` lines 502–511). That matches Sannikov PDF p. 14 eq. (18): cascade `i` is `[t_i, t_{i+1}]`, `t_0 = 0`, and the next starts where the previous ends. Yaazarai’s notes add a small overlap of the next probe spacing “to fix light leak” (`radiance-cascades2`, the `range += length(...)` line in `cascadeTexelInfo`). The working paper does not contain that overlap.

## 3. Cascade hierarchy

The penumbra condition is Sannikov PDF p. 9 eq. (5): spatial step `Δ_p` scales under a linear function of distance `D`, and angular step `Δ_ω` scales under a linear function of `1/D`. The hypothesis, same page, section 2.2, says a discretization is representative at every scale when it satisfies that condition. Osborne PDF p. 4 states the same split: near contributions need high spatial and low angular resolution; far contributions need the reverse.

The scaling that satisfies it is Sannikov PDF p. 15 eq. (19), tex `eq:cascade_scaling`:

```
Δ_p ∼ 2^i
Δ_ω ∼ 1/2^i
t_i ∼ 2^i
```

Osborne PDF p. 5 eq. (12) is the same law with a branching factor `α ≥ 1`. Spacing still doubles. Angular samples and interval length scale as `2^{α i}`. Their examples use `α = 1` in figure 4 and `α = 2` in the runs. `α = 2` is a 4× jump in angle and in interval length. Sannikov’s Radiant 2d section says that demo used 4× angular resolution instead of the 2× in the flatland section, and that this was a departure from the written law.

Each cascade is its own uniform grid. Cascade `i + 1` probes are twice as far apart as cascade `i` (Sannikov PDF p. 15). Directions are uniform inside a cascade (PDF p. 14: “uniformly distributed samples in both space and direction”). The upper rays run until they leave the domain (Osborne PDF p. 5, under eq. (13)).

The GPU picture is not that hierarchy.

- One grid, 128 by 128 (`light.comp` line 58, `pack.rs` line 21). Not one grid per range.
- Positions come from `probe_axis` over the floor span only (`light.comp` lines 94–117). The density is `1 + 3 exp(−|x − player| / 3)` (`probes.rs` lines 14–16 and 130–131). `DENSITY_RATIO` is 4 and `DENSITY_FALLOFF` is 3 meters. The same constants are in `scene.frag` lines 109–110. This is one moving non-uniform grid. It is not three uniform grids.
- 64 directions at every range (`light.comp` lines 531–534). The far range is not finer in angle.
- The three intervals are walked from that same probe (`light.comp` lines 502–511). The far interval does not move to a coarser lattice.
- `near_end` is `max(span * 0.12, spacing * 4)`, `far_end` is `max(span * 0.85, near_end * 2)`, `world_end` is `max(span * 4, far_end * 2)` (`pack.rs` lines 609–611). The “about 12%” figure is the first term only. On a small floor the four-cell term wins. The lengths are not `t_i ∼ 2^i`.

The penumbra hypothesis is not satisfied by this grid. Spatial step follows distance from the player, not distance along the ray. A contact interval far from the player is coarse in space. A far interval next to the player is still one of 64 directions on the fine lattice. Eq. (19) requires the opposite: the far cascade is coarse in space and fine in angle.

The CPU field is a different builder, and the picture does not use it. It does store three uniform grids (`field.rs` lines 372–397). Near spacing is 0.16 m with 12 directions. Far spacing is 0.48 m with 36 directions. World spacing is 1.28 m with 64 directions. The order matches the hypothesis: nearer means more positions and fewer directions. The factors do not match eq. (19). Spacing goes 1×, 3×, 8×, not 1×, 2×, 4×. Directions go 12, 36, 64, not a doubling. The intervals use the same `0.12 / 0.85 / 4` split as the GPU (`field.rs` lines 376–394). `docs/lighting.md` says the CPU field keeps fewer directions in the near range than in the far range, and fewer in the far range than in the world range. That sentence matches `field.rs`. It does not describe the picture.

`docs/lighting.md` also says near spacing is at most half the far spacing. The shader constant is a density ratio of 4, so the cell beside the player is about a quarter of the far cell, not a half (`probes.rs` lines 14–16).

## 4. Merge

Sannikov PDF p. 12 eq. (11), tex `eq:merging`:

```
L_{a,c}(p, ω̂) = L_{a,b}(p, ω̂) + β_{a,b}(p, ω̂) L_{b,c}(p, ω̂)
β_{a,c}        = β_{a,b} β_{b,c}
```

A hit has β = 0, so the farther interval is dropped. A miss has `L_{a,b} = 0` and β = 1, so the farther interval replaces the miss. Eq. (13) on the same page repeats this down the chain, from `t_0 = 0` out to infinity. Full incoming radiance is that merge, eq. (12).

The farther interval is not cast from the same probe. PDF p. 14 says a cascade that meets the penumbra condition can be linearly interpolated in space and in direction, and that this is why a cascade does not need a disocclusion fix. PDF p. 19 says the interval of cascade 1 is interpolated onto every probe position of cascade 0, and onto the finer angles, and then merged. Figure 10 on that page is the picture of that merge. Osborne PDF p. 7, section 2.5, names the four-probe version: bilinear weights of the four probes of cascade `i+1` in the same cone. The bilinear fix in that section is not a blur. It retraces the near ray to each of those four ray starts so parallax does not double-count a source. The working paper does not contain the bilinear-fix formula. It does contain the interpolation claim on PDF p. 14.

`merged_dir` (`light.comp` lines 499–511) does not do this.

- Same origin, same direction, three intervals.
- A near hit returns the near color and drops the rest. That matches β = 0.
- A near miss with a far hit returns `far.color + near.color`. The miss color is `floor_along`, which eq. (7) sets to 0. Adding a zero would match the replace. Adding floor light does not.
- A miss of both near and far returns the sum of all three colors. Same add, one range further.
- There is no read of a coarser probe and no read of a neighboring angle. The comment at lines 500–501 says an angle snap drew wedges and a coarser-neighbor blend drew a ring, so both were removed. The paper’s answer to those artifacts is the penumbra spacing plus interpolation, not dropping the interpolation.

The stored texel is the mean of those 64 merged colors (`light.comp` lines 529–537). One `vec4` per probe. The direction is gone before the next pass. Pass `n` reads that average through `sample_bounce` (`light.comp` lines 352–382). The host runs four passes, 0 through 3, and shows the field when the pass counter reaches 4 (`gpu.rs` lines 3035–3039 and 3154–3161). The CPU builder is two passes, direct then one bounce (`field.rs` lines 43–45). The picture is the four-pass GPU field.

`scene.frag` never asks for a direction. `sample_raw` (lines 469–506) bilinear-blends four texels in the density index. `probe_rejected` (lines 450–458) drops a probe inside a solid, and drops a probe whose segment crosses a wall, except when the probe is within `spacing * 1.25`. `spacing` is the fine cell at the player (`pack.rs` lines 605–606, `probes.rs` lines 69–71), not the local cell. Inside that radius the wall test is skipped on purpose. The comment says rejecting the neighbor left a black ring.

What that does to a colored bounce: the bounce is no longer “the color in this direction.” It is a blend of nearby averages. Two probes that saw different surfaces produce a third color that neither surface emitted. The paper’s interpolation is legal only because each stored sample is one interval at one angle, which is smooth in space (PDF p. 14). An average of unrelated hits is not that function, so the bilinear blend invents mixed colors.

What that does to a fire behind a wall: a probe on the fire side of the wall has the fire in some of its 64 directions, so the stored average is red. A hallway sample within 1.25 fine cells of that probe accepts it (`scene.frag` lines 456–458). The red average is blended into the hallway. Eq. (11) would not do this. The hallway direction that hits the wall has β = 0 and never reads the fire-side cascade sample. The paper says the disocclusion fix is not required when the stored value is an interval (PDF p. 14). We store the merged average, then apply a fix, then disable the fix for the near neighbor.

The CPU merge is also not eq. (11). `merge_at` (`field.rs` lines 503–527) picks the nearest probe, not the bilinear four, and snaps the angle with a floor (`field.rs` lines 534–537). A dark near direction is replaced by the far color times 0.45, or the world color times 0.2. Those scales are not in the paper. `sample` then bilinear-blends the already merged near color (`field.rs` lines 60–66 and 475–500). The picture does not run this path.

## 5. Off-screen world light

The papers do not say “store a world probe outside the floor.” They do say that off-screen light reaches the screen only when the intervals are marched in the world.

Sannikov PDF p. 34, Limitations: light outside the view casts no indirect light in the Path of Exile 2 form, because that form marches screen-space data. The same paragraph says the loss is from the march, not from storing the cascades in screen space. Screen-space cascades can store world-space intervals, and that would remove the screen-space artifacts. PDF p. 28, Radiance 3d, shows indirect light from off screen and says it is possible because the march is in the world. PDF p. 31, section 4.5, says world-space intervals let an off-screen emissive or directly lit surface light geometry on the screen. Osborne PDF p. 5: the upper cascade extends until the ray leaves the simulation domain, so the domain boundary is in the field. The domain there is the model, not the camera.

So the papers require world-space intervals over the volume that can send light, including light the camera does not see. They do not require a screen-space march. A ray that starts on screen and hits an off-screen surface in its interval is enough. A coarser cascade then spreads that sample by interpolation. Rebuilding from scratch every frame is the main-text rule (PDF p. 3). The SDF demo on PDF p. 31 optionally reprojects the previous frame. That reprojection is one implementation, not the interval equation.

Our two builders:

CPU field. World probes are a third grid. The origin is half a floor-span outside the floor on each side (`field.rs` lines 396–415, `margin = span * 0.5`). Those probes sit outside the floor. Their rays start at `far_end` and run to `world_end`. `sample()` does not read them. It reads the near merged color only (`field.rs` lines 60–66). `seal` copies a world direction into that near color when the near and far directions are under `1e-4`, at 0.2 scale (`field.rs` lines 512–520). `sample_world()` reads the world grid (`field.rs` lines 70–72). The picture does not call `sample` or `sample_world`. So the CPU builder does place probes outside the floor, and that light can change `sample()`, and the picture ignores it.

GPU gather. `probe_at` places every probe with `probe_axis` on `[origin, origin + floor span]` (`light.comp` lines 112–116, `probes.rs` lines 45–57). There is no probe outside the floor. `world_end` is a ray length, not a grid (`pack.rs` line 611). A ray can leave the floor and hit an occluder that was packed. Occluders are packed when `affects_light` is set, before the view test (`pack.rs` lines 161–169). The view test drops draws, not occluders. So an off-screen wall that affects light is in the gather. It changes an on-screen pixel only when some floor probe’s ray hits it inside one of the three intervals and the 64-direction average still carries that color. The fragment shader then reads four floor probes around the shaded point (`scene.frag` lines 469–506). It cannot ask for the direction toward the off-screen surface.

Can off-screen light change an on-screen pixel today? On the picture path, only as a diluted share of a floor probe’s average, and only if a floor ray hit that surface. There is no stored world probe outside the floor for a miss to read. The CPU path can put off-floor probe light into `sample()`. The picture does not use that path. The path that drops the paper’s off-screen probe is the GPU gather.

`docs/lighting.md` says the world grid is larger than the floor and that probes sit outside the floor. That sentence describes `field.rs`. It does not describe `light.comp`.

A lamp that is off screen is a separate question. `direct_at` (`scene.frag` lines 284–308) tests the analytic lamps and the fire in 3D, with no screen test. That is not the radiance field. An off-screen lamp can light a pixel that can see it. An off-screen surface that is not a lamp cannot, except through the floor-probe average above.

## 6. Equation diffs

What we drop: β, the per-direction interval at shade time, angular interpolation, and the spatial sample of the coarser cascade. The GPU also drops a separate resolution per range.

What we replace: `L_{a,b}(p, ω̂)` with the mean of 64 merged directions; `β L_next` with “hit returns, miss adds”; the coarser cascade with a longer ray from the same probe; the uniform grids with one player-centered grid.

What we add that neither paper has: `floor_along`; the separate `direct_at` ray; the shade `albedo * (direct * 0.58 + bounce * gain * 0.75)` (`scene.frag` line 737); `overhead_tint` (`scene.frag` lines 325–364); the 1.25-cell acceptance; the lamp peak and tail (`light.comp` lines 227–231); holding the finished field until the player or a lamp moves 1 m, or a lamp color, an occluder, or the rest of the light hash changes (`pack.rs` lines 24, 1458–1478, and 1489–1518).

| Paper term | Our term | What the picture does wrong |
| --- | --- | --- |
| Interval radiance `L_{a,b}(p, ω̂)`, one color per direction. Sannikov PDF p. 11 eq. (7). Osborne stores `(I, τ)` per direction, PDF p. 4 eq. (9). | One `vec4` per probe: the mean of 64 directions, each already merged across all three intervals (`light.comp` lines 529–537). The fragment shader has no direction (`scene.frag` lines 722–724). | A direction that sees the fire, or a colored bounce, tints the whole probe. The pixel cannot pick the direction that actually hits. |
| Adjacent non-overlapping intervals, merged by `L + β L_next`. A miss is 0 with β = 1, so the next interval replaces it. Sannikov PDF p. 12 eq. (11) and p. 14 eq. (18). | The same three endpoints, non-overlapping (`light.comp` lines 502–511). A hit returns and drops the rest, which matches β = 0. A miss adds `floor_along` onto the farther hit (`light.comp` lines 507–511). β is not stored. | The replace becomes an add whenever the empty interval’s floor term is non-zero. A dark miss no longer means “use only the next range.” |
| Angular interpolation. The coarser cascade’s directions are interpolated onto the finer angle. Sannikov PDF p. 14 and p. 19. Osborne PDF p. 7 bilinear weights inside the cone. | The same 64 angles at every range. No neighbor angle is read (`light.comp` lines 500–511). The CPU snaps with a floor (`field.rs` lines 534–537) and the picture does not use that either. | A bounce that should slide across angles pops between probes, then gets smeared by the spatial blend of averages. |
| Spatial bilinear of the coarser cascade, in the missed direction, at merge time. Sannikov PDF p. 19, figure 10. | No coarser lattice. After the average, `sample_raw` bilinear-blends four probes of that average (`scene.frag` lines 469–506), and `probe_rejected` can drop a probe across a wall unless it is within 1.25 fine cells (lines 450–458). | The blend mixes full averages, including light from the other side of a nearby wall. The paper blends one interval in one direction, which does not cross an opaque hit. |
| Separate resolutions. Cascade `i` has spacing `∼ 2^i`, angle `∼ 1/2^i`, interval `∼ 2^i`. Sannikov PDF p. 15 eq. (19). | One 128² grid. 64 directions at every range. Interval ends are fractions of the floor span (`pack.rs` lines 609–611). Spacing follows the player (`probes.rs` lines 14–16). | The grid does not meet the penumbra hypothesis. Contact detail far from the player is coarse. Far light next to the player has no extra directions. |
| World-space probes of the far cascades, over the domain, not over the camera. Upper rays leave the domain. Sannikov PDF p. 28 and p. 34. Osborne PDF p. 5. | GPU probes cover the floor span only (`light.comp` lines 112–116). The CPU world grid does extend half a span past the floor (`field.rs` lines 408–415), and the picture does not read it. | An on-screen miss cannot read a probe that sits off the floor. Off-screen surface light is dropped unless a floor ray happened to hit it and the average kept the color. |
| No previous frame. Built from scratch every frame. Sannikov PDF p. 3. A build does not reuse radiance samples from the last frame. The SDF section on p. 31 is an optional reprojection, not the equation. | A gather does not read the previous frame. Pass `n` reads the other half of this build (`light.comp` lines 348–350 and 480–481). We match that. We do not rebuild every frame. `FieldAnchor` keeps the finished field until the player or a lamp moves 1 m, or a lamp color, an occluder, or the rest of the light hash changes (`pack.rs` lines 1458–1478 and 1489–1518, `gpu.rs` lines 223–226). | The gather input matches. The held field does not match “every frame.” It is not temporal accumulation inside an interval. |
| No ambient fill. A miss is 0. Sannikov PDF p. 11 eq. (7). Ambient occlusion and environment maps are the two extremes the cascades replace, not a constant added in the dark (PDF p. 5). | No ambient term. `field.rs` lines 74–76. A probe with no hit stores black (`light.comp` lines 525–527). An empty `sample_raw` returns black (`scene.frag` lines 502–504). We match the “no fill” rule. | The match is real for a constant fill. `floor_along` and `overhead_tint` are still extra light the interval equation does not have. They are not an ambient constant. |

## 7. Which mismatches explain the failures

The picture was not re-rendered for this note. The mappings below are from the shaders. A line that is labeled guess is not something the code proves by itself.

### Red dots of fire inside a hallway

Evidence. The fire is one more lamp in `lights_at` (`light.comp` lines 240–243). A probe that sees a fire-lit face in any of 64 directions stores a red average (`light.comp` lines 529–537). `scene.frag` has no direction, so every pixel that accepts that probe gets the red (`scene.frag` lines 722–724). `probe_rejected` does not apply the wall test when the probe is within 1.25 fine cells (`scene.frag` lines 456–458). A fire-side probe that close to a hallway sample is blended in. Eq. (11) would have dropped the farther interval on the wall hit. The 1.25 exception is the line that lets the fire-side average cross the wall.

`direct_at` is a different ray. It returns black when the 3D wall blocks the lamp (`scene.frag` lines 267–275 and 284–308). A smooth red wash from the analytic fire is that ray, and only where the ray misses the wall. The systems note says a ray that clears the top of a wall still arrives (`docs/systems/lighting.md`). That path is smooth, because it is per pixel.

Guess. The word “dots” fits the probe averages, not `direct_at`. This note has no frame, so it does not prove the dots are those probes rather than a short wall letting `direct_at` through in spots. The code path that paints fire through a blocking wall is the 1.25-cell blend of averaged probes.

### Splotchy color where only bounced light lands

Evidence. Where the lamp term is 0, the pixel is `albedo * bounce * gain * 0.75` (`scene.frag` line 737). On an upward face `gain` rises to 2 as the direct term falls (lines 728–733), so bounce-only floor is the loud case. `bounce` is the bilinear of four direction-averages (`scene.frag` lines 469–506). Those averages mixed every hit the probe saw. Neighboring probes saw different first hits, so the blend is a color neither hit emitted. The next bounce pass reads that blend, not a direction (`light.comp` lines 352–382), so the splotch is the input of the next bounce. The paper’s smooth colored bounce is the interpolated interval at one angle (PDF p. 14). We dropped that interpolation in `merged_dir`.

This is the same missing angular interpolation and the same averaged irradiance row in the table. It does not need a guess to connect the code to a blotchy bounce. Guess: the blotch diameter on screen is one local cell of the player-centered grid. The shader does not record that size.

### Mixed colors that slide, and the density step a few meters out

Evidence. Extra density is `3 exp(−d / 3)` (`probes.rs` lines 130–131). At 3 m from the player the extra has fallen by `1/e`. A few meters from the camera is where the grid changes spacing. The center of that curve is the eye used at the last gather (`light.comp` lines 112–116). The gather is kept until the player moves 1 m (`pack.rs` lines 1462–1465). While it is kept, the scene buffer is not rewritten (`gpu.rs` lines 223–226), so the fragment shader’s eye matches the probes. When the player crosses 1 m, the lattice is rebuilt around the new eye. The mixed colors are blends of those probes (`scene.frag` lines 469–506), so they are tied to the lattice and move when it recenters.

The paper’s spacing changes with cascade index, eq. (19), not with distance from the camera. A world-space cascade does not slide a density step along with the player.

Guess. If the mixed colors slide on every frame, inside the 1 m hold, that is not this rebuild. Inside the hold the lattice stays on the anchor. What still moves on screen is the camera looking at a world-locked step that sits a few meters from that anchor. The code does not move probe positions until the 1 m break.

### On-screen pixels with no off-screen world light

This one is section 5. The short form: the papers require a world-space march, and they let a miss interpolate the coarser cascade. The GPU builder has no probe outside the floor, and the fragment shader cannot query the off-screen direction. The CPU builder has the outside probes and the picture does not read them.

## 8. What a paper match would store

Do not blur the averages. The paper has no blur. The ringing fix in Osborne section 2.5 is a retrace onto the four coarser ray starts, then the same bilinear weights. It is not a screen blur. Yaazarai’s overlap is not in the working paper. Eq. (18) says the ranges meet.

A match stores, for each cascade, a grid of probes and, at each probe, one radiance interval per direction: the `L_out` of the first hit in `[t_i, t_{i+1}]`, or 0, and the β of that segment. Cascade 0 is the fine, short, low-direction grid. Each next cascade has about twice the spacing, about twice the directions, and about twice the interval length, as in eq. (19). The last cascade’s rays start at the end of the previous interval and run until they leave the domain we care about, which includes the region outside the camera and outside the floor (Sannikov PDF p. 28 and p. 34, Osborne PDF p. 5).

Those far probes are the world probes if we keep that name. They are not a longer ray on the near probe. They sit on the coarse grid, including cells the camera does not see and cells outside the floor. The CPU builder’s third grid is the only one of our builders that places probes outside the floor (`field.rs` lines 408–415). A match would not then average the directions and scale them by 0.2.

An on-screen miss in cascade `k`, in one direction, reads cascade `k+1` at that direction. The read is the bilinear of the surrounding coarse probes and the linear blend of the coarse directions that bracket the angle (Sannikov PDF p. 14 and p. 19). The merge is eq. (11): add the coarse sample only multiplied by β. A wall hit has β = 0 and does not read the fire behind it. A true miss has β = 1 and `L = 0`, so the coarse sample replaces it. The coarse sample already contains the off-screen surface if any coarse ray hit it. That is how off-screen world light changes the on-screen pixel. The pixel’s shade then integrates the merged directions. It does not sample a single averaged texel.

The additive distance gradient around a sharp light square is not in either paper. In the picture it is `overhead_tint` (`scene.frag` lines 325–364): within 2.5 m of an occluder footprint, add the occluder’s albedo times the light on its top, times `1 / (1 + dist² * 1.6)`, with a smooth edge and a side fade. Sannikov’s soft edge is the penumbra of an area source under eq. (5), reconstructed by the cascade interpolation. A falloff painted around a square is a different approximation. The lamp’s own peak and tail (`light.comp` lines 227–231) are also not a penumbra. They are our distance curve on the analytic lamp.
