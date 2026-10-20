# 0013: Lighting invalidation is local, not one global stillness hash

Status: **proposed** (draft for Anthony's review, 2026-10-20). Design: [`docs/systems/lighting/localized-invalidation-design.md`](../systems/lighting/localized-invalidation-design.md).

## Context

`lighting_passes` hashes every lamp, occluder and the camera into one value. When that hash changes, `still` goes to 0 and every GI pass runs over the whole frame. The stress/hall breakdown on 2026-10-20 (`/home/box/shared/moving-lighting-breakdown-2026-10-20.md`, master `27228dc4`, relative numbers) found:

- frozen scene: lighting 0.43 ms after it settles
- three small moving boxes, no lamp moving: 19.95 ms
- boxes plus 2–3 moving lamps: 22.56 ms

So about 89% of the moving cost is a fixed on/off charge for "something moved somewhere". Only about 2.6 ms (mostly direct light) scales with how much moved. While the boxes move, stress/hall never reaches `still > 0`.

## Decision

Replace the global hash with **change records**. A change record is the old and new bounds of a mover, or a lamp's old and new position and reach. The records are turned into a **dirty tile** mask on screen and a **dirty list** of surface-light-cache pages in the world. Direct light, the denoiser, compose, gather, the probe filter and screen-probe tracing work only on dirty tiles or pages. Clean tiles keep their last finished value. In dirty tiles, direct light uses lamp pick with reuse (ReSTIR DI). The camera moving, a resize, or a dirty share above a threshold falls back to today's full-frame path.

## Rules this keeps

- Only finished values are published, and each is blended once (MSME / hold). A tile never shows a partial result.
- The stored light never depends on the camera. The camera only orders the work (ADR 0012).
- Lighting VRAM stays within the ~200 MB soft budget. The new state costs about 75 MB at 1080p.

## Rejected

- **Global hash with cheaper passes** (RAYCEIL, WORKCUT, S79 half-res). The cost is the area being redone, not the per-pixel price. Cutting rays made the moving frame slower (RAYCEIL +1.35 ms).
- **Detecting change only from samples** (pure A-SVGF gradients). The scene already knows exactly what moved. Gradients stay as a safety net, not as the trigger.
- **Per-slot scattered work lists as the only granularity** (S78A). Atomic order and poor locality cost more than they saved. Work goes in dense 16×16 tiles.

## Consequences

The full-frame path stays as the fallback and as the A/B reference (`--lighting-invalidation global`). Each phase in the design lands as its own PR with an exclusive still/moving A/B. A phase that does not lower the moving frame is dropped under the standing drop rule.
