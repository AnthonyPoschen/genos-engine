# Object-attached mover lighting cache

Status: **GPU path live** (Stages A–I + #286 pool port; Stage J lavapipe numbers in the PR).

## Intent

Moving objects must carry their own lighting records so a spin or slide does not smear world cells and re-converge every frame. Static geometry stays on the world lattice (ADR 0012). GIBS attaches every surfel; we attach **movers only**.

## Design (approved)

1. **Side pool.** Fixed pool of small (~64 B GPU) mover texels — not fat ~392 B slots in the shared world hash. High ≈ 32 768 texels (~2 MiB), scaled per preset (`surface_cache.mover_texels`).
2. **Object-local position + object id.** Lighting stored in **world directions** so a spinning mover stays correct. Skinned meshes use the top-weight bone (later).
3. **Tiers with hysteresis.** Near ≤ ~384 texels, Mid ≤ ~24, Far = 0 (reads world cells, never writes them). Pool pressure shrinks every share (1000 crowded Near → ~32 each on High).
4. **O(1) mover table** by object id. Replaces the 16×4-vec4 hitch list in the change block. Free texels when a mover is dropped.
5. **World boost.** Cells in 1 m bricks around the mover's swept bounds get a fill priority boost; movers too small at their distance dirty nothing.
6. **One scheduler.** Mover texels are ordinary fill candidates (change, distance moved, lamp proximity, closeness). Compose reads mover texels for mover pixels and world cells elsewhere.

## Departures from GIBS

- Movers only (not all geometry).
- World-direction lighting in the texel (GIBS keeps local frames for position; incoming light is what changes).
- Discrete Near/Mid/Far tiers instead of continuous screen-relative radius.

## Networking (rubric)

- Replicated state: none yet (local lighting cache).
- Authority: simulating client / server later owns mover transforms; cache is derived.
- Prediction / rollback: not required for the cache; invalidation follows transform authority.
- Determinism: CPU table ops are deterministic; GPU fill order must stay budget-stable.
- Headless server: pool can be zero; no compose.

## What landed

| Stage | What |
|---|---|
| A | `MoverCache` pool + O(1) table + unit tests |
| B | Tiers, hysteresis, pressure share, free-on-drop; `surface_cache.mover_texels` |
| C | Fill candidates + swept 1 m brick boost |
| D | Delete object-local shared-hash salt; stop change-block hitch pack; host sync into `MoverCache`; unhold count kept in head.w |
| F | GPU SSBO (binding 15): header + table + texel pool; dirty-range upload |
| G | G-buffer object id; compose O(1) table + hashed span read |
| H | Shared fill list; cache pass deposits world-direction lighting from the lattice |
| I | Compose: O(1) mover read; Far → world cells only; hitch scan off when attach_movers |
| #286 port | Presence list; hit never writes world; drop on-mover / stale change-volume cells; pool slide test |

## Stage J / #286 numbers (lavapipe)

- Slide: before max step **1.7008** at 250 mm; after **0** (pool fine `[7,3,1]`). world_hit 0, object_hit 1, drops 1.
- Rays/frame at 10/100/1000 Near movers (budget 4096): **4096 / 4096 / 4096**.
- Compose SPIR-V: **246728** B (ceiling 248000).
- ReSTIR `REF_ERR` 0.05 gate scenes: pass (merge oracle still fails on lighting-v3).
- fill_knockout: early→late error 0.8244→0.5557 (gain ok). Knockout late ceiling 0.213 is a doc note, not a code constant.
