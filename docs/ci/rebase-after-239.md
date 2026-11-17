# Rebase plan after #239 lands

Do not rebase yet. #239 merges current master (semgrep venv, #253 quit
path, and the rest of today's master) into lighting-v3. Once that merge
is on lighting-v3, rebase live stacks so they pick up those fixes
instead of copying CI files onto each head.

This list is from 11 Nov 2026, 10:48+ AEDT. Re-check `gh pr list` before
starting; close anything already merged.

## 1. lighting-v3 itself

Confirm #239 is merged and `lighting-v3` contains `dae34ae2` (semgrep
venv) and `a2def62d` / #253 (quit path). No rebase of lighting-v3
beyond that merge.

## 2. Integration stack (do this first, one writer at a time)

Rebase onto the new lighting-v3 tip, then re-review if the rebase is
not a fast-forward of already-reviewed commits.

1. #252 (`cursor/lighting-integration`) — the integration PR, base
   lighting-v3. If it has already merged, skip to its children against
   the new lighting-v3 tip.
2. Direct children of #252, in this order:
   - #258 (lamp picture time) then #259 (A-SVGF lambda) then #261
     (lambda settle). These are a chain: 258 ← 259 ← 261.
   - #255, #257, #262, and #265 (on #262's head) can follow in
     parallel once #252 is retargeted, one writer per branch.
3. #263 (direct running mean), base lighting-v3, after or beside #252.
4. #225 (sync validation), base lighting-v3, independent of the
   integration picture work.

## 3. Older lighting-v3 docs and hold PRs

These target lighting-v3 directly. Rebase only if they are still going
to merge; otherwise leave them or close them as superseded by #252.

- Docs / design: #100, #122, #124, #147, #148, #150, #164
- Flagged / unmeasured: #98
- Live C5 still open beside the integration: #120

## 4. Stage-stack drafts superseded by #252

Do not rebase these unless someone still wants them as standalone
history. Prefer closing them once #252 (or its successor) is the
integration path.

#173, #175, #182, #184, #185, #186, #187, #188, #189, #190, #192,
#193, #194, #195, #196, #197, #204, #206, #207, #208, #209, #211,
#240.

If any of these must stay open (for example a docs note that is still
the review surface), rebase that one onto the new lighting-v3 or onto
the surviving parent, one at a time.

## 5. Non-lighting stacks (onto master)

Master already has the CI fixes. Rebase each chain from the bottom up
after the lighting-v3 work is queued or idle:

- Scene / jobs: #222/#223 are merged; remaining chain #231 (if still
  open) → #249 → #250 → #251.
- Animation: #221 (if still open) → #235 → #238.
- Vehicles: #200 (if still open) → #202; #201 on the thruster parent.
- Assets: #178 on `cursor/cooked-material-record-be4b` (needs #177 on
  the parent, already merged to master — retarget or rebase onto
  master).
- Other stale-lint PRs that target master can rebase onto master
  whenever convenient; they are not blocked on #239.

## Rules

- Do not edit several lighting branches at once; one writer per branch.
- Do not force-push a reviewed tip without a note on the PR.
- After each rebase, confirm `.github/workflows/lint.yml` has the
  semgrep venv (`python3 -m venv` and `semgrep==1.184.0`) and that the
  quit path from #253 is in the tree if the branch still runs quit-gap.
- Leave #219 unmerged (existing hold). This plan does not touch it.
