# S73S — Feedback reads raw means

S73R gave a revealed pre-fill a history of 1/16. On lavapipe the mechanism worked and the first on-screen sample was still about 1.47 against a 1.08 reference. That sample is the published 25 cm cell. A pre-fill neighbourhood holds one or two sets, so the SHaRC read (four sets) misses and the bounce takes the dark-fill resolve. The resolve is about 1.65 because it is fed back into the next sample. A plain mean of those same samples sits on the same value. Turning dark fill off for every reader leaves the basin and was rejected: on a 4070 the frame went 9% darker, dark areas 16%, bounce-only 20%, and 189 fps fell to 132.

## Two reads

The display resolve is unchanged. `lighting_sc_shown`, `lighting_sc_wrc_now`, compose, and probe or fill hits still read the published irradiance. Dark fill stays the hole filler. Flag 67108864 stays off, so the k^2 weight stays on.

Anything that becomes a bounce or is written into accumulation uses `lighting_feedback_radiance` instead. That is a cache ray in world mode 3: a pre-fill seed, a shown fine texel, and a world-radiance cell's own rays. The plain mean is that same bounce, not a second lookup. Probes stay on the published read.

A raw mean is the value before the dark-fill weight. A world-radiance cell (size above 0.2) is the plain estimator, `a.rgb` scaled by the sun gain, count `b.w`. It is refused when `b.w` is at most 0.5 or `b.x` is above 0. A fine texel is its stored irradiance, count `floor(w)`, refused while a set is in progress (`fract(w) >= 0.1`) or the count is under half a set. The published world-radiance rgb is not read.

The size walk is the existing SHaRC walk: the finest size that is still small against the hit, or the hit size when the hit is far enough or the surface is slanted. A cell with at least four raw samples is the whole answer. Otherwise

```
L = (n_f * L_f + k * L_c) / (n_f + k)
```

with `k = LIGHTING_FEEDBACK_K = 2`. `L_c` is the next coarser level's raw mean. An empty coarser level is skipped, up to four levels, and a level above 4 m is not used. `n_f` is 0 on a miss, so the result is that coarser mean. A low-count fine cell whose coarser levels are all empty returns the fine mean. The result is 0 only when every level is empty. The sun share uses the same weights.

The hit-size cell is still claimed when the old read would have fallen through to `lighting_cache_irradiance`, and the lit-from offer still runs. The claim does not read the published irradiance. A fine cell that already has four sets does not claim, matching the old early return.

`lighting_feedback_read` is set for the duration of that read. `lighting_feedback_published` counts, in `diag[63]`, a return of a world-radiance cell's published irradiance while the flag is set. The pre-fill test reads that word after the turn, after the hold, and on the pre-fill-off run, and asserts 0.

The fractional history from S73R stays. A seed that is still wrong should lose to the first on-screen sets.

## Deviations

- SHaRC continues the path past a cell that is too close or short of samples. This blends one coarser raw mean with prior weight 2. No second bounce is traced.
- The published dark-fill mean and the plain mean are still two values of one cell (S73Q). Both are now means of the same raw bounce. Only the published mean keeps the k^2 weight. Display reads the published mean. Feedback reads the plain mean, or a fine texel's stored irradiance.
- S73R's fractional history, the 120-frame LastUpdated age, and the override of A-SVGF and the hint while the marker is set are unchanged.
- Feedback lookups do not claim except for the hit-size residency claim above. They stamp LastUsed on the cells whose raw means they return.

Dark fill stays on. `DEFAULT_BIAS` stays 0.50 until a measured mean relative error at about frame 4 is at most 0.10.

## Curve

Lavapipe, 320×180, settle 48, one-frame turn of 0.5 rad, hold 8, sun frozen. 13637 fine texels were pre-filled at the settle. 2365 of them were shown on the reveal frame (the just-revealed cohort and the all-revealed cohort are the same set). Compared with the locked 192-frame pre-fill-off dump at the same end yaw (`/tmp/genos-gi-prefill-lock/reference/f192`, reference luminance 1.076 on these keys). The in-test reference of 4 frames (luminance 0.520) is not this number. `diag[63]` was 0 after the turn, after the hold, and on the pre-fill-off run.

