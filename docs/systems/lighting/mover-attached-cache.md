# Object-attached mover lighting cache

Status: **removed**. The texel pool, its struct, its buffer, and its shader are gone. A mover is a level-0 G-buffer sample.

## Intent

Moving objects must carry their own lighting records so a spin or slide does not smear world cells and re-converge every frame. Static geometry stays on the world lattice (ADR 0012). GIBS attaches every surfel; we attach **movers only**.

## Design (approved)

1. **Side pool.** Fixed pool of small (~64 B GPU) mover texels — not fat ~392 B slots in the shared world hash. High ≈ 32 768 texels (~2 MiB), scaled per preset (`surface_cache.mover_texels`).
2. **Object-local position + object id.** Lighting stored in **world directions** so a spinning mover stays correct. Skinned meshes use the top-weight bone (later).
3. **Tiers with hysteresis.** Near ≤ ~384 texels, Mid ≤ ~24, Far = 0 (reads world cells, never writes them). Pool pressure shrinks every share (1000 crowded Near → ~32 each on High).
4. **O(1) mover table** by object id. Replaces the 16×4-vec4 hitch list in the change block. Free texels when a mover is dropped.
5. **World boost.** Cells in 1 m bricks around the mover's swept bounds get a fill priority boost; movers too small at their distance dirty nothing.
6. **One scheduler.** Mover texels are ordinary fill candidates (change, distance moved, lamp proximity, closeness). Compose reads mover texels for mover pixels and world cells elsewhere.

## Scope this stage (yaw-only, boxes-first)

This stage is **yaw-only** and **boxes-first**. That is intentional and safe for the live analytic box occluders; it is not full rotation and not mesh-surface sampling.

| What | Today | Safe? |
|---|---|---|
| **Yaw** (heading about Y) | Host stores `yaw_cos` / `yaw_sin`. Compose converts world→local with the inverse yaw before the hash lookup. Fill transforms local→world with the same yaw. | Yes — matches box occ yaw. |
| **Pitch / roll / tumble** | Not stored. No pitch/roll in the mover record or in `lighting_mover_world_to_local`. A tumbling box keeps the last yaw frame only. | Safe for current analytic boxes (yaw-only motion). Wrong for a pitched or rolling mover until full-rotation lands. |
| **AABB-surface texels** | Host layout places texels on the six faces of the local AABB with outward normals, hashed into compose buckets. | Correct for boxes. Spheres float off the true shell (AABB shrink-wrap). |
| **Mesh movers** | Hitch packs centre from world bounds and **identity yaw**. `half_extents` is left zero → layout clamps each axis to `LIGHTING_SC_TEXEL` (a 25 cm cube). No mesh-surface samples. | Safe (does not crash or smear world cells); lighting quality on mesh movers is a coarse stand-in until mesh-surface texels land. |

### Named follow-ups

1. **Full rotation** — store a full orientation (or pitch/roll with yaw); world↔local uses that frame for lookup and fill. Required before pitched / tumbling movers are trusted.
2. **Mesh-surface texels** — spawn and look up on the mesh (or SDF) surface with real half-extents / bone frame, not the 25 cm AABB stand-in.

## Departures from GIBS

- Movers only (not all geometry).
- World-direction lighting in the texel (GIBS keeps local frames for position; incoming light is what changes).
- Discrete Near/Mid/Far tiers instead of continuous screen-relative radius.
- This stage is yaw-only AABB boxes; GIBS attaches surfels on arbitrary oriented surfaces.

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
| #286 port | Presence list; hit never writes world. The on-mover and stale change-volume frees are gone: those cells keep their mean, and a cell the mover is changing takes the same 1-in-10 floor ray as a still cell |

## Stage J / #286 numbers (lavapipe)

- Slide: before max step **1.7008** at 250 mm; after **0** (pool fine `[7,3,1]`). world_hit 0, object_hit 1, drops 1.
- Rays/frame at 10/100/1000 Near movers (budget 4096): **4096 / 4096 / 4096**.
- Compose SPIR-V: **≤237848** B (ceiling 248000; dense face lookup + fill rotation; hitch mesh/on out of compose, lattice warm-up). Shader magic back to **309**.
- Warm-up locks: slide `warm_ok` = live gather; `compose_lattice_warm_up_prior_loads_stored_ids` = `GENOS_COMPOSE_NO_WALK` stored-id load.
- ReSTIR `REF_ERR` 0.05 gate scenes: pass (merge oracle still fails on lighting-v3).
- fill_knockout: early→late **0.5954 → 0.0828** (under frozen **0.213** ceiling). Gain gate `MIN_ERROR_GAIN=0.03` stays. The 0.5557 regression was stale-drop treating `last==0` as aged (wiped hall cells) + Stage H `prepare_fill` spending the batch on KEEP leftovers via closeness-only priority.

