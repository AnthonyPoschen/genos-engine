# 0017: Flagged static lamps keep a baked indirect layer

Status: **Proposed**. Design: [`docs/systems/lamp-layering-design.md`](../systems/lamp-layering-design.md).

## Context

Anthony's plan of 8 Oct 2026: probes never throw a value away; a change blends and settles in 100–200 ms; a point lamp can be marked fixed; a fixed lamp gets a baked layer that is re-baked only where the scene changed (`docs/systems/lighting/history.md`). The same note calls that smart layering.

[ADR 0012](0012-gi.md) already stores indirect light in a world-space cache and reads only finished values. [ADR 0013](0013-lighting-invalidation-is-local.md) caps every expensive pass and keeps a detail level on each cache entry. The in-flight budgeted refresh is `cursor/lighting-p3-capped-refresh`. This decision does not change that branch.

`docs/systems/gi-design.md` gave every moving lamp its own layer and put every static lamp in one shared static layer. A moving lamp's indirect field is not a stable multiply, so that private layer would be rebuilt whenever the lamp moved. The 8 Oct plan is the other way around.

## Decision

1. **Fast-blend comes first**, on the one **live layer**. The live layer is today's indirect cache. Every lamp that does not have a lamp layer writes it: moving lamps, unflagged static lamps, the sun's indirect, and emissive. After the multi-scale mean estimator (Barré-Brisebois et al., Ray Tracing Gems, chapter 25) says the short mean has moved, the published long mean closes on that short mean with time constant τ. High and Medium use τ = 50 ms. Low uses τ = 100 ms. Potato does not run it. The 0.9 lamp-delta cap in the current shader is not part of this path. Readers see the long mean only.
2. **A static point lamp may set a flag** that gives it a **lamp layer**: its own finished indirect irradiance, baked at unit white on the cache entries inside its reach. Off, on, dim, and a glow are a **layer weight**. They do not trace rays. **Layer combine** writes `published = live + Σ weight × lamp layer` before any probe or pixel reads. Direct light stays per pixel and uses the same weight.
3. **Caps.** The layer pool is a ceiling inside the ~200 MB lighting budget: Potato 0, Low 1.5 MB (4 lamps), Medium 5 MB (8 lamps), High 18 MB (16 lamps). A flagged lamp past the count cap or its byte cap stays on the live layer. Samples live on the cache entry's existing detail level. Coarsening merges them into the parent; a returning fine entry starts from that parent. They are not a second LOD scheme.
4. **A bake stays hidden** until every sample it will publish is a finished estimate. The old published sum stays up during the bake, so the picture cannot show a half-traced layer or count the lamp twice.

## Departures

- **MSME listing.** The catch-up weight gains a time floor `1 - exp(-dt / τ)` only after the listing's own catch-up input is positive. The listing has no clock.
- **Enlighten** (Martin and Einarsson, SIGGRAPH 2010). We take the linear scale of a per-light indirect term. We do not take edit-time lightmaps or the CPU radiosity solver. The bake is this engine's tracer, inside the ADR 0013 ray cap.
- **gi-design.md light layers.** Moving lamps do not each get a layer. Unflagged static lamps share the live layer. The flag is opt-in.
- **Current shader.** The 0.9 lamp-delta cap is a departure already recorded in ADR 0012. Fast-blend removes it on this path instead of raising it.

## Consequences

The grader is `layering-gate` in `genos-lighting-ref`. Planted pictures prove the rows. `--capture DIR` scores `frame-NNNN.png` from the engine. Without a capture, and for any layer counter the engine does not have yet, the row is `NOT-IMPLEMENTED`. `pop_magnitude` does not see a uniform step; `fade_deviation` does. Shipping either phase still requires the rhai gates and a path-traced grade. A phase that misses its gate is dropped.
