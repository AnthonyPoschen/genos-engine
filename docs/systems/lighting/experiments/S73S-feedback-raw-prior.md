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

Lavapipe, 320×180, settle 48, turn 1, hold 8, against the locked 192-frame pre-fill-off reference. The numbers are written here after that run. The display image is compared with a gi-v2 dump at the same bench pose.
