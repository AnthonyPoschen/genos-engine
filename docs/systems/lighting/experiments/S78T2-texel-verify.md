# S78T2: texel verification (rejected; patch gi2diag/g78/S78T2-texel-verify.patch)

Idea: each round, a rotating, hash-spread 1/256 of settled texels gets the S74A same-seed
gradient check. Normally they only get it near a hint. A check with lambda >= eps drops
history as before and wakes a ring of 4 checks: radius 2 x lambda/eps cells, halved per hop,
one event stamp per texel, capped at 4096 a frame.

First try: compare a settled texel's new set with its mean (3 per-set sd) and reset on a miss.
It never fired, because resolved texels take no sets (the A-SVGF branch returns first, x29 = 0).

stuck.sh (rough, under gaming load). Columns are mean|d|, dark>12 and bright>12 against the never-moved reference.
- S78X:  t00 12.86 / 14.5% / 40.7%; t03 9.93 / 0.04% / 40.9%; t11 9.93 / 0.016% / 41.1%
- S78T2: t00 12.93 / 13.8% / 40.9%; t03 10.09 / 0.13% / 41.1%; t11 9.94 / 0.017% / 41.2%
Still-scene activity: 18 rotation checks a frame, 2 spreads, 2 woken checks. Cost unchanged (qs fps 33 in both).

Postmortem: no healing. The dark patches heal by t03 without this change. What remains is a broad
offset where about 41% of pixels in the moved run are brighter than the reference. Neither the probe
checks (S78Z) nor these texel checks move it. The rotation also reaches only about 4.6k texels,
because compact's earlier skips (unread texels and the idle list) return before the check.
The offset probably doesn't come from stale texels. Next step: dump each component
(world cache, bounce, direct) for the moved run and the reference to find it. Drop S78Z.
