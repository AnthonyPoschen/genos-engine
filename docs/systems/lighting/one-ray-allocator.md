# One ray allocator (design, for review)

Status: **Proposed** (18 Nov 2026). Design only. Replaces `moving-lights-design.md`. Nothing here is accepted until Anthony Poschen reviews it.

Measured on lavapipe, 18 Nov 2026, the frozen fill-knockout harness (frame 40, 320×180, High, hall pose). Spending `fill_budget` from one histogram of every live cell left the late path-reference error at 0.438 on this machine and 0.423 in CI. The same harness with the tile contest and the off-screen bins is 0.200. The gate is 0.213. The tile contest, the off-screen bins, and the 64/16 pour are deleted. The device traces one floor ray for each live cell whose slot index mod N equals the frame mod N, plus `fill_burst` rays spent highest value first. N is `fill_floor_rate` (1, 2, or 4). High is every frame and burst 25000 (Medium 12500, Low 6250). The count is the floor picks plus that burst, not 100,000. `fill_budget` above 0 only turns the pass on. The confidence rule (history weight is one minus the value, and one agreement measure moves that value) is in `crates/fill-ref/src/alloc.rs` and in the device blend. The lamp-note replace, the 1/8 and 1/32 floors, `catch_up = 1`, and the flag-4096 hit wipe are gone. A moved lamp still shortens direct history to the reach fade where its old or new ball covers the point, and a visibility flip or a large luma jump there still drops that mean. The old whole-screen word-39 cut does not. A failed reprojection and a slot whose surface has left (leash, age, eviction) still drop that record.

World mode 3 (the product path) already has a score. The device ray count is the floor rate plus the burst. Direct light stays per pixel.

A new cell starts with a high value, meaning "rays requested". The value falls as the cell converges. A mover, a lamp change, or a newly seen bit of view raises that value. Each frame, rank cells by value times a few weights (closeness, on screen), and walk from the top until the budget is spent. A small reserved slice of that same budget is a random trickle, so a converged cell in range still gets a ray now and then.

