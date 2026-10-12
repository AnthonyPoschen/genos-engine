# S73B: All scout hits claim fine texels and stamp LastUsed

- **Date:** 2026-10-12
- **Result:** reverted (8b62829b); kept on gi-v2-prefill (22bcd47f)
- **Commit / patch:** 22bcd47f

## Idea
All scout hits claim fine texels and stamp LastUsed

## Motivation
not recorded

## Paper basis
Lumen surface cache GPU feedback

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Reverted to the checkpoint tree (6b4fe308) before the S74 cost cuts. Reason as relayed by the Engine agent from the chat with Anthony (the transcript page containing it was not available to this write-up): the pre-fill made the picture darker, slower and popped more. Decision: off-screen pre-fill comes back the Lumen way only (surface-cache feedback), once performance is good; kept on branch gi-v2-prefill (22bcd47f).

## Lessons / when to revisit
S-curve off-screen lag bound idea (Anthony 15:55) deferred until spare performance.
