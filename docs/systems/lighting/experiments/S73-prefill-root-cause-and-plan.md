# S73 pre-fill: why it darkened, and the plan to bring it back

- **Date:** 2026-10-12
- **Branch:** `cursor/gi-prefill-spare-budget-3544` (off `gi-v2` e61b89d)
- **Status:** S73A-S73G implemented (S73D 6f54914, S73E 1071524, S73F 8ac033e, S73G 12387cc). Not run: real GPU tests are on the owner's PC. The root-cause evidence is from reading the code.

## Done on the branch

| Commit | Change |
|---|---|
| 0dc6c4b | S73A port (097d67de, shader parts only, line for line): a quarter of the scout rays go through a guard band just outside the frustum, biased to the turn side, and their hits claim fine texels through `lighting_world_irradiance`. |
| 71547c9 | S73B port (22bcd47f, line for line): every scout hit claims fine texels; the non-guard hits are feedback (they stamp LastUsed, not shown). |
| af449f6 | S73C: every scout hit is feedback, the guard band included. A feedback seed claims the 25 cm cell and stamps it used, not shown, so it starts no reveal burst. |

0dc6c4b and 71547c9 reproduce the reverted, darker build on purpose, so it can be A/B tested against the fixes.

## Darkening root cause

Two causes add up. Both come from treating an off-screen pre-fill claim as if it were a pixel.

### 1. Pre-filled texels' first sets become their long-term mean (estimator)

A texel's first set is taken whole. In `lighting_cache.comp`, when `old.w < 0.5` the MSME branch is skipped and the code falls through to `mean = old.w < 0.5 ? e : ...`, with the count set to 1.

From the second set on, the MSME long mean barely moves during warm-up:
- At the first MSME step (`b.x <= 0`) the state is seeded with the short mean set to the stored mean and the variance set to `(0.15 L)^2` (S72T).
- The warm-up then uses `k = min(k_msme, 1/(t+1))`. The comment calls this a plain mean, but the code only caps `k` from above.
- Worked case: the second set is 30 % brighter than the first, and `rho = 0.08 * 0.3 / 0.15 = 0.16`. With `inc` at 0.93, `k = smoothstep(0.16 * 0.73) ≈ 0.038`, so the long mean moves 3.8 % toward the new set where a plain mean would move 50 %.
- Past 16 sets, `k` falls to the 1/256 floor whenever `rho * (inc - 0.2)` stays small. That holds offsets up to about 0.4 of the per-set standard deviation.

So the long mean stays close to the first set. PR #3 ([cursor/gi-headless-stuck-patch-2bf4](https://github.com/AnthonyPoschen/genos-engine/pull/3)) adds to this: 3 in 4 texels never pass both the IC gate and the layer-2 gate, so they take no set after their 16th. It measured a never-moved run 14.4 % darker than the reference, with fine texels at 0.618 of their WRC cell's luminance.

Pre-fill feeds this mechanism the darkest first sets it can get:
- A scout-claimed texel takes its first set as soon as it is claimed. Its region was only just found by the scouts, so the 25 cm WRC cells its rays hit are newly claimed and mostly unlit. Those hits return a parent, a lit neighbour, or 0 (`lighting_cache_irradiance`, flag 32768). That is the missing bounce at fill time.
- A feedback texel is then relit only when a scout hits it again (`used = last - updated`, `lighting_compact.comp`). It reaches the screen holding one or two stale, pre-bounce sets.
- Once on screen, those sets show directly (S72V8: own sets before the seed), count at full weight in the a-trous filter (S72V5), and anchor the MSME long mean.

The seeds are not the problem. With `lighting_sc_seed_split()` on (flag 134217728 off by default), they stay apart from the texel's own sets (S72N).

### 2. Pre-fill took on-screen budget (scheduler)

- **Priority.** A never-relit texel gets `used = min(used, 60)` and `w = clamp(cap / max(t, 1), 1, 8) = 8`, which puts it in bucket `log2(1 + 480) * 4 ≈ 35` (`lighting_compact.comp`). That is the same bucket as a brand-new on-screen texel, and above settled on-screen texels. With S73B every scout hit (4096 rays a round, 4 bilinear corners each) could claim up to about 16k texels a round at that priority. Each round's demand then exceeded the base budget, which also triggers the S66A boost.
- **Reveal bursts.** Each unlit fine texel a scout reached seeded through `lighting_sc_parent_bilerp`, which claims the 25 cm cell and stamps it shown. A WRC cell shown within 2 frames and under 16 samples is a reveal burst: it goes to the young list, takes whole sets, and is served ahead of everything. Up to `LIGHTING_WRC_BURST_MAX` (2048) cells a round qualify, first come in slot order, so off-screen bursts displaced on-screen ones. Fixed in S73C.
- **Shown stamp.** S73A's guard texels were stamped shown, so they entered layer 2 and the on-screen schedule. Fixed in S73C.
- **Camera bug.** When S73 was tested, the S74A camera bug made the scouts fire every round, even with the camera still. Every cost above therefore applied on every frame.