## Black / missing-triangle fixes (RTX 4070 / #288)

1. **Object-id collision (lighting):** G-buffer ids used the draw instance index (`ids.x == 0` → `instance + 1`), colliding with `MoverCache` keys. Compose returned full-trust pool irradiance on static pixels. Fix: `ids.x == 0` = not a mover; **stable draw→occluder/mesh KEEP id at submission** (not translation match); sphere gate before pool read; mesh hitch centres use world bounds (not `[0,0,0]`). G-buffer oid is depth-elected with distance. Slide `static_pool == 0`.

2. **Instance SSBO stride (geometry, the transparent/sky holes):** First bad commit `eb154989` added `ids: [u32;4]` to `InstanceRec` (128→144 B) but only `gbuffer.vert` grew. `scene.vert` / `reflections_portal.vert` kept a 128 B local struct → depth and G-buffer disagreed; triangles after instance 0 vanished. Validation: 0 errors (in-bounds misread). Fix: one shared `instance_ssbo.glsl` included by all three verts; CI `instance_stride` asserts GLSL size/offsets == `InstanceRec` and bans local `struct Instance`. Coverage + `GENOS_BUFFER_POISON` stay. The `gpu-integrity` job (`scripts/ci-gpu-integrity.sh`) runs that path under validation; house pods (16 GiB) were OOMKilled (exit 137). The script caps `LP_NUM_THREADS=1`, `CARGO_BUILD_JOBS=1`, `--test-threads=1`, `--release`, and `GENOS_LIGHTING_PRESET=potato` (High’s million-slot table is the default; lavapipe keeps freed High pages until destroy; validation still grades VUIDs only).

3. **Red-box dark bands (mover faces):** Compose returned `w = 1` whenever mover texel `samples > 0.5`, so underfilled pool texels painted full-trust dark patches. Trust now requires ≥ 8 sample sets before full-trust; below that, blend with a world prior just outside the surface along the normal (on-mover world cells are dropped, so hollow fallthrough stayed dark).

4. **Persistent blocky dark faces (orbit/spin @ 4070):** Root causes were (a) compose looked up `pos - centre` without inverse yaw, so rotating faces keyed the wrong hash buckets; (b) host layout was a volume grid with every normal = `+Y`, so side-face fill origins sat inside the mesh and self-shadowed; (c) fill ray origins had no outward bias. Fix: object-local lookup via inverse yaw, AABB-surface layout with outward normals, and a **scale-aware** fill bias (`LIGHTING_MOVER_RAY_BIAS_FRAC` × max(`LIGHTING_SC_TEXEL`, min half-extent)) along the world normal.

5. **Thin dark slit / crawling rectangular patches on the red mover (hall eye @ 4070, frames 180→400):** Tip `lighting-v3` @ `525bf44e` has **no mover texel pool** (`lighting_mover_texel_lookup` absent); lavapipe tip still shows a *mild* top slit / soft blockiness (`dark_frac` ≈ 0.003, `block_min` ≈ 25) — treat that mild grain as pre-existing, not fixed here. Severe near-black face rectangles (`dark_frac` ≈ 0.366, `crop_y<25` ≈ 0.62) are #288 pool-path: (a) **hash%count + first-writer**; (b) fill tied every texel to the same priority so a 256-deposit budget locked onto the lowest pool indices; (c) empty pool reads fell through into the on-mover world-cell drop; (d) face pick from `abs(local)` flipped at edges. Fix: **dense face-grid** layout + normal-preferred lookup, underfill-boosted rotating fill order, empty-texel → lattice prior only (`dark_frac` ≈ 0.201 on lavapipe f400). Regression: `dense_face_index_*` + `fill_budget_rotates_across_underfilled_texels` in `mover_cache.rs` (4070 `--shot` remains the visual gate).

## Review follow-ups (#288)

| Item | Fix |
|---|---|
| Stable draw id | `pack.draw_occluder` / `draw_object` → `mover_object_id_for_draw` |
| G-buffer oid | Write oid only when `dist_bits <= atomicMin` winner |
| Presence ≤64 | `MOVER_PRESENCE_CAP = 1024` |
| Knockout late | Stale age `>= 2` and never age-drop `last==0`; still frames skip mover fill |
| Host fill scan | One pool pass + still-frame early out |
| Knobs | `LIGHTING_MOVER_FILL_*` named constants |
| README | Compose pool read marked live |
| #290 tile-reach disc | Still reads change-list spheres. Mover table holds the **current** sphere only, not before/after. Do not replace the disc read until the table stores swept/before–after bounds; then mark from the table and keep the change-list path as fallback when `attach_movers` is off. |
