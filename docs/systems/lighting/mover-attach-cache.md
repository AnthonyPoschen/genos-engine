# Object-attached mover lighting cache

**Status:** in progress (stages A–E on `cursor/mover-attach-cache-ccf2`)  
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
| C | Scheduler integration + world-cell boost |
| D | Compose wiring; delete old attach path and 16-mover change-block list |
| E | Scaling scene (10/100/1000), spin gate 0.027, design note |

## Networking

- Replicated state: none (client lighting cache).
- Authority: local renderer.
- Prediction / rollback: n/a.
- Determinism: same inputs → same pool layout given a fixed insert order.
- Headless server: pool may be zero; no GPU upload.
