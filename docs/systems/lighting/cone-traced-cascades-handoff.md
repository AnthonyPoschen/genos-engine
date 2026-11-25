# Cone-traced cascades: state, open faults and plan (25 Nov 2026)

A handoff for the next agent on branch `lighting/cone-traced-cascades` (pushed at `655547f6`). Read `AGENTS.md`, `docs/agent-rubric.md`, `docs/lighting/GLOSSARY.md` and `docs/systems/lighting/cone-traced-cascades-plan.md` section 18 first. This file says what is known, what is not, and what to do next.

## Status, 26 Nov 2026

Superseded in part by `cone-traced-cascades-plan.md` section 19, which has the results. In short:

- Fault 1 (unlit patches) had three causes, each found with the new `cone_pick` probe and fixed: the pass floor, the cone horizon, and the field seat at the front of its surface. Candidate 1 below was right for the notches; the black curtains were none of the four.
- Fault 2 (frame rate): per-stage GPU times exist (`cone_stats.pass_ms`). The frame at Anthony's pose went from 41.8 ms to 30.7 ms, not to 13. Section 19 lists what is left and why.
- `cones_closed` over `cones_traced` is not a fault: the two add up to probes times directions. A closed cone is one its chain closed before its interval; it is not traced.
- "Light through thin cloth" is the curtain's emblem before its texture loads.
- Gate G passes everywhere now. Gates D (Sponza) and E (hall) are open; see section 19.

## Desired outcome

1. **No unlit pixels.** Understand, then remove, the black (unlit) patches that pop in and out on surfaces, most clearly on Sponza's curtain folds. "Understand" comes first: the fix must name the mechanism and a probe must show it.
2. **Three times the frame rate in cone mode.** Anthony's editor, Sponza at 2526x1376, cone mode: 25 fps (39 ms a frame, worst 61 ms). The target is about 75 fps (13 ms) at the same view and quality. Cone mode must also cost no more than the old path at the same view.

Both without giving back what the gates hold: no flicker on a still camera (A), nothing dropped (B), stable seats (C), the same picture after turning away and back (D), lamps followed in a frame (E), no more pops than the old path when moving (F), no dip at a clip rebuild (G).

## Anthony's constraints (from this session)

- The direct light is every frame. Only the bounce may keep a short history (now 0.1 s). No cost per lamp.
- Correct over fast: no shortcut that changes what is lit. For a pixel that sees the sun, a direct ray for crispness is fine. Everything else, bounce included, uses the field's aggregate.
- Keep the screen-space voxel layer (probes placed by pixels). Off-screen surfaces get much coarser world probes, and those must cost far less than the screen probes.
- `theta` 2 is the default. `finest_probe` stays at or under 0.25 m.
- Time steps by real frame dt, not frame counts.
- Headless only: no windows or pointer on his desktop. Run renderers under `systemd-run --user --scope -p MemoryMax=12G -p MemorySwapMax=0`. Write evidence under `./genos-debug/`, never `/tmp` (it is RAM). Do not poll with `pgrep` loops.
- Anthony's editor listens on port 8765. You may read from it (`camera`, `capture`, `timing`, `cone_stats`). Do not change its settings without asking. Headless timing is worthless while it runs (it holds the GPU).
- Run `scripts/lint.sh`, `tools/lighting/cone_stability_gate.py` (all four views) and `scripts/vk-validation-gate.py` (on lavapipe with `GENOS_MCP_PORT` set away from 8765) before pushing.

## How the system runs (one frame)

Field (CPU thread, `crates/render/src/cone_field.rs`): voxels at levels 2-9 (6.25 cm to 16 m). Levels 2-4 are clip volumes, 128 voxels across, about the eye; they are rebuilt when the eye moves 2 m. Each voxel face (6 ways) keeps its albedo, area, light and plane. Fine levels (2-3) keep a point box per face. Solids that moved in the last 2 s are left out.

