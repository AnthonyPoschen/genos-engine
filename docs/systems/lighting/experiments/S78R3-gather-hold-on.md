# S78R3: Gather hold on (S78R re-test after S78U + S78W)

- **Date:** 2026-10-12
- **Result:** pushed (rough numbers; idle-GPU A/B queued)
- **Commit / patch:** this commit (LIGHTING_GATHER_HOLD = true)

## Why it wins now
S78R was rejected because most probes were dirty every frame (S78S: the held check ray relit against stale held hits) and because the held path's two stats atomics went to host-visible memory (S78W). With S78U (no wake from cache drift) and S78W (device-local stats) the still scene holds 8,088 of 8,093 probes' gathers.

## Numbers (rough, measured under gaming load, RTX 4070, stress, 1080p)
Gather dispatch still: 0.85 -> 0.09 ms; still lighting total 7.20 ms / 115 fps (one run). Filter rose 1.09 -> 1.30 ms in that run (reads the raw-cell table; check at the idle A/B). Moving run invalid (game running).
Smoke a 0.348; pop 0.105 % / 0.143 % (S78W 0.114 / 0.142).

## Screenshots
shad-S78R3/b.png viewed (mean abs diff vs S78W 0.74/255).
