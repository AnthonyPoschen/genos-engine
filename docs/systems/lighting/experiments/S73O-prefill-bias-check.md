# S73O — Lavapipe check for coverage and bias

The turn test settles, turns, then holds at the end yaw. It still requires never-shown fine texels to hold a finished set before the turn, and some of those keys to be shown after it.

Two more checks:

- At the end of the turn every shown pre-filled key still has at least one finished set. The handoff keeps the colour and sets the count to 1, so the texel does not become unfilled again.
- After the hold, the mean relative luminance of those keys is compared with a second run that settled at the end yaw with pre-fill off (`GENOS_LIGHTING_PREFILL=0`, debug word bit 26). That run does not accumulate a never-shown fine texel. The cap is `GENOS_GI_PREFILL_BIAS`.

Relative luminance is `|a-b| / max(a, b, 0.05)`, so a dark texel does not dominate. The hold (`GENOS_GI_PREFILL_HOLD`, default 32) is there so the plain-mean handoff can replace the off-screen average before the comparison.
