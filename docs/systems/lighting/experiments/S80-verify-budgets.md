# S80: Two verification budgets

- **Date:** 2026-10-13
- **Result:** pushed
- **Commit / patch:** this change

## Idea

A settled frame used to run pixel direct and compose only. The age-gated surface-cache and world-cache checks (A-SVGF deferral 8→128, WRC refresh at 180 frames, one new set blended into a mean of up to 64) do notice a wrong dark region, but only after a long time. The owner watched a sunlit room's bounce fade in eventually.

Two settings, not one split of a shared budget:

- `screen_verify` — rays per frame. Each ray starts at a visible probe's pixel, traces one hemisphere direction, and compares the hit's cache irradiance with the texel's stored mean. The visible set is the probe grid, so the budget is dense relative to it (high: 2048, about every fourth probe per frame at 1080p).
- `world_verify` — rays per frame over settled world-cache cells. Three quarters aim just outside the frustum and claim that cell. One quarter walks the settled cells.

A stored luminance under 0.002 (black; half a display code) against a fresh hit over 0.02 replaces the mean with that hit's irradiance (count 4) and wakes the screen probe. A dim shadow that one ray happens to aim out of does not match, so a converged corner is not reset every frame. The replacement does not add one sample onto the old mean. The host then drops the settled idle so the cache pass refills it.

The budgets are diag[38] and diag[39]. Words 142 and 143 stay the sun mark: the direct pass writes them before verification runs.

`0` disables that budget and leaves the other one running. Preset high is 2048 and 128. Potato and day-cycle are 0. The values are `LightingSettings` keys, `GENOS_LIGHTING_SCREEN_VERIFY` / `GENOS_LIGHTING_WORLD_VERIFY`, the stress knobs, and MCP `set_key`.

## Why not PR #31

gi-v2 had no always-on random verification of settled probe radiance or world-cache irradiance. The held check ray tests hit geometry, and a confirmed hit ignores cache drift (S78U). PR #31's stride verifies direct luminance of 16 probes a frame and does not visit world-cache cells, so a bounce-dark floor with correct direct is a hit. A miss there tightens a tolerance; it does not reset the mean. This change owns the dark-room fix. PR #31 can keep ring wakes.

## Paper deviations

- DDGI updates every probe every frame. These are two sparse budgets.
- Lumen keeps a fixed update budget and accumulates about four frames. A miss here replaces the running mean.
- A-SVGF's check confirms and defers. A dark sample that matches a dark mean confirms. This check treats stored darkness against a lit hit as a miss.
- The world budget's guard rays are a shell just outside the frustum, not S73A's reverted fine-texel claim and not only the forward scout cone.
- Verification retraces. It does not reuse or hold a previous frame.

## Cost

On the settled idle the extra work is `screen_verify + world_verify` trace rays (high: 2176) plus the shade of each hit through the existing cache lookup. Lavapipe does not time the GPU. On a 4070, with the bench lock:

```sh
cargo run --release -p genos-stress -- --view hall --freeze-sun --still-boxes \
  --time-of-day 10.9 --frames 120 --gpu-times 30 --lighting-debug --no-panel
```

The trace bucket on a settled frame is this cost. Set either key to `0` to measure that budget alone.

## Test

`poisoned_visible_texels_wake_within_sixteen_frames` zeros every settled texel a pixel in a screen rectangle gathers, then requires `screen_fails` within 16 frames and the rectangle's bounce back to half its settled luminance within 24 frames. The gate is the detection latency, not merely that a wake is possible. On lavapipe, after 32 quiet frames, the rectangle (bounce luma 5.58) went to 0.32 when poisoned, the screen budget woke it on the next frame, half the bounce was back by frame 15, and at frame 48 the luma was 4.13.