Fewer sets for on-screen texels means less catch-up from their dark early sets (cause 1), so the picture stayed darker in every capture.

## Design (Lumen surface cache, GIBS, budget as a ceiling)

- **Residency.** Scout-hit feedback (Lumen's Surface Cache GPU Feedback) claims texels and stamps LastUsed only. Done in S73C.
- **Off-screen accumulation.** A texel never shown (`WORLD_AT.z == 0`) accumulates like Lumen's radiosity, over at most 4 updates (`r.LumenScene.Radiosity.TemporalMaxFramesAccumulated`), and the MSME stays off. Early dark sets are forgotten as the bounce builds up. When the texel is first shown, the MSME starts from that current value with a count of at most 4.
- **Spare budget only.** Never-shown texels go to their own list. The batch places that list after the young and mature entries, cut at the base ceiling, not at the whole boosted batch. Pre-fill never enters the priority histogram or the boost decision. If nothing is due, nothing is traced.
- **Trickle.** A pre-fill texel takes one 16-ray entry per relight, and its quarters add up to a set (as the S65A trickle does). It is due only when a ray read it since its last relight (LastUsed > LastUpdated).

### Deviations to flag in the commit messages
- The guard band of primary rays is ours. Lumen has no guard band.
- Lumen always spends its fixed budget. Here pre-fill gets only what is spare under the base ceiling.
- On-screen texels keep MSME to 64 sets. Only never-shown texels use Lumen's 4-update accumulation.
- The pre-fill list's rotating start is a fairness rule of ours.

### Interaction with PR #3
- Pre-fill texels skip the IC and layer-2 gates while off screen.
- They reach the screen with at most 4 current sets and no MSME state, so PR #3's part-2 warm-up (k² weights) treats those 4 sets as the first, lightly weighted, sets.
- Textually, PR #3 also edits the MSME warm-up in `lighting_cache.comp` and the gates in `lighting_compact.comp`. Expect small merge conflicts there.

## Implementation plan (one small commit each; do not force-push)

Each step names the file, the place, and the exact change. Build once at the end: run `cargo build --workspace`, which compiles the shaders through `crates/render/build.rs` and needs `glslc`.

### S73D: never-shown texels accumulate over at most 4 updates
File `crates/render/shaders/lighting_cache.glsl`, near `LIGHTING_SC_CHANGE_ALPHA`:
```glsl
// S73D: a fine texel no pixel has shown yet (pre-fill) accumulates as Lumen's
// radiosity does, over at most this many updates
// (r.LumenScene.Radiosity.TemporalMaxFramesAccumulated).
const float LIGHTING_SC_PREFILL_SETS = 4.0;
bool lighting_sc_prefill(uint slot) {
    return lighting_world_mode() == 3u && cache.v[3u * slot + 1u].w <= 0.2
        && floatBitsToUint(cache.v[LIGHTING_WORLD_AT + slot].z) == 0u;
}
```
File `crates/render/shaders/lighting_cache.comp`, in the surface cache texel branch: right after `float cap = lighting_sc_cap(slot);` and its `atcap` counter, and before `if ((lighting_sc_flags() & 1048576u) != 0u && old.w > 0.5) {`, insert:
```glsl
        if (lighting_sc_prefill(slot)) {
            float np = min(t + 1.0, LIGHTING_SC_PREFILL_SETS);
            vec3 mp = old.w < 0.5 ? e : old.rgb + (e - old.rgb) / np;
            lighting_tex_put(slot, vec4(mp, np));
            return;
        }
```
Leave `LIGHTING_SC_EST_AT` untouched (it stays zero from the claim), so the MSME starts at the first relight after the texel is shown (`b.x <= 0` init). Commit message: root cause 1, plus the deviation "MSME only once shown".

### S73E: the pre-fill list's storage and per-round count
1. `lighting_cache.glsl`, next to `LIGHTING_TDL_COUNT_AT`: add `const uint LIGHTING_SC_PREFILL_COUNT_AT = LIGHTING_SC_HIST_AT + 74u;`, `const uint LIGHTING_SC_PREFILL_MAX = 16384u;` and `const uint LIGHTING_SC_PREFILL_AT = LIGHTING_ATROUS_LIST_AT + 5u * 65537u + 6u * 65536u + 16u;`. The last one must come after `LIGHTING_ATROUS_LIST_AT`, near line 2725. Done differently: GLSL requires these constants before `lighting_sc_prefill_take` and `lighting_cache_batch_slot`, which sit above the a-trous section, so `LIGHTING_ATROUS_MARK_AT`, `LIGHTING_ATROUS_LIST_AT` and the three pre-fill constants are declared together just after `LIGHTING_CHECK_P_AT`. The addresses are unchanged.
2. `crates/render/src/lighting_gpu.rs`, the `cache_keys` buffer size (around line 616): append `+ LIGHTING_SC_PREFILL_MAX` words. Add a Rust constant `LIGHTING_SC_PREFILL_MAX: u64 = 16384;` and a comment `// S73E: + pre-fill list`.
3. `lighting_gpu.rs`, the count resets: add `LIGHTING_WRC_COUNTS_AT + 8` to the `for at in [...]` list (about line 865). In the world path (about line 1055), add `(self.fns.cmd_fill_buffer)(cmd, self.lighting.cache_keys.buffer, (LIGHTING_WRC_COUNTS_AT + 8) * 4, 4, 0);`.

No behaviour change. Commit.

### S73F: compact lists never-shown texels on the pre-fill list only
File `crates/render/shaders/lighting_compact.comp`: right after the closing `}` of the `if (wrc && !burst && LIGHTING_SC_SCHED) { ... }` block, and before the `// S72D sparse fill` comment, insert:
```glsl
        // S73F: a pre-fill texel (never shown) is due when a ray read it since
        // its last relight (Lumen LastUsed - LastUpdated); it waits on its own
        // list, which takes only what the round leaves (lighting_cache_batch_slot).
        if (LIGHTING_SC_SCHED && !wrc && lighting_sc_prefill(slot)) {
            if (int(lighting_frame() - last) > int(LIGHTING_CACHE_LIFE)) {
                atomicAdd(gstats.freed, 1u);
                ckeys.key[slot] = 0u;
                cache.v[3u * slot + 1u] = vec4(0.0);
                cache.v[3u * slot + 2u] = vec4(0.0);
                return;
            }
            uint upd = lighting_sc_updated(slot);
            if (upd == 0u || int(last - upd) > 0) {
                uint at = atomicAdd(ckeys.key[LIGHTING_SC_PREFILL_COUNT_AT], 1u);
                if (at < (LIGHTING_SC_PREFILL_MAX >> lighting_sc_budget_shift())) {
                    ckeys.key[LIGHTING_SC_PREFILL_AT + at] = slot;
                }
            }
            return;
        }
```
Nothing reads the list yet, so pre-fill texels are idle after this commit. That is still a valid state.

### S73G: the batch takes the pre-fill list from spare base budget
File `lighting_cache.glsl`:
1. Add a helper before `lighting_write_args`:
```glsl
// S73G: pre-fill entries this round: what young + mature leave of the base
// ceiling (never the boost headroom).
uint lighting_sc_prefill_take(uint first, uint mature) {
    uint base = lighting_sc_base_room() >> lighting_sc_budget_shift();
    uint used = first + mature;
    uint have = min(ckeys.key[LIGHTING_SC_PREFILL_COUNT_AT], LIGHTING_SC_PREFILL_MAX >> lighting_sc_budget_shift());
    return used < base ? min(have, base - used) : 0u;
}
```
2. In `lighting_write_args` (mode 3), replace `a = min(room, first + lighting_mature_count());` with `a = min(room, first + lighting_mature_count() + lighting_sc_prefill_take(first, lighting_mature_count()));`.
3. In `lighting_cache_batch_slot`, replace
`if ((lighting_young_only() && lighting_world_mode() != 3u) || k >= count) { return ~0u; }` with:
```glsl
    if (lighting_young_only() && lighting_world_mode() != 3u) {
        return ~0u;
    }
    if (k >= count) {
        uint p = k - count;
        uint pf = lighting_world_mode() == 3u && LIGHTING_SC_SCHED ? lighting_sc_prefill_take(first, count) : 0u;
        if (p >= pf) {
            return ~0u;
        }
        uint have = min(ckeys.key[LIGHTING_SC_PREFILL_COUNT_AT], LIGHTING_SC_PREFILL_MAX >> lighting_sc_budget_shift());
        return ckeys.key[LIGHTING_SC_PREFILL_AT + (p + lighting_hash(lighting_round()) % have) % have];
    }
```
`have > 0` whenever `pf > 0`. Each pre-fill texel takes one entry, a quarter set; the quarters add up in `lighting_cache.comp` (`q < LIGHTING_SC_QUARTERS`). Mind the case where the room is cut short of `first + count`: `pf` is 0 then, because `used >= base`.

Commit message: "budget is a ceiling: spare base budget only, never in the histogram or boost; deviation: Lumen spends a fixed budget; rotating start is ours".

### S73H: docs
- `docs/systems/lighting/README.md`: add index rows for S73C-S73G, and drop the README principle "Off-screen pre-fill ... waits until there is spare performance" in favour of "pre-fill uses spare base budget only".
- Add the new deviations to "Deviations from the papers".
- Update the result lines in the S73A and S73B experiment files to point at this file.

### Verification (owner's PC, RTX 4070)
- **Brightness:** smoke mean luminance at pose a against `gi-v2` e61b89d, and against 71547c9 (the darkening port). Expect e61b89d to match the fix, and 71547c9 to be darker.
- **Turns and walking backwards:** the fill run (`fill.rhai`) and the turn strip should show fewer empty texels filling in at the screen edge.
- **Cost:** still and moving ms. Still should be unchanged, because scouts fire 1 round in 8 and pre-fill texels are due only when re-read. Moving gains only spare entries.
- **Pop share:** at frames 20-80 and 80-320.
- **A/B within the branch:** set `LIGHTING_SC_GUARD = false` to separate the guard band from the plain scout feedback.
