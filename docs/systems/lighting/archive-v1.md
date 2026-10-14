# GI v1 archive

GI v1 (radiance cascades, the world probe tier, `GENOS_GI`, and the v1 shaders and modules) was removed on 2026-10-12 (commit `6a3cd7c`, branch `gi-v2`). Nothing in the engine runs that path.

The live lighting system is [README.md](README.md). The accepted decision is [ADR 0012](../../adr/0012-gi.md). Historical terms stay in the [glossary](../../lighting/GLOSSARY.md) Historical section and must not be reused in code.

## Where the old docs live

The full v1 notes were deleted from the tree. Recover them from git history on this branch (or on `master` before the GI cutover), for example:

| Former path | Content |
|---|---|
| `docs/lighting.md` | Working notes for radiance cascades |
| `docs/systems/lighting-v1.md` | Systems note for the v1 path |
| `docs/systems/lighting-handoff.md` | Pre-GI local-update / occluder-grid handoff |
| `docs/references/radiance-cascades-diff.md` | Cascades vs paper |
| `docs/systems/gi-v2-baseline.md` | Gate numbers on tag `gi-v2-baseline` (master `0874229d`) |

```text
git show gi-v2-baseline:docs/lighting.md
git log --all -- docs/systems/lighting-v1.md
```

Superseded ADRs [0008](../../adr/0008-lighting-uses-radiance-cascades.md) and [0011](../../adr/0011-occluders-use-a-sparse-camera-box.md) remain as short stubs.

## Baseline numbers (tag `gi-v2-baseline`)

Recorded on zanven-pc (Linux, RTX 4070), 2026-10-10, against the then-master lighting (GI v1). Full tables were in `docs/systems/gi-v2-baseline.md`.

| View (320×180, CPU reference) | mean_rel | bias |
|---|---|---|
| hall | 0.043 | +0.001 |
| room-a | 0.198 | +0.107 |
| doorway | 0.040 | +0.016 |
| corner | 0.272 | +0.272 |
| noon / night | 0.055 / 0.058 | −0.026 / +0.049 |

v1 flicker gate (worst frozen max_delta, limit 0.004): 0.046 FAIL (hall). Lamp-colour settle: 92 frames.
