# Held values outside the refresh rect: reprojection proposal (#70)

Status: proposal for review. No renderer code yet. Follows `localized-invalidation-design.md` §16–17.

## Problem

At the controller floor (1/64 scale, always reached on lavapipe and for whole runs on the 4070 at 1080p High), two things broke the hall turn against master:

1. **Held screen values.** Outside the refresh rect, direct and trace keep last frame's value per screen slot. After a camera turn those slots belong to different surfaces, which shows as round and cross-shaped blotches and a global darkening. À-trous had the same fault and was already ungated (§17).
2. **Cache-refresh starvation.** `refresh_budget_shift` added one halving per halving of the controller scale (up to +6 at the floor). Every per-round cache list (young, trickle, WRC refresh, gradient entries) was cut 64×, so cells reached through a turn were never refreshed and read dark or from 1–4 m parents.

The cache ray grant (a positional cut that also fed stale hits) was a third cause, removed in 3769d16d.

## Measured (box, lavapipe, 640x360 High, turn.rhai, mean |d| / p99 vs master)

| step | change | still | turned | turned+16 |
|---|---|---|---|---|
| 3769d16d | cache ray grant dropped | 16.7/57 | 55.3/99 | – |
| M1 bd6c43c8 | no controller halvings in `refresh_budget_shift` | 4.0/18 | 17.2/63 | 16.9/58 |
| M2 87a34100 | spawn cap unscaled | 3.7/15 | 16.4/63 | 16.2/58 |
| M3 309db9fd | direct ungated | 3.6/16 | 12.7/54 | 12.4/49 |
| M4 f18dbed0 | trace ungated | 1.37/8 | 0.91/5.0 | 0.83/5.0 |
| ref | controller off | 1.42/8 | 1.45/5.7 | – |

Frame time (lavapipe, 20-frame `--gpu-times` means): 3769d16d ~1881 ms, M4 ~2085 ms (+11%); a rerun of the pushed head read ~2403 ms under CPU contention from another job. Lavapipe sits at the floor every frame, so this is a worst case for the controller and says nothing reliable about the 4070.

M4 is on the branch now. It reaches controller-off quality by removing the controller's screen levers and its cache-refresh cut, so at the floor the controller only scales trace/à-trous grants it no longer gates on.

## Proposal

1. **Bring back the direct and trace screen gates** (revert the two `plan.*_gate = false` lines in `schedule_push`). The rect rotation of §17 stays.
2. **Reproject held values instead of holding them per slot.** For a slot outside the rect, reproject its world position (from the G-buffer) into last frame's screen with last frame's camera, as ReSTIR already does with `lamp_cam`. Accept the history texel only if:
   - it was inside the display last frame,
   - view depth matches within a relative tolerance (start at 2%), and
   - normals agree (dot ≥ 0.9).
   Bilinear over the accepted taps only, with weights renormalised.
3. **Fall back to a world-cache lookup** when no tap is accepted (disocclusion, off-screen history). Use the same lookup compose uses for unfilled texels, so the slot gets a cache-quality value, not a held or black one. Mark the slot so the rotation reaches it first on the next frame (cheap: a per-tile counter folded into the existing tile mask).
4. **Cap `refresh_budget_shift`.** Restore the controller term but clamp the extra halvings to a cap `K`, chosen by measurement: sweep K = 0, 1, 2, 3 on turn.rhai and on the 4070, and take the largest K whose turned mean |d| stays within the controller-off reference.

## Expected cost vs M4

- Gated direct and trace at the floor cost one 4×4 rotation cell (1/16 of the display) instead of the full crop. On the 4070 at 1440p trace+direct are the largest lighting passes, so this is most of the saving the controller existed for.
- Reprojection adds one full-crop pass of ~4 taps plus depth/normal reads, expected well under 1 ms on the 4070 at 1440p. The cache fallback costs one lookup per disoccluded slot, a small fraction during a turn and near zero when still.
- The refresh cap returns up to 2^K of the cache-list cut.
- Net: expect most of M4's extra cost back. That is a target, not a measurement; the 4070 run decides it.

## Test plan

- turn.rhai on the box: still, turned and turned+16 each within the controller-off reference (≤1.42 / 1.45 mean |d|, p99 ≤ 8 / 5.7), for the prototype and for each K.
- Same at controller default plus `GENOS_LIGHTING_BUDGET_SCALE=0.25` with the controller off, the fixed repro.
- 4070 at 1440p High: stress frame time and per-pass `--gpu-times` for 3769d16d, M4 and the prototype, plus the C5 turn and switch scenes for flicker. Run when no one is using the PC; fps from a busy PC is labelled not clean.
- Unit tests: `schedule_push` gates again at the floor; a reprojection check with a known camera delta; K is clamped.

## Risks

- **Ghosting:** a held value on a moving or changing surface passes the depth/normal checks but carries stale light (moving lamp, light switch). Mitigation: reject history on slots the localized invalidation marked this frame, and let the rotation refresh them.
- **Disocclusion:** newly visible surfaces have no history and fall back to the cache, which is lower quality near camera. A fast turn exposes a whole edge strip. Mitigation: leading-edge-first rotation (already in §17) and the young-cell priority.
- **Thin geometry and grazing angles** fail the depth test and fall back often. That costs quality, not correctness.
- If the prototype cannot match the controller-off reference at a lower cost than M4 on the 4070, M4 stays.
