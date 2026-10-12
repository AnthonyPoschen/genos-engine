# S78B2: time-based MSME catch-up floor (rejected; patch gi2diag/g78/S78B2-msme-time-floor.patch)

GIBS / kajiya MultiscaleMeanEstimator (checked against WickedEngine's copy): catchUp = clamp(smoothstep(0, 1,
relDiff * max(0.02, inconsistency - 0.2)), 1/256, 1) * vbbr, applied once per update. In kajiya that is one
update per frame, so the floor gives a 256-frame memory. Ours matched it verbatim, but per set.
Change: floor k >= min(1 - (255/256)^df, 1 / (min(t, 16) + 1)), where df is the frames since the texel's last set.
The time-based floor is a deviation; the plain mean of the last 16 sets bounds the noise.

comp5 (lamps 5, frame 1,400, rough under gaming load):
- S78X: screen +10.6 levels, fine ratio 1.197, sets 9 vs 16 (moved/ref).
- S78B2: screen +9.6 levels, fine ratio 1.222, sets 9 vs 16.
Pop (f20-80 / f80-320): 0.124 / 0.140 %, against 0.092 / 0.137 % for S78X (slightly worse early).
qs: 36 fps against 35 fps.

Postmortem: the hypothesis was wrong for these texels. The biased fine texels stay at 9 sets through frame 1,400,
so they take no more sets at all and the blend floor never runs. They are A-SVGF "resolved"
(lighting_sc_resolved: 8+ sets and SE < 5 %). Resolved texels only get same-seed gradient checks, which re-trace
the stored ray and so measure a change in inputs, not the bias carried in the stored mean. S78S2 forced 16 sets
before resolving, and those texels then held a 1.16 ratio. So the biased value is formed during the fill sets
themselves, and the gradient checks cannot see it.
