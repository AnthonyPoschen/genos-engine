# S73Q — Pre-fill reads a plain mean of the world radiance cache

The S73O lavapipe compare left revealed pre-filled texels near 1.69 against a pre-fill-off settle near 1.07 (mean relative luminance 0.374 over 3385 keys). The 0.50 cap recorded that gap. It is not the fix.

## What the dumps ruled out

Same keys in `/tmp/genos-gi-prefill-lock` (pre-fill, frame 96) and that run's 192-frame pre-fill-off reference:

- Own direct light matches. The texel direct cache ratio is 1.015 (mean relative 0.046). Sun fraction of that cache is 0.069 versus 0.070.
- Normals match (dot 1). Positions sit within about 8 cm.
- The scale is about 1.50 on every channel. The sun part of the irradiance is 1.23 times the reference; the rest is 1.52. A doubled sun or sky would not scale the rest the same way.
- The firefly cap cannot pull a mean of 1.57 down to 1.07 when the samples themselves sit near 1.57. The first sample is uncapped on both paths.
- The plain-mean handoff does not move the value. New on-screen samples read the same bright neighbourhood, and a texel gains about one set every ten frames.

## Where the samples come from

A pre-fill ray almost never takes the fine-texel read. That read requires four finished sets, and a never-shown texel holds about two. At a settled frame the bounce came from the 25 cm cell or its parent. Those cells are the world radiance cache.

With dark fill on, that cache at the hidden cells is already about 1.65 by frame 48, in both the pre-fill run and the pre-fill-off run (relative luminance between the two caches 0.058). The pre-fill texel's own value sits on that cache (ratio about 0.95). A fine texel that has been shown since frame 0 keeps its early samples and levels off near 1.07 to 1.18. Its recent samples (short mean about 1.22 at frame 192) stay below the pre-fill value. The two runs agree on the world radiance cache and disagree on the fine texel.

Turning dark fill off (`GENOS_LIGHTING_SC_FLAGS=73115914`) drops the hidden-cell cache to about 1.03 and the pre-fill texels to 1.069 against the old 192-frame reference 1.074 (ratio 0.996, mean relative 0.146 over 2494 keys). The same flag also darkens on-screen fine texels at frame 48 (0.59 to 0.45), so it is not a local fix. Replacing each pre-fill texel by the mean of its 25 cm cell moved 0.146 to 0.132. Copying the cache cell itself onto the texel was worse (mean relative 0.260).

## What landed

The published world-radiance irradiance is unchanged: dark fill, the change floor, and every existing reader. Beside it, the estimator slot keeps a second mean with the dark-fill weight left out (`a.rgb` the mean, `b.w` the count, `b.x` held at 0 so trickle, dormancy, and variance readers skip it). Sun gain scales that rgb with the rest of the estimator.

A world-radiance cell's own rays still publish the dark-fill sample. A second cache read of the same hit, taken only for those rays, feeds the plain mean, so the plain mean is its own loop. A pre-fill ray's one bounce read uses the plain rgb when the cell has a plain count. Parent and neighbour fallbacks on that read do the same. Pixels, probes, and shown texels do not.

Deviation: Lumen and the world-radiance notes have one value per cell. This keeps two, and only pre-fill reads the second. The second read is a cache lookup, not a second trace.
