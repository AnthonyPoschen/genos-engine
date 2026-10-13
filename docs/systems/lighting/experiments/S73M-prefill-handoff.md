# S73M — Pre-fill handoff into MSME

A never-shown texel averaged up to 4 updates and skipped the MSME firefly cap (S73D). On the first shown frame MSME started from that mean. Its warm-up only caps `k` (`min(k, 1/(t+1))`), so a bias inside the variance never moved. Frames 12–23 of the owner's turn were worse than the path that had been updating the whole time.

Discarding the colour on first show would make the texel unfilled again. Instead:

- While never shown, each new set is clipped with the MSME firefly cap (`0.1 + mean + 8 sigma`, sigma `max(0.15 L, 1e-3)`) before the 4-update average.
- The first pixel that shows a fine texel with a finished set keeps that colour, sets the count to 1, and writes the estimator as a newborn (`variance (0.15 L)^2`, `b = (1, 1, -1, 1)`). `b.z < 0` is the handoff marker. Nothing else reads that component.
- While the marker is set and the count is under 16, the blend is the plain mean `k = 1/(t+1)`, not a cap on the MSME `k`. The marker clears on the set that reaches 16. The normal young-texel cap is unchanged.

The original sample then weighs about 1/16 of the mean, and the detector still sees a finished set.

Deviation: MSME and the papers do not force a plain mean for a texel that was accumulated off screen. Lumen's 4-update radiosity accumulation remains only while the texel has never been shown.