Mean relative luminance is `|a-b| / max(a, b, 0.05)` per key.

| Frame | Mean lum | Mean rel | Mean sets | Mean hist | Pop from previous (mean step, share > 0.10) |
|---|---|---|---|---|---|
| settle (48) | 1.120 | 0.169 | 1.25 | 0 | |
| reveal (49) | 1.123 | 0.168 | 1.00 | 0.062 | 0.004, 0.016 |
| +1 (50) | 1.305 | 0.207 | 1.86 | 0.918 | 0.170, 0.611 |
| +2 (51) | 1.339 | 0.220 | 2.00 | 1.062 | 0.031, 0.114 |
| +4 (53) | 1.369 | 0.229 | 3.30 | 2.364 | 0.050, 0.162 |
| +8 (57) | 1.385 | 0.236 | 4.20 | 3.260 | 0.031, 0.091 |

Every key carried the S73R marker. History 0.062 is 1/16. The reveal mean is 1.123 against 1.076 (about 4% high) and the mean relative error is 0.168. By frame +2 every key had one on-screen set. Solving `1.123/17 + 16/17 y = 1.339` gives a new sample near 1.35. Frame +8 is 1.385 (about 29% high) at mean relative 0.236. The 0.10 target is not met, and the reveal mean is outside 2% of 1.07. The cap stays 0.50.

The texel direct-light cache on the same keys matches the reference: mean luminance 0.539 at the reveal and at frame +8, against 0.547, mean relative 0.045. The gathered irradiance is the biased value. S73R's before curve, on a published dark-fill read, was mean relative 0.313 at the reveal (luminance 0.773) and 0.329 at frame +8 (luminance 1.587).

`k` stays 2. At the settle the hidden fine texels (13637, under 4 sets) had mean luminance 1.134, and the world-radiance plain mean was 0.818 (54898 cells; published mean 0.833). The old 1.65 basin is gone, and the coarser raw mean is the darker of the two. A count of 2 blended with that prior is about 0.98 at k = 2, 1.03 at k = 1, and 0.92 at k = 4. That lookup is already near the 1.076 reference. The texel value is the integral of the hit radiances, and the first on-screen sample is about 1.35. Moving k inside 1–4 cannot close that sample onto 1.08, and a larger k darkens the bounces. No second curve was run.

The fractional history does not help this curve. The seed mean is the closer of the two values, and the 16/17 weight replaces it with the 1.35 sample. It stays in the tree; taking it out is its own change.

## Display image

Frame 48, the settle, same bench pose, against current `gi-v2` (`c58dd5b`). 320×180, tone-mapped 8-bit, Rec.709 luma. The dark quartile is the pixels at or below the gi-v2 25th percentile. S73R's still frame at the same pose is the row before this split.

| Compare | Tone mean | Tone ratio | Tone mean rel | Dark-quartile mean | Dark ratio | Dark rel | Share of pixels over 5% / 10% |
|---|---|---|---|---|---|---|---|
| gi-v2 → S73S | 59.62 → 57.47 | 0.964 | 0.053 | 23.00 → 21.08 | 0.916 | 0.085 | 0.510 / 0.075 |
| gi-v2 → S73R | 59.62 → 60.34 | 1.012 | 0.013 | 23.00 → 23.04 | 1.002 | 0.012 | 0.033 / 0.004 |

Shown fine texels with at least one set, same 17489 keys, linear luminance: gi-v2 0.465, S73S 0.426 (ratio 0.917, mean relative 0.107). Their dark quartile goes from 0.215 to 0.194 (ratio 0.902).

The display path still reads the published cell. The values in that cell moved. At frame 48 the published world-radiance mean is 0.833 on this branch and 1.050 on gi-v2 (S73R 1.049). The samples that write it are now raw-prior bounces, and the shown fine texels accumulate from the same bounces. The picture is 3.6% darker overall and 8.4% darker in the dark quartile. That is the same direction as the rejected 4070 dark-fill-off result (9% overall, 16% in dark areas) and smaller on lavapipe. The 4070 measurement of this split is still the owner's.
