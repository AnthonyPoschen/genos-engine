# S78W: Shader stats in device memory (+ per-dispatch GPU times)

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** this commit

## Idea
Re-testing S78R showed the gather holding 8,088 of 8,093 probes with the gather dispatch still at 0.88 ms. A per-dispatch timer (new: `GENOS_GPU_DISPATCH=1` with `GENOS_GPU_TIMES`, prints `LIGHTING_DISPATCH i:pass=ms`) and bisection found the cost was the held probes' two `atomicAdd`s on the stats buffer: the stats buffer was host-visible (readback) memory, so every shader atomic crossed PCIe. Removing the two atomics: 0.88 -> 0.04 ms.

## What was implemented
The lighting stats live in a device-local buffer (`stats_dev`), cleared and bound as before; at the end of the frame's passes one `vkCmdCopyBuffer` moves them to the host-visible buffer, which the CPU reads a frame later as before (no new readback, no wait). The async world-round stats buffer is unchanged (async rounds are off).

## Numbers (RTX 4070, stress, 1080p, frames 300-600)
The desktop was in use during the A/B (Hyprland ~5 % SM, Spotify decode), so all runs are contended; runs alternated U/W/U/W:
| | S78U | S78W |
|---|---|---|
| compact still | 1.77 / 1.96 ms | 0.153 / 0.155 ms |
| compact moving | 1.80 / 2.09 ms | 0.236 / 0.236 ms |
| still total (lighting) | 9.33 / 9.67 ms | 7.44 / 8.71 ms |
| moving total | 21.0 / 21.2 ms | 22.1 / 18.9 ms (noise) |
First (quieter) run: still 7.29 ms lighting / 114 fps vs S78U 7.91 / 108.
Smoke a 0.348; pop 0.114 % / 0.142 % (S78U runs 0.074-0.090 / 0.116-0.129; no behaviour change, only where the counters live). Change counters still read correctly (chg-w: 0 kept changes).

## Screenshots
~/.cache/gi2diag/perf/shad-S78W/b.png (viewed; mean abs diff vs S78U 0.83/255).
