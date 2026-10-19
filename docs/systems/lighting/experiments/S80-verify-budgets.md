# S80: Verify settled bounce with two budgets + yaw-edge warm (PR #39)

- **Date:** 2026-10-13 (work); closed 2026-10-19
- **Result:** rejected / not remerged (4070 cost never measured on this tip vs current master)
- **Commit / patch:** PR #39 tip `69ed030c` on `cursor/gi-verify-budgets-7623` (closed; branch deleted).

## Idea

Two independent verification budgets on settled idle: `screen_verify` (rays from visible probes comparing cache irradiance to stored mean) and `world_verify` (settled world-cache cells, with a guard shell just outside the frustum). A clearly lit hit against a near-black stored mean replaces the mean and wakes refill. Also warm yaw edges via guard-band scouts every prefill round.

## Motivation

Settled frames ran pixel direct + compose only; age-gated checks confirm dark-vs-dark and barely move a 64-sample mean, so a poisoned bounce room stayed wrong for a long time. PR #31's stride votes **direct** luminance and does not visit world-cache cells, so it cannot own this fix. Newly revealed columns popped because still scouts were sparse.

## What was implemented (PR branch)

`screen_verify` / `world_verify` on `LightingSettings` (high: 2048 / 128), CLI `--screen-verify` / `--world-verify` with env mirrors, panel sliders, diag slots, mean-replace on black-vs-lit miss, guard-band scout fraction, lavapipe tests (`poisoned_visible_texels_wake_within_sixteen_frames`, `a_small_yaw_keeps_the_revealed_edge_near_the_settled_view`).

## Numbers

| What | Result |
|---|---|
| Lavapipe poison wake | detected next frame; half bounce by frame 15; luma 4.13 at 48 (from 5.58 settled → 0.32 poisoned) |
| Lavapipe yaw 0.10 rad | edge MAE 0.20 / 0.02 / 0.04 display codes (gate 1.5) |
| 4070 settled idle cost | **not measured** on the PR (plan: 2176 extra trace rays on high) |
| Tip vs current master | merge hit **12 conflict files**; no exclusive A/B this window |

## Outcome and why

**Closed.** Correctness tests on lavapipe look promising, but there is no 4070 frame/lighting proof against current master, and the branch does not merge cleanly after GPU_MS / sun / settings evolution. Standing rule: no long conflict untangle without a measured win. Revisit as a fresh PR from current master with settled-idle `--gpu-times` A/B (verify on vs `--screen-verify 0 --world-verify 0`) before merge.

## Lessons / when to revisit

- Land verify budgets only after measuring settled `GPU_MS` `lighting` / `trace` on the 4070.
- Keep potato/day-cycle at 0; high defaults need the cost table in the README.
