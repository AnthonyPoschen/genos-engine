# Object-attached mover lighting cache

**Status:** stages A–E landed on `cursor/mover-attach-cache-ccf2` (GPU texel fill
and G-buffer object-id compose read still open; see Left undone).  
**Paper:** GIBS (Halén & Hayward, SIGGRAPH 2021) §1.5 transform/bone attachment  
**Supersedes:** object-local keys in the shared world hash table behind
`surface_cache.attach_movers`, and the 16-mover list packed into the 64-vec4
change block.

## Intent

Moving objects keep their own small lighting texels instead of fighting the
world hash table for 392 B slots. Compose reads mover texels on mover pixels
and world cells everywhere else. One scheduler owns both.

## Design

1. **Pool.** Fixed pool of ~64 B texels (object-local position, object id,
   world-direction SH-L1). High defaults to 32 768 (~2 MiB). Scaled per preset
   via `surface_cache.mover_pool`. Not full world-hash slots.
2. **Attachment.** Like GIBS: store object-local position + transform/object id
   (G-buffer). Skinned meshes use the top-weight bone. **Departure:** lighting
   lobes are world directions, so a spinning mover stays correct without
   rotating stored irradiance.
3. **Tiers.** Near ≤384 texels, Mid ≤24, Far = 0 (reads world cells, never
   writes them). Hysteresis on screen size. When the pool is full, every
   mover’s share shrinks (e.g. 1000 on screen ≈ 32 each at High). Layout can be
   shared per mesh; lighting is per instance.
4. **Table.** O(1) lookup by object id, scales to 1000+. Dropping a mover frees
   its texels immediately (no orphans after the keep TTL).
5. **World effect.** World cells within 1 m of the mover’s swept bounds get a
   priority boost (DDGI-style). Movers too small at their distance dirty
   nothing.
6. **Scheduler.** Mover texels are ordinary candidates in the existing
   screen-tile / F-stack budget. Priority from measured change, distance moved,
   lamp changes, and closeness.

## Out of scope

Denoiser unhold (whole-picture vs tile-mask, `LIGHTING_MOVER_KEEP`, catch tail)
stays on the other agent’s PR. This path must not edit those sites.

## Stages

| Stage | Deliverable |
|---|---|
| A | Pool + mover table + O(1) lookup + unit tests + `mover_pool` setting |
| B | Allocation, tiers, hysteresis, free-on-drop |
| C | Frame sync into the pool; world-cell priority boosts enter the change list for `lighting_near_change` / compact |
| D | Delete object-local hash + 16-mover change-block list; head.w keeps live count for unhold; compose uses world cells for movers (detail) until mover-texel SH fill lands |
| E | Scaling scene (10/100/1000), spin gate 0.027, design note |

### Observability (stage C)

- `Lighting.mover_stats`: `[live_movers, live_texels, world_boosts]`
- Debug log (`GENOS_LIGHTING_DEBUG`): prints pool use and boost count with the mover line

## Networking

- Replicated state: none (client lighting cache).
- Authority: local renderer.
- Prediction / rollback: n/a.
- Determinism: same inputs → same pool layout given a fixed insert order.
- Headless server: pool may be zero; no GPU upload.

## Scaling (CPU pool, High 32768 texels)

From `mover_cache::tests::scaling_ten_hundred_thousand_movers`:

| Movers | Live texels | Share / mover | World boosts |
|---:|---:|---:|---:|
| 10 | 3840 | 384 (Near cap) | 10 |
| 100 | 32700 | 327 | 100 |
| 1000 | 32000 | 32 | 1000 |

Rays/frame and GPU dirtied-cell counts need the owner-PC gate script
`debug/lighting/scripts/gates/mover-pool-scale.rhai` and `gi-check` spin
(limit 0.027).

## Left undone

1. **Mover-texel SH fill** in the cache pass (scheduler candidates that write
   the 64 B pool). Compose still shows movers via the world-cell detail read.
2. **G-buffer object / transform id** so compose can O(1)-lookup mover texels
   per pixel (GIBS §1.5). Table lookup by id is ready on the CPU side.
3. **GPU upload** of `pack_table` / `pack_texels` into a shader-visible buffer
   (binding or work-buffer region).
4. Owner-PC: spin grade ≤ 0.027; knockout late ≤ 0.213; ReSTIR `REF_ERR` 0.05
   unchanged by this change.
