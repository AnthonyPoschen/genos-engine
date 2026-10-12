# S73K: scout feedback claims without the irradiance read

- **Date:** 2026-10-12
- **Result:** lavapipe passes the turn assertion. After 96 still frames, 18335 never-shown fine texels held a finished set. A half-radian turn showed 4547 of those keys.
- **Commit:** d4a666f
- **Deviation:** a scout hit does not seed an empty texel. Seeds stay on the pixel path.

## What changed

`lighting_direct.comp` used to call `lighting_world_irradiance` on a scout hit and drop the returned vec4. Under `lighting_sc_feedback` that read claimed the four corners, stamped LastUsed, offered the texel, skipped the shown stamp, and ran the same seed, parent, and blend the pixel path uses.

Inlining that function into the direct shader faults lavapipe. The fault is the direct dense dispatch (`vkQueuePresentKHR`, SIGSEGV in the JIT). Stopping before that dispatch does not fault. The trace-dense indirect dispatch does not fault. `robustBufferAccess` does not stop it. Wrapping the call in `false &&` lets a full frame finish, and the pixel path still calls `lighting_world_irradiance` in that run. The read is not a clampable out-of-range access.

`lighting_sc_feedback_claim` keeps the part the scheduler needs: the four corners, the LastUsed stamp, `lighting_sc_offer`, and the mover-local case. It does not stamp shown. It does not seed. S73A said a guard hit claims exactly as a pixel does, including the seed. That seed is the deviation.

## What the short lavapipe run showed

Settle 8, turn 4, 320×180, small hall, no fault. Frame 3: `prefill_listed` 5651, `prefill_taken` 0, `prefill_sets` 0. Frames 7 and 8, still, spare open: taken 1552 then 3132, sets still 0, 5651 fine texels never shown and none with a finished set, 4682 on a partial quarter. Turn frames 9–16: taken 0, listed grew to 12278.

That is S73I's first gate on a short run, not only in the source. Two spare frames are one or two quarters of a four-quarter set, so settle 8 cannot pass the assertion.

The default test (settle 96, turn 24, yaw 0.5 rad, 320×180) passed on lavapipe in 246 s. The frame-96 dump: 37008 fine texels, 19515 never shown, 18335 of those with a finished set, 1180 still on a partial quarter. Sets 0..4+ were `[1180, 9776, 3826, 3765, 968]`. After the turn, 4547 of those 18335 keys were shown, 13788 were still hidden, and none had been freed.

Spare is not gone for the whole turn once the hall has settled. Frames 99–114 (the turn) still took 9k–16k pre-fill entries and finished thousands of sets a frame. By frame 119 the new on-screen texels had filled the ceiling again: `prefill_listed` stayed near 16k and `prefill_taken` was 0. A settled turn fills the guard band at the start. A turn that keeps revealing unsettled texels stops tracing pre-fill, which is the spare-budget rule.
