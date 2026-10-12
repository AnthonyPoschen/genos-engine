# E01: Splotch diagnosis: which cause makes v2 blotchy (cache resolution, per-texel albedo, probe rays, still-scene averaging)

- **Date:** 2026-10-10
- **Result:** finding
- **Commit / patch:** none (throwaway worktree; report /workspace/shots/gi2diag/SUMMARY.md on the box)
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Splotch diagnosis: which cause makes v2 blotchy (cache resolution, per-texel albedo, probe rays, still-scene averaging)

## Motivation
Anthony (t6u, 13:58): v2 still low detail and splotchy up close.

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
12 variants measured; per-view tables in the box SUMMARY.md, not copied here.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
Splotches come mostly from screen-probe gather noise, not light-cache resolution; the 32-frame still averaging hid it, and in the stress scene lamps and boxes always move so averaging never kicked in.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony asked (t9u/t10u) to read exactly how Lumen solves it ('it is a very very good lighting engine'): Lumen's published methods became the default answer. He also asked (t12u/t13u) for executable checks instead of image interpretation, hand-validated first, then ~1% spot checks: this became gi-check.

## Lessons / when to revisit
Origin of gi-check and of the 'Lumen first' rule.
