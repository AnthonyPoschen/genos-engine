# S73R — Revealed pre-fill is low-confidence history

Dark fill stays on. Turning it off made a 4070 frame 9% darker (16% in dark areas, 20% in bounce-only), dropped 189 fps to 132, and left the a-trous pass at 3.79 ms. The unfilled share did not improve. With dark fill on, PR #25 matches gi-v2 on screen and in speed, and a revealed pre-fill is still 1.50× the reference (mean relative luminance 0.341).

S73M kept one finished set, then blended with `k = 1/(t+1)`. The displayed count starts at 1 and a texel gains about one set every several frames, so the bright seed still dominates at frame 4. The fix is at consumption. The cached colour is kept.

## What a first show writes

The first pixel that shows a pre-filled fine texel:

- Shows the cached colour with displayed count 1, so the pixel stays filled and the unfilled detector does not count it.
- Writes the estimator as a newborn (`variance (0.15 L)^2`, `b = (1, 1, -1, 1/16)`). `b.z < 0` is the handoff marker. `b.w` is the history, not the displayed count.
- Ages `LastUpdated` by 120 frames when that stamp would not mark the texel stale. A stale texel is dropped from the picture, so a stamp inside the sun-rise window, or one whose sun ratio exceeds 2, is replaced by the previous frame, or left as it was.

## What the next on-screen sets do

While the marker is set and the history is under 16, the blend is `k = 1/(hist+1)`, after the young cap, A-SVGF, and the hint, and it replaces them. The history starts at 1/16, so the first on-screen set is 16/17 of the mean. The marker clears on the set whose history reaches 16. The displayed count still increments from 1. The sun-share mix uses the same history.

`LastUpdated` is not stamped on a partial set while the marker is open, same as a never-shown pre-fill. The first quarter must not undo the age before the other three quarters run.

The reveal frame itself does not trace the new set. The handoff runs in compose, after the cache pass. The aged stamp is visible to the next compact.

The old estimator (flag bit 1073741824, MSME off) does not read the fractional history.

## Deviations

- MSME and Lumen do not keep a fractional sample count. The displayed count and the estimator history are different numbers here.
- A-SVGF and the fast-update hint do not get a vote while the marker is set. Both papers let a change raise the blend.
- The 120-frame `LastUpdated` age is not Lumen's LastUsed − LastUpdated. It exists so the 8× sample-count weight, which already saturates at one displayed set, loses to a texel that has been read for a while.
- Holding `LastUpdated` until the set's last quarter, while the marker is open, is the never-shown pre-fill rule applied after the texel is shown.
- A 7×7 luminance clamp of on-screen neighbours was measured and removed. The reveal step was 0.021. Lumen and MSME do not clamp a pre-fill that way, and this tree does not either.

Dark fill (S72N) is unchanged. The test cap stays 0.50 until a measured mean relative error at about frame 4 is at most 0.10.

## What the turn test prints

`examples/stress/tests/gi_prefill.rs` dumps the settle frame, the last turn frame (reveal), and reveal+1/2/4/8 when the hold reaches them, plus the end of the hold. For two cohorts (texels whose shown stamp is the reveal frame, and every pre-filled texel shown by then) it prints mean relative luminance against the pre-fill-off reference, the mean displayed sets and history, and the visible pop: the mean relative step between consecutive dumps and the share of keys whose step exceeds 0.10. The assert at the end of the hold is still the 0.50 cap. It does not assert 0.10.

The reveal frame's pixels can still show the world-radiance cell until the SVGF field is fresh. The curve is the layer-1 texel value, which is what that field is built from.

## Lavapipe curve

Settle 48, one-frame turn of 0.5 rad, hold 8, 320×180, sun frozen. 2343 pre-filled keys were shown on that frame (the just-revealed cohort and the all-revealed cohort are the same set). Compared with the existing 192-frame pre-fill-off dump at the same end yaw (`/tmp/genos-gi-prefill-lock/reference/f192`, reference luminance 1.077). The in-test reference of 4 frames is not this number. The run included the 7×7 clamp; its only measured effect is the reveal step.

| Frame | Mean lum | Mean rel | Mean sets | Mean hist | Pop from previous (mean step, share > 0.10) |
|---|---|---|---|---|---|
| settle (48) | 0.778 | 0.312 | 1.27 | 0 | |
| reveal (49) | 0.773 | 0.313 | 1.00 | 0.062 | 0.021, 0.082 |
| +1 (50) | 1.053 | 0.279 | 1.41 | 0.476 | 0.201, 0.403 |
| +2 (51) | 1.431 | 0.265 | 2.00 | 1.062 | 0.284, 0.573 |
| +4 (53) | 1.476 | 0.282 | 2.41 | 1.476 | 0.037, 0.140 |
| +8 (57) | 1.587 | 0.329 | 3.90 | 2.966 | 0.091, 0.315 |

Every key carried the marker. History 0.062 is 1/16. By frame +2 every key had one on-screen set (displayed count 2, history 1.062) and the marker was still set. Solving `0.773/17 + 16/17 y = 1.431` gives a new sample near 1.47. That is the dark-fill basin, not the 1.08 fine fixed point. Frame +1's mean luminance (1.053) matches the reference only because 41% of the keys had jumped and the rest were still at the dark seed. Mean relative error stays above 0.26.

The 0.10 target is not met. The cap stays 0.50. The visible pop is the jump from the plain-mean seed onto that bright sample. Giving the seed a small weight cannot land on 1.08 while the on-screen sample is 1.47.
