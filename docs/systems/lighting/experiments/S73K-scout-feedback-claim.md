# S73K: scout feedback claims without the irradiance read

- **Date:** 2026-10-12
- **Result:** lavapipe runs the claim. A short settle lists never-shown texels and takes spare quarters. Finished sets need the still settle, not the turn.
- **Commit:** d4a666f
- **Deviation:** a scout hit does not seed an empty texel. Seeds stay on the pixel path.

## What changed

`lighting_direct.comp` used to call `lighting_world_irradiance` on a scout hit and drop the returned vec4. Under `lighting_sc_feedback` that read claimed the four corners, stamped LastUsed, offered the texel, skipped the shown stamp, and ran the same seed, parent, and blend the pixel path uses.

Inlining that function into the direct shader faults lavapipe. The fault is the direct dense dispatch (`vkQueuePresentKHR`, SIGSEGV in the JIT). Stopping before that dispatch does not fault. The trace-dense indirect dispatch does not fault. `robustBufferAccess` does not stop it. Wrapping the call in `false &&` lets a full frame finish, and the pixel path still calls `lighting_world_irradiance` in that run. The read is not a clampable out-of-range access.

`lighting_sc_feedback_claim` keeps the part the scheduler needs: the four corners, the LastUsed stamp, `lighting_sc_offer`, and the mover-local case. It does not stamp shown. It does not seed. S73A said a guard hit claims exactly as a pixel does, including the seed. That seed is the deviation.

## What the short lavapipe run showed

Settle 8, turn 4, 320×180, small hall, no fault. Frame 3: `prefill_listed` 5651, `prefill_taken` 0, `prefill_sets` 0. Frames 7 and 8, still, spare open: taken 1552 then 3132, sets still 0, 5651 fine texels never shown and none with a finished set, 4682 on a partial quarter. Turn frames 9–16: taken 0, listed grew to 12278.

That is S73I's first gate on a run, not only in the source. The turn spends the base ceiling on texels that just became shown, so pre-fill claims during the turn and does not trace them. A finished set has to accumulate while the camera is still and spare entries exist. The ignored test settles 96 frames before the half-radian turn for that reason. Two spare frames are one or two quarters of a four-quarter set, so settle 8 cannot pass the assertion.
