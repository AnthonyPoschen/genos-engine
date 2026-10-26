# 0019: Specular reflections, mirrors, portals, and water

Status: **accepted** (Engine bot, 2026-10-26). Anthony Poschen delegated design approval to the Engine bot, which accepted this ADR. Design: [`docs/systems/reflections-design.md`](../systems/reflections-design.md).

## Context

[ADR 0012](0012-gi.md) stores roughness and metallic and leaves specular reflections and mirrors as a non-goal. The [rendering extras plan](../systems/rendering/extras-plan.md) draws water as a transparent pass that does not write the G-buffer, and it says the screen probes are the reflection. The picture is still Lambert. A mirror, a portal, and a glossy surface have no path that can be graded against the path tracer.

[ADR 0013](0013-lighting-invalidation-is-local.md) is the cost model this work has to live inside: a world-space surface cache, finished values only, a per-preset work cap, and no full-frame fallback. The hardware floor is unchanged: Vulkan 1.3, 4 GB VRAM, Low tuned for a GTX 1060 6 GB and an RX 580, `VK_KHR_ray_query` optional, macOS on the software tracer.

## Decision

1. Glossy reflections are a Lumen-style stack: one screen-space march along the specular direction, a world-cache read when that march misses, and an optional trace through the shared tracer (`VK_KHR_ray_query` when the device has it, otherwise the software SDF). The Low preset schedules zero reflection rays.
2. The specular term stores a hit, not radiance. The hit is re-shaded from the surface cache in the same frame. One spatial filter runs. There is no radiance history, so a step change is not lagged by an exponential blend.
3. Planar mirrors and portals are extra raster views. One shared cap bounds them (Potato 0, Low 1, Medium 2, High 3). A further view is emitted only when the running Fresnel product stays at least 0.02 and the cap still has room. The near plane is the mirror or portal plane. Geometry behind that plane is rejected.
4. Water is a Gerstner mesh in world space. Shading is exact dielectric Fresnel, one Snell direction, and Beer–Lambert absorption. Caustics are off on Potato and Low, an image-space sun map on Medium, and that map plus an optional cache confirmation on High. Wave height at the shore is clamped to `0.75 × depth`.
5. Reflection cost is a ceiling inside the frame target. Potato stays at the classify check (0.05 ms, 0 MB). Low is 0.40 ms and 8 MB. Medium is 0.70 ms and 16 MB. High is 1.00 ms and 32 MB. Work past a cap waits. It is not run as a full-frame fallback.

Acceptance lifts two limits and no others: the specular-and-mirrors non-goal in ADR 0012, and the water paragraph in the extras plan that says the probes are the reflection. Those two edits landed with this acceptance. ADR 0012's other rules stay: stored light does not follow the camera, no radiance history, ray queries rather than ray-tracing pipelines, the software tracer on macOS, Vulkan 1.3, the 4 GB floor.

## Rules kept

Finished values only, blended once. The camera orders work and does not resize the cache. Lighting VRAM stays inside the ~200 MB soft budget; the reflection caps above sit inside it. Nothing depends on a minimum ray count. Near-camera error is the first term in the priority. Debug-menu rows are the setting keys, with no help text. A tool takes a command-line argument; an environment variable only mirrors it.

## Rejected

- **Radiance history for glossy hits** (Lumen's temporal reflection reuse; SVGF's temporal accumulation). It is the lag and the ghost ADR 0012 already refused for GI.
- **Schlick Fresnel on water.** At IOR 1.333 the max absolute error against the Fresnel equations is 0.059, which misses the 0.02 gate.
- **A reflection ray on Low.** The floor cards are compute-bound. The fallback is the screen march plus the world cache.
- **Hit lighting** (Lumen's optional per-hit material evaluation). The ray reads the finished surface cache.
- **Stochastic lobe samples every frame** (Stachowiak 2015 as the only gather). One direction per pixel, fixed by the normal and the view.
- **An unbounded mirror or portal recursion.** The view cap is the bound. A view that does not fit shows the world-cache reflection.
- **A scrolling caustic texture.** Flat water is graded against `1 − F0`. A texture that does not match that flux fails.
- **Lengyel's OpenGL projection listing copied into Vulkan.** The oblique near plane is the same construction. The matrix is derived for Vulkan clip space.

## Consequences

The P0 harness is `reflections-check` in `genos-lighting-ref`. Fresnel, Snell, GGX, Beer–Lambert, the mirror and portal maps, the flat-water transmittance, and the Fresnel view budget live in `genos-math` (`optics`), with a GLSL mirror at `crates/render/shaders/reflections.glsl`. Live grades those functions. The CPU picture grades `portal_leak`, `shoreline_ratio`, `shoreline_deep_ratio`, `near_weighted_rel`, `flicker_reversal`, and `low_reflection_rays` against an f64 Whitted oracle that does not call `optics`. `ssr_depth_rel`, `caustic_rel`, `ms_frac`, and `vram_frac` stay `NOT-IMPLEMENTED` on live. `denoise_lag_frames` stays `NOT-IMPLEMENTED` on every fixture until the reflection denoiser exists. A known-good candidate passes the implemented rows. A broken candidate fails them and exits 1. Glossy screen marches, the world-cache or `ray_query` fallback, and that denoiser wait on lighting phase 3 and do not edit the GI passes.

Each later phase is its own change, with the gates in the design. A phase that misses its gate is dropped.
