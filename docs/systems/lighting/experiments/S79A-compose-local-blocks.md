# S79A: Compose dirty blocks while the scene moves

- **Date:** 2026-10-13
- **Result:** pushed (not re-timed; the owner measures on the RTX 4070)
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

`lightingcompose` (debug trace) and `Renderer::lighting_compose_stats` report whether the dispatch covered the image and how many blocks it shaded.

## Deviations
- SVGF and Lumen composite every pixel from this frame's inputs. Here a clean block keeps its stored shaded pixel. That reuse is inside the frame's output buffer. It is not a held frame.
- A frame with nothing dirty pays a full compose (about the old 2.1 ms) instead of launching zero groups. The previous quiet path held the picture; holding a whole frame is out.
- The direct-light mark ignores lamp epochs. A sun that does not change a pixel's packed light does not redraw that pixel. The light pass still runs for that pixel when the bump word says a lamp moved.

## Numbers
Not measured here. Expected on the moving 1920×1080 stress scene, from the review's 2.10 ms full compose: about 1.6 ms saved when roughly a quarter of the blocks are dirty (2.10 down toward 0.50). Less when the sun changes most probe SH, because those blocks are on the list. A still frame whose list is empty pays the full compose and saves nothing.

## Screenshots
not recorded

## Outcome and why
The lavapipe test moves a box, checks the frame is on the list with `still_frames == 0`, then checks that picture against a full compose of the same world. A new sun must also stay off the hash gate.

## Lessons / when to revisit
If a moving sun retints every probe, the list is the whole image and this cut saves nothing. The owner's A/B is `GENOS_LIGHTING_COMPOSE_ALL=1` against the default.
