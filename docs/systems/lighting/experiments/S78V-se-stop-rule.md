# S78V: Standard-error stop rule for surface-cache texels

- **Date:** 2026-10-12
- **Result:** rejected (no measurable gain); patch g78/S78V.patch
- **Commit / patch:** g78/S78V.patch

## Idea
A texel whose mean's standard error is within its tolerance (lighting_sc_noise vs lighting_dn_target) and whose short/long means agree (GIBS inconsistency < LIGHTING_SC_SETTLE_SD) is done (Ward 1988; Krivánek 2005): it waits at the dormant interval (600 frames) instead of 240 / (1 + 16 inc), and takes no trickle rays. Local change, moved lamps and global light change still wake it.

## Numbers (RTX 4070, stress, 1080p, frames 300-600; idle GPU)
| run | S78U still / moving ms | S78V still / moving ms |
|---|---|---|
| 1 | 7.91 / 18.23 | 7.85 / 17.90 |
| 2 | 7.84 / 18.10 | 7.80 / 18.22 |
| 3 | 7.95 / 18.18 | 7.96 / 17.99 |
Pop (f20-80 / f80-320): S78U 0.074/0.119 and 0.075/0.116; S78V 0.121/0.158 (first run) and 0.077/0.111.
Smoke a 0.348, noise unchanged.

## Postmortem
The surface cache already stops on standard error: the S62A perceptual stop in lighting_sc_settled is that rule, and the cache pass costs only ~0.04 ms in the still scene. Moving settled texels from a 240-frame interval to dormant and dropping their trickle removes a few hundred rays a frame, which is below run-to-run noise (+-0.1 ms). The still frame's cost is not in cache rays (see the S78U/V time split in the README); the remaining work is fixed per-pixel/per-slot passes (compact, compose, a-trous, filter, gather). Rolled back; the noise-aware rewrite threshold already lives in S78U's check-ray test.
