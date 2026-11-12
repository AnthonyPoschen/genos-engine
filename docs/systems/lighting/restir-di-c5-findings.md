# ReSTIR DI stage C5: findings from the 4070 run (30 Oct 2026)

Stage C5 (PR #120, flag `restir_di`) puts ReSTIR DI on the live direct pass. It was measured on the RTX 4070 on 30 Oct 2026 (#120 @2469d274 with the atlas-clear fix applied locally, oracle profile `quick`). **It fails all three gates.** This note records what the numbers show and which fixes to try first, so that the rework can be reviewed before any code is written. See `restir-di-plan.md` for the original plan.

## Gates

| Gate | Target | Measured (flag on) | Verdict |
|---|---|---|---|
| `ref_err_near` | no worse than master | worse on sun-x100, sun-x10, turn and spin; better only on switch | FAIL |
| `grid_worst` | ≤ 0.15 | 0.27 to 0.89 | FAIL |
| Direct pass at 1440p | ≤ 3 ms | 25.8 ms | FAIL |

## Cost of the direct pass

Measured with genos-stress at 2560×1440 (`--proof`, default building), using the last 30 frames of a 120-frame run.

| Metric | Flag off | Flag on |
|---|---|---|
| Direct pass (`light=`) | 13.35 ms | 25.82 ms |
| ReSTIR initial candidates | | 7.02 ms |
| ReSTIR temporal reuse | | 0.47 ms |
| ReSTIR spatial reuse | | 4.45 ms |
| Lighting total | 32.62 ms | 45.28 ms |
| Frame | 34.52 ms (30 fps) | 47.23 ms (22 fps) |

What this shows:

- **ReSTIR adds to the old direct pass instead of replacing it.** The three ReSTIR passes take 11.9 ms, and the rest of the direct pass (about 13.9 ms) costs the same as the whole direct pass with the flag off. ReSTIR's job was to replace per-light shading with a few resampled shadow rays, so the flag-on cost should be lower than flag-off, not the sum of the two.
- **The flag-off baseline alone is 4× the 3 ms budget.** Even with a perfect ReSTIR, the old 13.3 ms path is the biggest obstacle to the frame-rate target.
- Initial candidate generation (7.0 ms) is the most expensive ReSTIR stage. Spatial reuse (4.4 ms) comes second. Temporal reuse is cheap (0.47 ms).

## Error and the jump in `grid_worst`

Oracle (`quick`) results, near error / grid error:

| Scene | Master | Flag off | Flag on |
|---|---|---|---|
| sun-x100 | 0.2255 / 0.5595 | 0.2307 / 0.5591 | 0.2343 / 0.5722 |
| sun-x10 | 0.230 / 0.7163 | 0.2345 / 0.7152 | 0.2420 / 0.7199 |
| turn | 0.474 / 0.7743 | 0.4753 / 0.7753 | 0.5925 / 0.8434 |
| spin | 0.121 / – | 0.1194 / 0.0 | 0.1234 / 0.8907 |
| switch f0 / f119 | 0.792 / 0.372 | 0.79 / 0.3791 (grid 0) | 0.7469 / 0.2651 (grid 0.7852) |
| switch-dark grid | – | 0.0 | 0.0 |

What this shows:

- **ReSTIR introduces frame-to-frame instability.** In spin and switch, `grid_worst` goes from 0.0 with the flag off to 0.89 and 0.79 with it on, while the near error barely moves. The image is about as correct on average but flickers, which suggests the reservoirs carry samples that are no longer valid when the view or the lights change.
- **Camera motion hurts most.** In turn, the near error rises from 0.475 to 0.593. That points at temporal reuse following the wrong pixels or keeping stale history when the camera rotates.
- switch gets better near error (0.79 to 0.747), so resampling helps once the lights are stable. The problem lies in reuse, not in the estimator.
- sun-x100 and sun-x10 already fail the 0.15 grid gate on master and with the flag off (about 0.56 and 0.72). That part of the gap comes from outside C5.

## Recommended fixes, ranked

The savings are upper bounds taken from the measured stage costs. They need confirming on the 4070.

1. **Make ReSTIR replace the old direct shading, not run beside it.** With the flag on, shade only from the selected reservoir sample, and skip the per-light loop. This could save up to about 13.9 ms, which brings the flag-on pass down to about the 11.9 ms of the ReSTIR stages. It's the biggest win, and it's the condition for ReSTIR paying off at all.
2. **Cut the cost of initial candidates (7.0 ms).** Use fewer candidates per pixel, draw them from a light-importance or per-tile list instead of all lights, and defer visibility to the final sample only. Halving the candidate cost saves about 3.5 ms.
3. **Fix the reuse stability (the `grid_worst` gate), and cut spatial reuse (4.4 ms) at the same time.** Reject history and neighbours by depth, normal and light-ID or light-change validation, reproject temporal history with motion vectors, and clamp the M cap. Use fewer spatial neighbours, or run spatial reuse at a lower resolution. This targets spin, switch and turn, and it saves up to about 2 to 4.4 ms.
4. **Track the flag-off 13.3 ms baseline as its own item.** Even with fixes 1 to 3 the pass stays above 3 ms, so the 3 ms gate also needs the shared shading and shadow cost cut, whether or not ReSTIR is on.

## Decision needed

Should C5 be reworked along fixes 1 to 3 on #120, or should #120 be parked while the baseline (fix 4) is addressed first? #120 stays open and unmerged until then.
