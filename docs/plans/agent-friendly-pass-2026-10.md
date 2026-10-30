# Agent-friendly pass, October 2026

Date: 2026-10-28. First weekly pass over genos-engine. Analysis and plan only: nothing here changes engine behaviour. One prototype is on branch `prototype/gpu-evidence` (2df6609).

## In plain words (for Anthony)

Agents keep making the same handful of mistakes here, and most of them get past us because the checks agents run (lavapipe on the box) can't see them, or because the code lets the wrong thing compile. I want to fix the causes in the code itself rather than add more rules to AGENTS.md:

1. **Make every check say which GPU it ran on**, and make timing and picture gates refuse to pass on lavapipe. Today a lavapipe pass looks the same as a 4070 pass, which is how the HDR rings, the swapchain double-create, the destroy_device stall and the wall leak all looked "green".
2. **Make the renderer reset lighting itself when the scene changes**, so no test or game can forget to.
3. **One shared GPU test fixture** instead of 20 hand-copied `open()` functions, carrying (1), (2) and Vulkan validation.
4. **Turn validation errors into test failures** on the box, after fixing the six error kinds already on master.
5. **Make git setup impossible to get wrong**: a setup script for worktrees and a CI check on commit author, instead of a rule agents forget.
6. **Finish the lighting renames** the glossary already agreed, and add a lint that fails when a retired name comes back.
7. **Put cost limits in code** (e.g. guard band area) with a unit test, so a tuning change can't quietly grow the picture.

**Needs your OK before I start** (big or cross-cutting): items 2 (renderer API behaviour), 3 (moves every GPU test), 4 (new CI gate), 6 (public renames with deprecated aliases), and making the 4070 gate mandatory for `crates/render` PRs. Items 1, 5 and 7 are small and go up as normal PRs for review.

## Evidence: the mistakes agents repeat

Counts come from `git log` on master (825 commits, 100 merges, 86 PRs) on 2026-10-28, the 4070 log in `/home/box/shared/ge-4070-verify-28oct.md`, the postmortems, and today's token review. Anthony gives almost no GitHub review comments (0 inline, 29 issue comments, all bot-written closes quoting his rules); his feedback lives in chat and is recorded in the fleet rules and Chief's GI history.

