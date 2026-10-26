# 0013: Lighting cost is budgeted and reuses a world-space cache

Status: **accepted** (Anthony, 26 Oct 2026). The first code slice is local change tracking. Design: [`docs/systems/lighting/localized-invalidation-design.md`](../systems/lighting/localized-invalidation-design.md). Departures are in the lighting system record, section Local change tracking.

## Context

One global stillness hash switches every GI pass on or off. On stress/hall that means 0.43 ms frozen versus about 20 ms as soon as three boxes move (`/home/box/shared/moving-lighting-breakdown-2026-10-20.md`, master `27228dc4`). Rev 1 proposed screen dirty tiles with a full-frame fallback. Anthony's review: in real play the camera spins while things move, so screen tiles go fully dirty and the fallback is unbounded. He wants a hard static upper limit with maximum reuse.

## Decision

1. The persistent **surface light cache** is the reuse layer. It is extended, not replaced by GIBS surfels: hash LOD levels with hysteresis, a persistent coarse layer, merge-coarsening on eviction, and parent seeding on return.
2. Every expensive pass has a per-preset **work-unit cap** (rays, relights, spawns, coarsen ops, fresh shadow rays, denoise items). Work beyond the cap is deferred by priority. **There is no full-frame fallback.** Under load, values get older but are always finished. The millisecond cap is per preset. A later controller scales the work units to hold that cap. Phase 0 only records the pass times those caps are calibrated from.
3. A **priority scheduler** (change magnitude from change records, near-camera error, age, new visibility, inconsistency, plus a reserved starvation slice) replaces the global stillness hash. An optional game-side importance hint is a later phase, not phase 0.
4. Screen space does only fixed-size resolve work: apply, budgeted gather, ReSTIR DI resampling, compose, and one denoise iteration. Low and Medium may reuse a cached shadow. Fresh shadow rays prefer near-camera pixels.
5. The coarse-layer cap is fixed per preset (High 24 MB, Medium 16 MB, Low 8 MB). When it is full, evict the farthest coarse entries, then the least recently seen on a distance tie.

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

The budgeted frame is in the engine as well. Presets use `--lighting-schedule budget`. Work caps, a controller on the recorded lighting total, a guard-band crop, and shader gates for direct, à-trous, and trace are pushed each frame. Empty cells can read or claim a parent from 25 cm through 4 m. `global` pushes zeros.

The GPU does not build the §5.2 histogram, does not merge a retiring fine entry into its parent, and does not evict coarse cells farthest-first. The hash table stays about 368 MB, over the ~200 MB budget. The departures are in the lighting record under Budgeted refresh and in design §16.
