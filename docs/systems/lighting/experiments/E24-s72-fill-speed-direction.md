# E24: Fill-speed direction for S72: sparse fill, reuse, denoise, then regions refine together

- **Date:** 2026-10-12
- **Result:** decision
- **Commit / patch:** S72A-S72S
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Fill-speed direction for S72: sparse fill, reuse, denoise, then regions refine together

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t299u-t301u): many small pops while filling; popping is fine if the screen fills near-immediately, smoothing would be lag. (t302u-t305u): measure how filled things need to be; papers on not filling every texel; 'we don't need tests we need action'. (t307u) fills faster but pops are bigger, not spread wide enough; still scene ~40 fps. (t308u/t309u) spread radius should scale with camera distance (not built beyond texel size). (t313u) quality before performance. (t315u-t317u) whole-image consistency: low everywhere or high everywhere beats a patchwork. (t321u-t325u) page squares pop; first-fill tearing when rounding corners. (t327u) two layers (traced data never shown; shown smooth field). (t328u/t329u) suppress history smoothly by how different new data is; follow the papers' maths. (t331u/t332u) GTX 1060 rule. (t333u/t334u) indirect light is smooth; fewer, smarter probes. (t335u/t336u) follow papers' algorithms and structure; our design choices are where bugs come from. (t337u) ec3d5e74 better but pages blocky with moving lights -> paper-exact SVGF (S72V). (t339u) d23a9218 'very very splotchy, big massive patches; don't abandon it'; (t342u/t343u) ripples, 'have we missed something?' (-> V5/V6 wiring bugs). Checkpoint 6b4fe308 tagged after Anthony's approval.

## Lessons / when to revisit
not recorded
