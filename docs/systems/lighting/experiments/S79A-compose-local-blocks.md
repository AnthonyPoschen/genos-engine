# S79A: Compose dirty blocks while the scene moves

- **Date:** 2026-10-13
- **Result:** no moving-scene skip on the RTX 4070 (compose stayed ~1.9 ms). Counters below.
- **Commit / patch:** this commit

## Idea
The 8×8 compose skip was gated on a global scene hash. A moving box or sun changed that hash, so compose redrew all of 1920×1080. Mark blocks from the inputs compose reads, and dispatch that list while the scene moves.

## Motivation
`docs/systems/rendering/perf-review.md`: compose is 2.10 ms on the moving stress scene. The review's gap for this pass is about 1.60 ms (2.10 down toward 0.50) when only the dirty blocks run.

## Paper basis
Lumen's screen-probe composite (Wright et al. 2022, four probes, plane distance and normal) is unchanged. SVGF (Schied et al., HPG 2017) and Lumen composite every pixel every frame. Skipping unchanged blocks is the same idea as the dirty-page a-trous (S77D, S78X). No paper specifies this compose skip.

## What was implemented
The CPU full compose remains for the first frame, a resize, a world mode other than the surface cache, a picture with more than 65,535 blocks, and `GENOS_LIGHTING_COMPOSE_ALL`. `Renderer::set_lighting_compose_all` is the test override. The scene hash is no longer a reason.

Producers mark a block when a value compose reads changes:

- direct light, when the geometry (1 mm position, normal, albedo, sidedness), the packed direct light, or the emission differs. Lamp epochs are not part of that word. A lamp or sun move sets one bump word so pixel reuse still relights, and compose marks the pixel only if the packed light or the geometry changed. A pixel that lost its surface marks its block.
- the probe filter, when the SH it writes differs (unchanged)
- the surface-cache push, when the packed filtered field differs (unchanged)

One prepare group runs before the indirect dispatch. A non-empty list stays one group per dirty block. An empty list, or a list past the cap, sets the full-image dispatch. A frame is never held, and a whole frame is never reused as the picture. Clean blocks keep the shaded pixels already in the output buffer. An empty pixel inside a dirty block writes the toned sky, which is the raster clear.

`lightingcompose` (debug trace) and `Renderer::lighting_compose_stats` report whether the dispatch covered the image, how many 8×8 blocks it shaded, the image block total, and whether an empty or over-cap list was promoted (`fallback` 1).

## Deviations
- SVGF and Lumen composite every pixel from this frame's inputs. Here a clean block keeps its stored shaded pixel. That reuse is inside the frame's output buffer. It is not a held frame.
- A frame with nothing dirty pays a full compose (about the old 2.1 ms) instead of launching zero groups. The previous quiet path held the picture; holding a whole frame is out.
- The direct-light mark ignores lamp epochs. A sun that does not change a pixel's packed light does not redraw that pixel. The light pass still runs for that pixel when the bump word says a lamp moved.

## Numbers
Owner, RTX 4070, 1920×1080, two runs against `9e08d7d`: moving compose 1.89–2.07 ms before, 1.91–1.95 ms after. No change. No visible artifacts.

The moving stress scene should not have a skippable set. 1920×1080 is 240×135 = 32400 blocks, under the 65535 cap, so a fully dirty frame stays a list (`full` 0, `fallback` 0) and still shades every block. The sun is directional, so a scene change bumps its epoch and the day clock moves it every frame. Packed direct light then changes on sunlit pixels, and a probe whose SH differs marks the three-tile neighbourhood compose samples. Either one covers the image. The 320×180 hall test skips blocks because that scene has no moving sun.

The same list is why a-trous rose 0.4–0.7 ms on the moving scene in the compose-only commit, which does not change `lighting_atrous.comp`. A full compose used to clear each texel's screen rect before the passes. The push reads that rect and returns when it is empty. The list path leaves the rects up, so the push walks them. That walk sits in the last a-trous stamp. The prepare group and the indirect compose are the compose stamp.

## Screenshots
not recorded

## Outcome and why
The lavapipe test moves a box, checks the frame is on the list with `still_frames == 0`, then checks that picture against a full compose of the same world. A new sun must also stay off the hash gate.

## Lessons / when to revisit
Do not tune this skip unless `lightingcompose` shows `blocks` well under `total` on the moving stress scene. The expected line is about 32400/32400, `full` 0, `fallback` 0. A ratio under about a half, with a-trous levels actually on `list`, is the first sign a skip exists.
