# S79B: Variance and push use the work lists

- **Date:** 2026-10-13
- **Result:** slower on the RTX 4070 (moving a-trous +0.4–0.7 ms, still compact 0.15 → 0.27 ms). Counters below.
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

`lightingatrous` prints `var`, `push` and one word per level (`list`, `dense`, `cap`, or `off`) plus `fallback`, the number of those seven passes that hit the cap. `dense` is the fraction gate. `cap` is a list past 65536. `LightingAtrousStats` uses 1, 2, 3 and 0 for the same codes.

## Deviations
- SVGF, the à-trous filter, and the pull-push recompute every pixel every frame. A slot the early-out would skip is not visited. A texel that was already inactive keeps the zero masks already stored.
- The push's view-held stamp bump does not run for a slot left off the list. `lighting_sc_shown` already treats a held camera as a fresh field stamp, so the picture does not drop the field on that timer. The 1-in-16 floor still queues the slot.
- The due test in compact runs before the cache pass writes. The cache pass and the level-5 stamps append what that test could not see yet.

## Numbers
Owner, RTX 4070, 1920×1080, two runs against `9e08d7d`: moving a-trous rose 0.4–0.7 ms, including on the compose-only commit. Still compact went 0.15 → 0.27 ms with this commit. Still fps dropped 6–7. No visible artifacts.

The moving rise on the compose-only commit is the push walking screen rects, described in S79A. This commit adds a second cost on a still frame: while the fraction stays at or under 0.25, compact tests every live slot for the variance and push due sets and appends them. Variance and push then leave the coalesced early-out sweep for those lists. Levels 1–5 were already lists on a still frame.

A moving stress frame is expected to stay on the dense side of the gate (`dirty` above 0.25, every level `dense`, `var` and `push` `dense`, `fallback` 0). The 0.9 ms estimate applies only when those words say `list`.

## Screenshots
not recorded

## Outcome and why
The lavapipe test forces threshold 1, then 0, then 1. Variance, push and each level report list, then dense, then list, and the cap count stays 0. The list picture and the dense picture stay within the same-mode frame step plus 2 byte levels, the tolerance the level test already used.

## Lessons / when to revisit
Do not extend these lists unless a moving stress frame prints `list` on the levels with `dirty` at or under 0.25 and `fallback` 0. A `dense` line is the fraction gate. A `cap` line is a due set past 65536. Neither has a skip left to take.