| # | Class | Count | Real examples |
|---|---|---|---|
| A | A software-GPU (lavapipe) pass taken as proof of real-GPU behaviour | 4 shipped bugs today; 10 tests documented as "lavapipe-only" fails that also fail on the 4070 (7 gpu_frame, particle_frame, 2 picture) | HDR10 readback rings since #75 (620d89f, fixed d431091; readback error 57.4 → 0.27); swapchain double-create in #73 ("Surface already has a fifo"); destroy_device 13.5–20 s on the 4070 vs 20–36 ms on lavapipe; surface-cache wall leak since #54 (ba9adca). AGENTS.md still lists 3 gpu_frame tests + picture.rs as lavapipe-only. |
| B | A gate that grades something other than the GPU output | 2 in the token review, plus #261-style "token value, not rendered page" in the fleet postmortems | Checks comparing a CPU model; readback that copied PQ words as BGRA8 so every screenshot gate graded a different picture than was presented. |
| C | Hidden caller contract: switching World without `reset_lighting_cache()` | 4 call sites in the whole tree, 2 of them tests; ≥2 failing gates caused | sky test (fixed #84, fe53f65); `a_bright_lamp_does_not_cross_an_opaque_wall` passes only with the reset; #86 moves the reset into the renderer. |
| D | Git/process slips | 1 direct push to master (d431091, `push.default=upstream`); worktrees using Anthony's main repo as their store (fetch + local config written); 15 of 198 commits since 19 Oct under the wrong identity (9 Cursor Agent, 3 `agent@local`, 3 noreply) | ge-4070-verify-28oct.md 07:00 and 07:40 entries. |
| E | Stale tests after a default changes | 3 today | lamp-pick default test after ReSTIR (#62, fixed in #83); crop-margin test after #70 A2; budget-controller test scene no longer heavy enough after #70 (7.48 ms < 8 ms cap). |
| F | Silent cost growth from a tuning knob | 1 big case | #70 guard_edge 0.15 → picture area 1.69×, direct 1.70× master on s5; plus 22× inlined wall test → 73.6 s first pipeline compile. Fixed by 2c41cfad / 577a028e only after a 4070 profile. |
| G | Vulkan misuse on master | 6 validation error kinds | renderPass-00904, descriptorType-00333, vkCmdDraw-09600/02684, barrier-02816/04091 (Khronos VVL on lavapipe, #73 takeover 09:50). Nothing runs VVL in CI. |
| H | Term drift against the glossary | RENAMES.md "proposal, not yet applied" since 14 Oct: 16 renames, 9 synonym groups, 7 collisions; `world_cache` still at 60 sites, `GENOS_LIGHTING_SC_*` 28, `shadow_filter` 12, `lamp_pick_rays` 11 | Anthony's rule: one meaning per term, rename code when it drifts. |
| I | No-op wiring | 2 in the token review | A setting or flag plumbed through config/MCP that nothing reads. 285 distinct `GENOS_*` env names in code make this easy to miss. |
| J | Duplicate fixtures and numbering | 20 hand-copied `open()` test fixtures; two ADRs numbered 0019 | `docs/adr/0019-agent-operable.md`, `docs/adr/0019-reflections.md`. |

Structural context that makes these easy: `crates/render/src/gpu.rs` is 8,696 lines with 96 `pub fn`, plus 5,359 lines `include!`d into it (`lighting_gpu.rs`, `lighting_dispatch.rs`). A test can reach almost any internal switch, and nothing records which device it ran on.

## Plan, by trust-pyramid level

### 1. Code structure (most weight)

**S1. Device evidence in the type system (classes A, B).** `Renderer::gpu_kind() -> GpuKind { Software, Hardware }`, read from `VkPhysicalDeviceProperties.deviceType`. Then a `GateResult` that carries `GpuKind`; timing gates and the "real-GPU picture" gates return `Unproven` instead of `Pass` on `Software`, and their summary line prints the kind. A lavapipe run can still prove correctness tests; it can no longer print PASS for a 4070 gate.
- *Prototyped:* branch `prototype/gpu-evidence` (2df6609). Test `crates/render/tests/gpu_kind.rs` reports `Software` on lavapipe and fails when the mapping is mutated (4 → 99). Clippy clean for `genos-render`. Open question "can we tell lavapipe from inside the renderer without new FFI?" is answered: yes, the property block is already fetched.

**S2. Scene identity owned by the renderer (class C).** `World` gets an identity from `from_scene` (clones keep it); `draw*` resets the lighting cache when the identity changes. `reset_lighting_cache()` stays for explicit use but stops being a contract callers must remember. This is #86's approach; it still fails `a_close_lamp_does_not_cross_the_wall` on the 4070, so it lands only when that passes. *Needs OK.*

**S3. One GPU test fixture (A, C, G, J).** A `genos-render` test-support module (dev-only) with `GpuFixture::open(w, h)` that holds the GPU mutex, opens the proof window, enables validation when `GENOS_VK_VALIDATION=1` (mirror of a fixture argument), and exposes `gpu_kind()`. Migrate the 20 copies, then delete them. *Needs OK* (touches every GPU test file).

**S4. Cost limits as constants with tests (F).** Express the guard band as a maximum area factor (`MAX_GUARD_AREA = 1.15`) and derive `guard_edge` from it per preset, with a unit test that fails when any preset exceeds it. Same pattern for the shader inline budget: a build.rs check on SPIR-V instruction count per pipeline against a recorded ceiling.

**S5. Settings read-or-fail (I).** Every key in the settings catalogue must have a reader; a test walks the catalogue, flips each key, and asserts the frame's settings hash or GPU push block changed. A no-op key then fails by name.

### 2. Static checks and scripted verification

**C1. Validation layer run on the box (G).** First fix the 6 VUID kinds on master, one commit each. Then a CI job (lavapipe + VVL) runs camera 60 frames and the gpu_frame suite with validation errors as failures. *Needs OK* (new CI gate).

**C2. Commit identity and push safety (D).** `scripts/setup-worktree.sh` makes a separate clone under `~/.cache` with `user.email=zanven42@gmail.com` and `push.default=nothing`, and refuses if the store is `~/git/github.com/anthonyposchen/genos-engine`. A lint step fails a PR whose non-merge commits are not by `zanven42@gmail.com`. Branch protection on master (no direct push) is Anthony's toggle. *Branch protection needs OK.*

**C3. Glossary lint (H).** `scripts/check-terms.sh` reads a "Retired" list from GLOSSARY.md and fails when a diff adds a retired name outside alias shims. It is diff-based, so the existing 60+ sites don't block anyone; the renames in RENAMES.md then burn them down, one commit per rename with a deprecated alias. *The public renames need OK.*

**C4. Default-change sweep (E).** Settings defaults live in one table; a test snapshots that table, so changing a default fails `settings_defaults_snapshot` and the same PR must update the snapshot and the tests that name it. This makes "stale test after default change" fail in the PR that caused it.

**C5. ADR numbering check (J).** A lint step fails on two ADRs with the same number. Renumber `0019-reflections.md` to the next free number.

**C6. 4070 gate for render PRs (A).** A PR touching `crates/render/` or shaders is merged only with a `gpu_kind Hardware` gate log attached (from the morning routine). Enforced by S1's output, not by memory. *Needs OK.*

### 3. Prose rules (minimal)

- AGENTS.md: replace the hand-kept "expected lavapipe fails" list with "run the suite; anything printing `Unproven` needs the 4070". The list is already wrong (it misses 7 tests).
- Add a rule table to AGENTS.md pairing each rule with its enforcer (per the `correct` skill), and delete each prose rule once its check lands.

### 4. Human review (least)

Anthony judges screenshots and feel; numbers go to the bots. No change.

## Order

1. Now, small PRs for review: S1 (from the prototype), C5, C2 setup script, AGENTS.md list fix.
2. After OK: C1 fixes then gate, S3, S2 (#86 once the close-lamp test passes), C3 + renames, C4, S4, S5, C6.
3. Re-run this pass next week and compare the counts in the table above.

## Why not lower levels for the top classes

- A and B: a rule "check on the 4070" already exists and was missed four times in a day; only a result that knows its device can't be misread.
- C: the reset was documented as a contract and still missed; ownership in the renderer removes the call.
- D: the identity rule is in fleet-rules.md and was broken in 15 commits; a setup script plus CI makes it the default path.