A small change does not clear the cell. It lowers the weight of the samples already stored, in proportion to how big the change is, and raises the value by the same amount. New rays blend into what was kept. A full clear is only for data that is actually wrong: the surface is gone or occluded, the pixel is a different surface, or the change is nearly the whole light (lamp off, a colour flip, a jump across the lamp's range).

Cell size stays a function of distance, with the band it already has (High: fine texels from 6 m, dropped past 8 m). Motion does not create, coarsen, or delete a cell. It only raises value. Hysteresis stays: a cell is born inside the inner distance and removed only past the outer one, after it has been unseen. The motion lookahead that slides that band goes away.

The picture does not have to freeze while things are still moving. What it must not do is look like a cold start after a small step.

Screen probes are already gone. "History" below is the world-cache mean and the direct reservoir.

## Who decides rays or refreshes (24)

Live on world mode 3 unless noted. One line: what it decides, where.

1. `lighting_fill_q` ranks a cell: closeness times remaining error plus `change_weight` times lambda. `lighting_fill.glsl`
2. `lighting_fill_visit` keeps one winner per screen tile and one per off-screen bin, and drops a cell past the distance leash. An on-screen fine child is left out. `lighting_fill.glsl`
3. `lighting_fine_spawn` creates four 12.5 cm children when the parent is inside the create distance. `lighting_cache.glsl`
4. `lighting_compact_hold_texel_rays` gives a fine texel no rays when it matches its cell. `lighting_cache.glsl`
5. `lighting_fill_dispatch` splits the budget into check rays, on-screen, far, near, and leftovers. `lighting_fill.glsl`
6. `lighting_fill_push_check` picks check-ray cells by hash, one per `fill_check_rate` block. `lighting_fill.glsl`
7. `lighting_fill_emit_on` / `lighting_fill_emit_off` hand out that split. A lock stops a second grant. `lighting_fill.glsl`
8. `lighting_fill_share_accept` writes a low-confidence seed into other cells of the same tile (F4a). `lighting_fill.glsl`
9. `lighting_fill_commit` replaces every world-cell mean while any lamp is noted. `lighting_fill.glsl`
10. `lighting_fill_pace` shortens the sample window while lambda is high. `lighting_fill.glsl`
11. `lighting_near_change` marks texels inside a mover sphere plus 1 m. The mover stays listed for 240 frames. `lighting_cache.glsl`
12. `attach_movers` (`lighting_sc_texel_world`) stores a mover texel in object space, so it follows the box, and the ray cut skips it. `lighting_cache.glsl`
13. `lighting_reach.comp` chooses the screen tiles the wavelet and compose refresh (mover before/after, a direct step, a hot cell).
14. `lighting_wrc_note_lamps` writes the 16-entry lamp ring. `lighting_cache.glsl`
15. `lighting_lamp_noted` is true for 120 frames after any ring entry, ignoring who the lamp can reach. The settle streak and the drain flag in `lighting_write_args` feed the same switch. `lighting_cache.glsl`
16. `lighting_lamp_delta` measures the light change at a point. Callers only test "any". `lighting_cache.glsl`
17. `lighting_shadow_epochs` decides which lamps drop their cached shadow and texel direct light. `lighting_cache.glsl`
18. `lighting_atrous.comp` denoises dirty pages, or the whole cache once the dirty fraction is high.
19. `lighting_direct.comp` skips a shadow ray on a quiet pixel whose signature matches.
20. `restir_live.comp` picks the pixel's direct sample (temporal and spatial). Header word 39 cuts history.
21. `lighting_direct.comp` `cache_pick` chooses which lamps a cache ray shadows.
22. Young/mature lists, the rate interval, the trickle list, the world-cache refresh, gradient checks, scouts, and `lighting_sc_cap_lists` still exist. `lighting_cache_batch_slot` returns nothing in world mode 3, so they do not place rays on the product path. `lighting_compact.comp`
23. PR #288 (open, not on this branch) would give movers a side pool of texels and trace those itself.
24. PR #292 (open, not on this branch) limits the direct-history cut to the previous and current range balls, and keeps 12 samples there.

## Who throws history away (14)

The trigger is the problem. Most of these fire on a tiny change and drop a lot of good samples.

1. `lighting_fill_commit` sets blend to 1. Trigger: any lamp note in the last 120 frames. A move past 1 mm, any recolour, or a range change past 0.001. Every world cell, including cells the lamp cannot reach.
2. `lighting_lamp_keep` is 0, and the filter then takes the new target in one step (`catch_up = 1`). Trigger: the same 1 mm move, or the lamp going out, for cells inside the ball. An in-place colour change keeps the samples (keep is 1). A luminance split past 1.3× also takes the target in one step.
3. The local blend floor replaces at least 1/8 of the mean per sample. Trigger: a mover inside its sphere plus 1 m, or any positive `lighting_lamp_delta` inside the ball since the cell was committed.
4. While the scene is not "quiet", every cell blends at least 1/32 per sample, whether or not the change reaches it.
5. With the urgent bit and flag 4096, a near mover wipes the stored hit point.
6. `lighting_fill_coarsen` / `lighting_fill_free` delete a fine cell that is off screen and past the distance leash. Trigger: distance, including the motion lookahead (`lighting_fine_reach`), not the size of the light change. The fine samples are gone.
7. `lighting_fine_evict` deletes a whole fine texel when a new one needs a slot. Victim: not shown for 60 frames, farthest, or past the drop distance.
8. `lighting_fine_adopt` overwrites a child's seed with its parent until the child has four own sets.
9. `lighting_shadow_epochs` drops that lamp's direct-light cache. Trigger: any difference in position, range, or colour, or any mover sphere inside the lamp's range.
10. `restir_live.comp` cuts every pixel's history count down to this frame's candidates. Trigger: header word 39, any lamp change anywhere.
11. `lighting_direct.comp` skips the running mean for every pixel on that same word 39. With the word clear, one step of the 12-bit log luma (about 1%) or a visibility flip still zeroes the mean.
12. A failed reprojection drops that pixel's history (relative depth 0.02, 0.05 m, normal dot 0.9). The pixel is a different surface. This one stays a full clear.
13. A slot nobody has read for `LIGHTING_CACHE_LIFE` frames, and not shown for 60, is freed. That is age, not a change.
14. Screen-probe history: the passes are already removed. Nothing left to drop.

Rows 6, 7, and 8 are the coarse/fine swaps. A moving camera or a moving box changes who is fine, so detail appears and disappears while the light has barely changed.

## What each path becomes

| Path | Fate |
|---|---|
| The fill score (`lighting_fill_q`) | Kept. It is the cell value. New cells start high. Samples decay it. |
| Tile winners, off-screen bins, the budget split, the check hash, the emit lock | Folded into one top-down spend. |
| Check rays and the off-screen floor | Folded into the trickle floor, one setting, a fraction of `fill_budget`. |
| F4a seeds | Deleted. A neighbour gets rays by winning the rank, not a copy. |
| Lamp note, keep 0/1, blend 1, the 1/8 and 1/32 floors | Deleted as clears. The lamp adds value, and shortens sample weight, by the size of the change, and only inside the previous and current reach. |
| `lighting_lamp_delta` | Kept. It is the size estimate. |
| Mover spheres, object-local texels | Kept as a value input. The texel that rides the box keeps its samples. PR #288's pool, if it lands, is storage. It does not run its own rays. |
| Reach pass, à-trous dirty pages | Kept. They choose screen refresh and filtering. They do not trace cache rays and do not clear a cell. |
| Fine spawn, leash coarsen, fine evict, parent adopt, motion lookahead | Deleted as responses to motion. Size stays on the distance band with hysteresis. |
| Shadow epochs, quiet-pixel skip, cache-ray lamp pick | Kept on the direct path. |
| ReSTIR history and the direct mean (word 39, the 1% luma step) | Folded into the same proportion rule, per pixel. PR #292's balls stay the "who". The fixed wipe and the fixed 12-sample cap do not. A failed reprojection still clears. |
| Young/mature, trickle list, refresh, scouts, gradient list, `ray_budget` cap | Deleted. Already off in world mode 3. |

Change size, plain: move distance divided by the lamp's range, and the relative colour or brightness change. A 5 cm step on an 8 m lamp is a very small size. A colour flip or a lamp switching off is about 1, which is the full clear. A mover raises value for cells its before-and-after sphere covers, by how much of the cell it can block, and does not zero them.

Trickle: every cell in range is in the random draw, even at value 0. The floor is reserved first; the rest of the budget goes down the ranked list. One ray from that floor that misses the stored mean by more than the cell's own noise raises the value sharply and lowers the sample weight by the same fraction. Movers and lamps can be rough. A miss the estimate skipped still comes back through this. It is not a second system.

## Order

1. Stop the clears. Sample weight scales with change size. Full clear only for a gone surface, a failed reprojection, or a change near 1. The 1 mm lamp move and word 39 no longer wipe.
2. One value, one top-down spend of `fill_budget`. Remove the tile winner, the bins, the hash check, and the split.
3. Movers, lamps, and new view only add to that value. Freeze cell size to the distance band.
4. Add the trickle floor in that spend. A disagreeing trickle ray raises value. Then delete F4a and the world-mode-not-3 lists.

## Tests

Lavapipe, hall, near error against the path reference (`1 / max(distance, 0.5 m)`).

| Case | Pass |
|---|---|
| A small lamp move (about 1% of range) and a 1.5% dim, each against a run that full-resets on the change | The frame after the change has lower near error than the reset run. The frame where error reaches the still-lamp floor is no later than the reset run. |
| The same lamp change with the ring and the change list not written, so nothing signals it | A trickle ray raises that cell's value, and near error is back to the still floor within 120 frames on High. Floor set to 0 fails this. |
| Boxes and lamps moving | Cell size does not flip between 12.5 cm and 25 cm because something moved. A cell changes size only by crossing the distance band. |

Direct pixels follow the first row on their own mean: a small lamp step keeps more of the mean than today's word-39 clear, and a lamp that goes out still drops inside its old ball.

## Later: neighbour light

Not built.

A cell may borrow light through the level-of-detail tree, looking up to its parent and down to its children. A fine cell near the camera can reach farther than the cells that touch it: its parent, and the rings two and three cells out, so a small cell draws on a wider patch of the world. The weight falls off with distance. Two small cells close together share strongly. Two large cells far apart share weakly. The weight also follows how well the two cells match.

Borrowed light is a guess. Every frame the cell's own ray scores the light already stored and the light it borrowed, and when the borrowed light clearly wins it replaces what was stored, so a day that turns to night goes black instead of grey. The guess comes only from a neighbour's own ray. A cell does not pass on light it borrowed, so the guess travels one hop. Taking a borrow raises that cell's value, and the next rays are what check it.
