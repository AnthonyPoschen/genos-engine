# S79B: Variance and push use the work lists

- **Date:** 2026-10-13
- **Result:** pushed (not re-timed; the owner measures on the RTX 4070)
- **Commit / patch:** this commit

## Idea
Variance and push walked all 1,048,576 slots even when few texels were dirty. Use the same work lists as a-trous levels 1-5 when last frame's dirty fraction is low.

## Motivation
`docs/systems/rendering/perf-review.md`: a-trous is 1.7-4.5 ms on the moving stress scene. The swing is levels 1-5 switching between the list and the dense sweep. Variance and push are full sweeps on both ends. The review's target is a 5 ms frame, about 200 fps.

## Paper basis
Stage 0 is SVGF variance (Schied et al., HPG 2017, section 4.2): a 7×7 spatial moment estimate when the texel has fewer than 4 sets, otherwise the texel's own variance. Stages 1-5 are the edge-avoiding à-trous filter, B3 kernel (Dammertz et al., HPG 2010). Stage 6 is pull-push (Gortler et al. 1996): the fine value weighted by its mask, the world-radiance cells bilinear for the rest. Those papers recompute every pixel every frame. The dispatch here is still one thread per slot. Threads past the list count return after reading the count, which is the S78X list, not S78A's indirect dispatch.

## What was implemented
The gate is S78X2's. Last frame's fullest level at or under `GENOS_LIGHTING_ATROUS_DIRTY` (default 0.25 of the filtered texels) builds the lists. Above that, or when a list passes 65536, the pass walks every slot. Threshold 0 is always dense. Threshold 1 keeps the lists up to the cap.

Compact appends the slots the dense pass would recompute. Variance: the S78B early-out inverted, including a texel that was active and no longer is (it still stores the zero masks). Push: the push early-out inverted. The cache pass appends a variance slot after an irradiance write, because compact saw the old value. Level 5 appends the texel and its ±1 neighbours when it stamps them for the push. One key word a slot stores the frame and two tag bits, so each of those writers adds a slot once. That word is not activity `.x`; `.x` of the second dirty-stamp vec4 is the level-5 dirty stamp.

The filtered set is about 108k texels in a still scene. That does not fit the cap, so the list is the due set, not every filtered texel. The dispatch is not shortened.

`lightingatrous` prints `var` and `push` (`list`, `dense`, or `off`). `LightingAtrousStats::variance_mode` and `push_mode` are 1, 2, and 0.

## Deviations
- SVGF, the à-trous filter, and the pull-push recompute every pixel every frame. A slot the early-out would skip is not visited. A texel that was already inactive keeps the zero masks already stored.
- The push's view-held stamp bump does not run for a slot left off the list. `lighting_sc_shown` already treats a held camera as a fresh field stamp, so the picture does not drop the field on that timer. The 1-in-16 floor still queues the slot.
- The due test in compact runs before the cache pass writes. The cache pass and the level-5 stamps append what that test could not see yet.

## Numbers
Not measured here. Expected on the moving 1920×1080 stress scene: about 0.9 ms when the dirty fraction stays at or under 0.25, and about 0 ms when it does not.

The 1.7-4.5 ms swing is 2.8 ms across the five level dispatches, about 0.56 ms between a list level and a dense sweep. Variance and push are two dense sweeps on both ends of that range (about 0.55 ms each if the five list levels are about 0.12 ms and the 1.7 ms floor includes them). A list in place of each is the level-list cost. Two of those replacements are about 0.9 ms. A due set larger than 65536, or a fraction above the gate, keeps the dense sweep.

## Screenshots
not recorded

## Outcome and why
The lavapipe test forces threshold 1, then 0, then 1. Variance and push report list, then dense, then list. The list picture and the dense picture stay within the same-mode frame step plus 2 byte levels, the tolerance the level test already used.

## Lessons / when to revisit
If the due set sits just over 65536 while the fraction is still under 0.25, the pass goes dense and this cut saves nothing. The owner's A/B is `GENOS_LIGHTING_ATROUS_DIRTY=0` against the default.