GPU passes (`crates/render/src/cone_gpu.rs`, `CONE_PIPES` = 11; shaders in `crates/render/shaders/`):

| Pass | Shader | Work |
|---|---|---|
| light | `cone_light.comp` | Every field face: light = albedo x (last frame's probe irradiance + sun x sun share), blended `1 - exp(-dt/0.1 s)`. Also re-marches the sun share of 1/8 of the faces each frame (`sun_blocked`). |
| insert | `cone_insert.comp` | Every pixel puts its cascade-0 probe (own cell, its face way) and the 8 trilinear neighbours that have a field surface into the hash. |
| world | `cone_world.comp` | A coarse probe on each field face within 16 m with no probe (2 m and 4 m cells), 1/4 a frame. |
| compact | `cone_compact.comp` | Hash slots become probes and are seated on their face's field surface. With no surface: the voxel centre, flagged unseated. |
| spread | `cone_spread.comp` | Each probe puts its parent (and below `CONE_PARENT_ONLY` 8 neighbours) into the next cascade. Unseated probes put their parent only. |
| chain / trace | `cone_chain.comp`, `cone_trace.comp` | Each probe traces one cone per direction through its cascade's interval in the field. |
| weights / merge | `cone_weights.comp`, merge | Each probe reads its parent cascade's probes; cascades merge from coarse to fine. |
| lamps | `cone_lamps.comp` | Lamps written into a lamp hash each frame; cones pick them up. |
| shade | `cone_shade.comp` | Each pixel reads the probes about it (`cone_weights`, two levels blended), adds the sun (the field's sun share, and its own `sun_blocked` march when the share is partial). |

## Numbers now

| Where | Cone mode | Old path | Source |
|---|---|---|---|
| Anthony's editor, Sponza, 2526x1376 (pose 11.35, 1.8, -0.98, yaw -1.435) | 39.3 ms (25 fps), worst 61 ms | not measured at that pose | `timing` on port 8765 |
| Headless hall, 1920x1080 | 18.0 ms | 9.1 ms | `tools/lighting/cone_cost.py --view hall` |
| Headless Sponza, 1920x1080 | 29.6 ms | 22.1 ms | `cone_cost.py --view sponza` |

The headless rows predate the last four fixes (coarse cover, movers, seat preference), which were not timed. Editor `cone_stats` at that pose: cascade-0 probes 16,436; cascades 1-4 have 8,182, 3,355, 922 and 268 probes. Cones traced are 131k, 159k, 220k, 55k and 54k. The field is 17.1 M points, 377,827 voxels and 215 MiB, built in 1.05 s.

Timings recorded earlier the same day, for reference: the world layer cost Sponza about 4 ms (9 ms before it went coarser). Far views cost 3x before the coarsest-cascade cap.

**What is not known:** the time of each pass. `timing` reports `gpu_ms: null`, and there is no GPU timestamp per cone pass. The earlier per-pass figures came from a temporary env switch that skipped passes; it has been removed.

## Fault 1: unlit (black) patches

### Evidence

- Sponza pose B (`-1.53241, 1.8, -0.39176`, yaw 4.7178, pitch -0.102), the blue curtain on the right. Captures are in `genos-debug/curtain/`:
  - `curtain-cone-old.png` (left cone, right the old path)
  - `sun-nosun.png` (left sun on, right sun off, brightened 2x)
- Small black notches run down the curtain's folds in a vertical line, plus a darker lower fold. The old path has none.
- They stay with the sun off, so they are probe light (bounce or lamp through cones), not the sun share.
- Anthony sees them pop in and out as the camera moves, and on the walk from pose A to B (pose files in `genos-debug/pops/pose-a.json` and `pose-b.json`). He reported the same behaviour earlier on other surfaces ("dark patches that go away when you move up close").
- Already fixed, for context:
  - Faces never lit took their light at once (`face_lit`).
  - The world layer now covers the off-screen voxels of cells the screen covers in part.
  - A pixel prefers seated neighbour probes of its way over its own unseated one (`cone_weights`, end). That fix removed a broad grey band on this curtain.
- `cone_stats` shows `seat_fallbacks` of 859, 227 and 49 on cascades 0-2 at Anthony's pose: probes with no field surface for their face way, seated at the voxel centre.
- The cascades also report `cones_closed` greater than `cones_traced` on cascades 3 and 4: 417k against 55k, and 495k against 54k. Either the counter means something else, or it counts more than once. Find out which.

### Candidate mechanisms (ranked; none proven)

1. **The pixel reads no probe.** `cone_weights` (`cone_common.glsl`) drops candidates in three ways: those not in the hash, unseated ones that are not its own cell, and, when `see` is set (`pc.lamp.x & 2`), it scales each weight by `cone_passing(pixel, seat)` through the field. On a fold, the seats of the neighbouring probes of the same way can lie behind the cloth's own fine planes, as seen from the pixel. All weights then reach 0, `level_irradiance` returns 0, and the pixel is black. A small move changes which level or probes the pixel reads (`cone_pixel_levels` blend), so the pixel pops.
2. **The probe's cones all close at once.** A seat placed in or under the cloth makes every cone start inside a covered voxel, and `cone_trace` writes closed cones dark. This happens with an unseated probe at a voxel centre, or with a seat lifted (`CONE_LIFT`) into the other layer of an S-fold, where one voxel holds two surfaces of the same way at different depths.
3. **Wrong plane on a fold.** `cone_seat_face` takes the face's mean plane in the voxel. Where a fold puts two layers of the same way in one voxel, the mean lies between them, inside the cloth.
4. **History.** The light pass holds a face's light while it has no probe (within world reach). A face seeded dark keeps it. This is less likely, because the notches pop with view, not with time.

### What to build to know

- **A pixel probe** (rubric: every failure mode has a probe). For example a `cone_pick` MCP command: from a screen point, return the pixel's levels and blend, each probe it considered, and for each one:
  - key, face and seat
  - seated flag
  - the `cone_passing` weight
  - whether it was kept
  - closed-cone count
  - mean radiance
- **A debug view** that colours each pixel by how many probes it read and their total weight. Zero is red.
- Then capture pose B headless, with the sun off, and pick a notch pixel. That separates mechanisms 1-3 in one look.

### Reproduce headless

```
# Sponza pose B, cone on; full frame into genos-debug/curtain/
genos-lighting --headless --menu hidden --scene sponza --size 960x540 --mcp-port 18794 --out genos-debug/curtain
# then: time pause; camera {position:[-1.53241,1.8,-0.39176], yaw:4.7178, pitch:-0.102}
#       lighting.set_settings {cone_cascades:on}; wait for cone_stats field not "coarse"; settle 3 s; capture
```

The helper used this session is `sponza_view.py` in the session scratchpad, which may be gone. The steps above are all it does.

## Fault 2: frame rate (target 3x)

### First: measure

Add GPU timestamp queries around each cone pass. Expose them in `timing` (`gpu_ms`, now null) and in `cone_stats`, and list them in `docs/systems/lighting/debug-standard.md`. Without them every item below is a guess. Measure with Anthony's editor closed, or ask him, at the editor size (2526x1376) and at 1080p, in the hall and in Sponza.

### Likely costs and options (to confirm by measurement)

| Cost | Why it may be large | Option | Keeps the constraints? |
|---|---|---|---|
| Sun share re-march, 1/8 of all faces each frame | Up to about 380k voxels x 6 ways. Each march steps 25 cm across the 32 m volume (to about 128 steps), and each step tests up to 8 voxels x 6 faces. | The sun share changes only when the sun turns or the field is rebuilt. Compute it once per field build: on the build thread with the existing CPU mirror `ConeField::sun_blocked`, or once on the GPU. Re-march only when the sun direction changes. | Yes. Same values, not recomputed. |
| Pixel sun march | Every pixel whose faces see part of the sun marches its own ray from 6.25 cm. | Cap the reach and start at the pixel's level past the first metres. Or march only pixels near a shadow edge (shares between, say, 0.1 and 0.9). | Yes, if edges stay crisp (check gate F and captures). |
| Light pass over every face, every frame | 377k voxels x 6 at Anthony's pose. | Update the faces within the world reach every frame, and the rest a share a frame. The bounce already has a 0.1 s history. | Yes. Bounce history only. |
| Cascade traces | About 620k cones a frame at Anthony's pose. | Trace cascades 2+ every 2nd or 4th frame (bounce, not direct). Fewer cascade-0 probes: `finest_probe` 0.125 (Anthony earlier saw no visual change; confirm the setting applies) or a larger `probe_px` at 3.5 MP. | Direct light must stay every frame: check that lamps through cones are not delayed (gate E). |
| World layer | About 1-4 ms in Sponza. | 1/8 a frame instead of 1/4, or one cascade. | Yes. |
| Insert and shade at 3.5 MP | Per pixel; insert does 9 hash inserts a pixel. | Insert at half resolution, or once per probe tile. | Check gates C and F. |
| `cones_closed` greater than `cones_traced` on cascades 3-4 | Possibly wasted work, or a counting bug. | Find out first. | — |

Report each change's ms in the plan's section 18, as earlier steps did.

## Other open faults

- **Pops while walking.** On the walk from A to B (Sponza, 4 cm steps, 960x516, exposure fixed), cone mode pops a mean of 19,094 pixels a step (worst 76,782). The old path pops 9,548 (worst 42,877). Both peak mid-walk (x about 1.7). The clip rebuilds (steps 50, 99 and 149) are not among the worst steps. The script follows gate F's rule; its results are in `genos-debug/pops/walk-on/pops.json` and `walk-off/pops.json`.
- **Light through thin cloth.** With the sun off, grey cell-sized blocks show on the blue curtain at pose B. A lamp hangs behind the curtains. The coarse cascades' cones likely see through the cloth, because a coarse voxel averages both sides. A fix is a design choice; one option is to stop a coarse cone at a two-sided surface. Ask Anthony.
- **Gates at Sponza 1262x1376.** D fails: 3 pixels over two codes, 27 over one code. G fails: a one-frame dip at a clip rebuild (frame 24); it failed before this round too. All other gates pass at all four views (`genos-debug/gates/`).
- **Movers.** A moving solid casts no cone shadow and sends no bounce. Its own light shows voxel-sized steps.
- **A failing test.** `genos-scene-program` test `a_scene_load_returns_inside_the_pump_budget` fails locally. It failed at the start of this session too; it may be timing under GPU load. Not checked.

## Tools

| Tool | Use |
|---|---|
| `tools/lighting/cone_stability_gate.py --view hall\|sponza --size WxH [--only A,...]` | Gates A-G |
| `tools/lighting/cone_cost.py --view ... --size ... [--pose X,Y,Z,YAW,PITCH] [--set k=v]` | Cone mode against the old path, ms |
| `tools/lighting/cone_grade.py` | Against a CPU path-traced reference (slow: `--width 64 --block 4`) |
| `crates/render/tests/cone_energy.rs` (`-- --ignored --nocapture`) | Lamp, bounce, sky and ceiling energy against the old path |
| `genos-single-frame-cascade --scene S --sun-check --eye X,Y,Z` | The field's sun march against the exact ray, with a map in scenes without triangles |
| `genos-single-frame-cascade --renderer-field --eye X,Y,Z` | Field clip volumes, build times and world faces |
| MCP: `cone_stats`, `cone_probes {cascade}`, `capture {mode}`, `timing {frames}`, `camera`, `objects`, `lights` | |
