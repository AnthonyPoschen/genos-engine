# S78S2: settle floor + neighbourhood re-check (rejected; patch gi2diag/g78/S78S2-settle-16-nbhd.patch)

Change:
1. Texels need 16 sets (was 8) and their target met at 1.96 standard errors before they settle or resolve.
2. Our A-SVGF adaptation: a resolved texel stores its neighbourhood's cache brightness (4 texels at
   +-3 cells). If that moves by more than max(3 SE, 2%), the move becomes the texel's lambda and the
   existing A-SVGF path shortens its history.

Test: comp5.sh (stuck repro with lamps 5, moved run vs never-moved reference at f1400).
- Before (S78X): screen +10.63 levels (45% of pixels brighter by >12); fine-texel ratio median 1.197; sets moved/ref 9/16.
- S78S2: screen +10.11 levels (44%); fine-texel ratio median 1.163; sets 16/16.
- direct_only stays at about 0 in both. Still-frame qs was 20 fps for both builds (rough, contended by gaming load).

Postmortem: the moved run's fine texels now settle after 16 sets, like the reference, but they are still
about 16-20% too bright. The early settle is a symptom, not the cause. A likely cause, not yet confirmed:
the GIBS/MSME long-term blend floors at k = 1/256 per set, and a settled texel takes a set only every
LIGHTING_SC_RATE_SLOW = 240 frames. Its time constant is about 256 x 240, roughly 61k frames, so whatever
value it carries in while the camera moves (resampled from a coarser size, or gathered from a still-filling
cache) never decays. Kajiya and GIBS apply 1/256 per frame, not per 240-frame set.
