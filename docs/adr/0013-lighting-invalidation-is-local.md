# 0013: Lighting cost is budgeted and reuses a world-space cache

Status: **accepted** (Anthony, 26 Oct 2026). The first code slice is local change tracking. Design: [`docs/systems/lighting/localized-invalidation-design.md`](../systems/lighting/localized-invalidation-design.md). Departures are in the lighting system record, section Local change tracking.

## Context

One global stillness hash switches every GI pass on or off. On stress/hall that means 0.43 ms frozen versus about 20 ms as soon as three boxes move (`/home/box/shared/moving-lighting-breakdown-2026-10-20.md`, master `27228dc4`). Rev 1 proposed screen dirty tiles with a full-frame fallback. Anthony's review: in real play the camera spins while things move, so screen tiles go fully dirty and the fallback is unbounded. He wants a hard static upper limit with maximum reuse.

## Decision

1. The persistent **surface light cache** is the reuse layer. It is extended, not replaced by GIBS surfels: hash LOD levels with hysteresis, a persistent coarse layer, merge-coarsening on eviction, and parent seeding on return.
2. Every expensive pass has a **static per-preset work cap** (rays, relights, spawns, coarsen ops, fresh shadow rays, denoise items). Work beyond the cap is deferred by priority. **There is no full-frame fallback.** Under load, values get older but are always finished.
3. A **priority scheduler** (change magnitude from change records, near-camera error, age, new visibility, inconsistency, plus a reserved starvation slice) replaces the global stillness hash.
4. Screen space does only fixed-size resolve work: apply, budgeted gather, ReSTIR DI resampling, compose, and one denoise iteration.

## Rules kept

Finished values only, blended once. Stored light never depends on the camera (ADR 0012). Lighting VRAM stays within ~200 MB (High ≈ 140 MB). Nothing depends on a minimum ray count (S72S).

## Rejected

- **Screen dirty tiles with a full-frame fallback** (rev 1): unbounded under camera motion.
- **GIBS disc surfels as a replacement cache:** the industry moved to hashed world caches, and ours already has MSME, pre-fill and dormancy (`docs/lighting/research/GIBS.md`).
- **Per-entry ray ceilings** (RAYCEIL, +1.35 ms moving).
- **Half-res denoise** (S79).
- **Scattered per-slot lists** (S78A).

## Consequences

`--lighting-schedule global` stays as the A/B reference until P6. Each phase lands with its own A/B and gameplay gates (spin, strafe, approach, many movers, A→B→A, settle). A phase that misses its gate is dropped under the standing drop rule.

## Code (26 Oct 2026)

Local change tracking is in the engine. Movers and point lamps emit change records. Those records dirty world-cache cells (swept bounds, lamp reach). The global full-frame drop is deleted. There is no full recompute fallback.

Not in this slice: the priority scheduler, per-pass work caps, cache LOD, and `--lighting-schedule`. Screen passes still run while a change record is in flight. The departures are listed in `docs/systems/lighting/README.md` under Local change tracking.
