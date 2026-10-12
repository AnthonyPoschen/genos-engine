# S78H2: SHaRC read rule, fine short-hit reads at 16 sets or settled (rejected; patch gi2diag/g78/S78H2-sharc-read-16.patch)
comp5 (lamps 5, frame 1,400, rough): fine-texel ratio 1.224 (S78X 1.197), screen +10.62 levels (S78X +10.63). Pop 0.117 / 0.152 % (S78X 0.092 / 0.137). qs 35 fps against 33 fps.
Postmortem: no effect. Short-hit fine reads are a small share of cache reads (about 1k of 17k a frame late in the run), so this path is not
where the offset comes from. Option 2 (the self-lighting limiter) was stopped before testing because coding moved to cloud agents.
Open: where the moved run's fine texels pick up their +20 % (their per-set incoming light still needs a dump).
