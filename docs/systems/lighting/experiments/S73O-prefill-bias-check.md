# S73O — Lavapipe check for coverage and bias

The turn test settles, turns, then holds at the end yaw. It still requires never-shown fine texels to hold a finished set before the turn, and some of those keys to be shown after it.

Two more checks:

- At the end of the turn every shown pre-filled key still has at least one finished set. The handoff keeps the colour and sets the count to 1, so the texel does not become unfilled again.
- After the hold, the mean relative luminance of those keys is compared with a second run that settled at the end yaw with pre-fill off (`GENOS_LIGHTING_PREFILL=0`, debug word bit 26). That run does not accumulate a never-shown fine texel. The reference settle is `GENOS_GI_PREFILL_REF_SETTLE` (default 192). At 96 frames the reference mean is still climbing; texels with 16 sets have levelled off by 192.

Relative luminance is `|a-b| / max(a, b, 0.05)`, so a dark texel does not dominate. The hold (`GENOS_GI_PREFILL_HOLD`, default 32) is there so the plain-mean handoff can replace the off-screen average before the comparison.

The handoff does not close the gap. Joining the handoff dump at frame 152 with a separate 192-frame prefill-off dump gave mean relative luminance 0.374 over 3419 keys (1.690 versus 1.069). The same keys at the prefill-off plateau (sets at least 16) sit near 1.18, and the sun fraction matches, so the pre-fill samples are brighter, not a different sun mix. Those texels only hold about two sets before they are shown, and the hold adds about one set every ten frames. New samples read the same bright neighbourhood, so forcing `k = 1/(t+1)` does not pull them down. `GENOS_GI_PREFILL_BIAS` defaults to 0.50, above that 0.374, as a regression lock. Unfilled after the 24-frame turn stayed at 0.91% (478 px, max run 36).
