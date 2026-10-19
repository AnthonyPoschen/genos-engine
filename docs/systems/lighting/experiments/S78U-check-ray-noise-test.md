# S78U: Held check ray is a test, not a rewrite (no wake from shared-cache drift)

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** this commit

## Idea
S78S found that 100 % of still-scene kept-light changes were the held probe's check ray: it was relit fresh every frame while the probe's other held hits kept their last light, so the converging surface cache showed up as one changed hit per probe per frame.

## What was implemented
- A check ray whose trace confirmed its hit (no hold fail, path crosses no change) is relit like every other held hit: only when a lamp in range or the sun/sky moved (S78D rule). Drift of the shared surface/world cache no longer changes a probe (Anthony, design point a).
- When a check ray is relit, its new light replaces the stored one only if the luminance differs by more than 3 standard errors of the hit texel's mean (MSME estimator var / sets, `lighting_sc_noise`) times the hit reflectance, plus a 1 % quantisation floor. Unknown noise (estimator not running) writes as before.
- `lighting_held_crosses` split out of `lighting_held_traced`.

## Paper basis
A-SVGF (Schied 2018) re-samples as tests against the previous value; the decision threshold follows the standard-error stop rules of Ward 1988 / Krivánek 2005 irradiance caching.

## Deviation
Held hits no longer pick up bounce-light convergence of the shared cache until their stratum is re-traced (hold fail, probe move, change crossing) or a lamp changes. Brightness and noise are unchanged in the smoke / shadow runs; the always-on verification budget (next design step) is where slow drift is meant to be caught.

## Numbers (RTX 4070, stress scene, 1920x1080, frames 300-600)
| | S78S | S78U |
|---|---|---|
| still | 8.05 ms / 106 fps | 7.91 ms / 108 fps (light 0.909 -> 0.757) |
| moving | 18.1 ms / 52 fps | 18.2 ms / 51 fps (noise) |
| still kept-light changes / frame | 6,480 | 0 |
| smoke a lum / noise | 0.348 / 3.78 | 0.348 / 3.79 |
| shadow b mean / noise | 97.9 / 3.08 | 97.7 / 3.10 |

## Screenshots
~/.cache/gi2diag/perf/shad-S78U/b.png (viewed; shadows and brightness as S78S, mean abs diff 0.9/255).
