# DP0 instrumentation (not measured here)

Phase 0 of `dynamic-perf-plan.md`. No picture change in the default build. The numbers are filled on the owner's RTX 4070; this file only says where they come from.

The plan's `diag[0..18]` are not free. Compact already writes slots 0-37 and 40-56, and the light pass writes 40, 41 and 47-49. The new counts are `gstats.dp[32]`, printed by `GENOS_LIGHTING_SC_TRACE=1` as `lightingdp`. They are compiled out unless the build is `GENOS_LIGHTING_DIAG=1`. `lightingdg` and `lightingchg` are unchanged.

| `dp` | Meaning |
|---|---|
| 0 | shadow rays traced by GI hits |
| 1 | shadow rays traced by pixels |
| 2 / 3 | hit shadow-cache hits / misses (`li < LAMP_ALL - 1`, lamp loop only) |
| 4 | GI hits with no fine texel |
| 5 | GI hits that lost the TDL budget |
| 6 / 7 | pixel lamp-list entries whose epoch did not move this round / all entries in that loop |
| 8-12 | a-trous work-list count for levels 1-5 (`atomicMax`; a value above 65536 overflowed) |
| 13 | stage-0 texels recomputed |
| 14 | level outputs that scattered |
| 15 | pixels with world trust `< 0.999` |
| 16 | tiles with at least one such pixel |
| 17 / 18 | hash lookups that miss (`claim == false`) / return a slot |
| 19 / 20 | reserved for the hit-light queue length and its overflow |

d6 compares each lamp's epoch with the copy taken at the start of `lighting_shadow_epochs()` (word `+ 6` of the lamp block). That function runs in the args pass, so d6 is for frames that ran a light-cache round. d8-d14 are for those frames too: a-trous is on the round path.

Existing `CHG_KEPT_TDL_REUSE / CHECK / LAMPS` (`lightingchg` slots 6-8) still split the kept hits' direct light. `perf/chg.py` reads that line.

Timers are the ones already printed. `GENOS_GPU_TIMES=600 GENOS_GPU_DISPATCH=1` prints `LIGHTING_DISPATCH i:pass=ms` in dispatch order and `LIGHTING_MS` summed by pass. Pass ids: 0 compact (the args step is also 0), 1 place, 2 trace, 3 direct, 4 gather, 5 filter, 6 cache, 7 compose, 8 a-trous. The moving per-dispatch table (plan 0.1) is this line for `perf/moving.rhai` and for the still scene. Record trace, both direct dispatches, gather, the three filter rounds, cache, each of the seven a-trous dispatches, compose, and compact.
