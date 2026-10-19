# WAKE (PR #31): Probe ring tests with group splitting

- **Date:** 2026-10-13 (work); closed 2026-10-19
- **Result:** rejected / not remerged (not pushed onto current master)
- **Commit / patch:** PR #31 tip `7729ef55` on `cursor/gi-probe-ring-wakes-ac40` (closed; branch deleted). Approved design remains [WAKE](WAKE-neighbour-wake-design.md) (planned).

## Idea

Stop waking probes from the shared world-cache field. A lamp or geometry change runs coarse-to-fine ring tests at doubling distances; a group splits only when it changed past tolerance. A verification stride scores direct luminance. Legacy wake mode stays the default until rings win.

## Motivation

Approved wake design (Anthony): real neighbour change only, ring tests with group splitting. Legacy still traces one check ray per placed probe; moving cameras pay full strata (WAKE0 baseline).

## What was implemented (PR branch)

Ring wake path, `--wake legacy|rings`, `lightingwake` counters, kept-hit reshade when a moved lamp/geometry bound reaches a hit (>2% unshadowed change), geometry bound ahead of lamps, lavapipe picture tests, owner 4070 A/Bs on intermediate tips. Commit `45172a7d` dropped ring-wake launches that did not shrink the frame. A false picture pass (box faces lagging under the 2% skip) was narrowed so movers get full reshade.

## Numbers (prior owner / lavapipe; not remeasured this window)

| What | Result |
|---|---|
| Owner `fc1a3b8e` moving | 48 → 57.5 fps, light 8.1 → 5.2 ms (claimed); picture later showed box-face lag (fixed on later tip, **not remeasured on 4070**) |
| Earlier `6cd3cef` vs gi-v2 | moving 30 → 23 ms, 34 → 44 fps (owner) |
| Lit corner 640×360 | ~112 → ~103 fps with GPU total equal (host readback later removed) |
| Lavapipe direct time (moving-box corner) | rings ≈ legacy (same band) |
| Triage (19 Oct) | prior note: rings cut probes but **no fps win** on one 4070 A/B |

No exclusive A/B of tip `7729ef55` vs master `a4722b59` this window: merge hit **12 conflict files** (direct/gpu/lighting_gpu/dispatch/settings/docs). Standing rule: do not spend long conflict surgery on a PR without a clear clean-GPU win on current master.

## Outcome and why

**Closed.** Design stays planned under WAKE. The branch was merged with an old `gi-v2` tip and does not apply cleanly to post-#23/#36/#57 master. Prior fps claims are tip-specific and partially contradicted by “no fps win” / “drop launches that did not shrink the frame”. Revisit only as a fresh branch from current master with an exclusive still+moving A/B before merge.

## Lessons / when to revisit

- Remeasure every tip that changes the 2% skip or mover bound on the 4070 before claiming fps.
- Keep legacy default until rings beat legacy `GPU_MS` lighting on moving **and** picture noise floor.
