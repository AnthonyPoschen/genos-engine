# COLD — lighting-ref cold scoring (PR #37) — dropped 2026-10-19

PR #37 (`cursor/lighting-ref-cold-a764`) scored each lighting-ref checkpoint from a fresh renderer so residual state could not leak between poses. That is still a useful quality-gate idea.

## Why it did not land

Merging `origin/master` into the branch produced conflicts in six files, including a large divergence in `crates/render/shaders/lighting_cache.glsl` (hundreds of lines) plus `lighting_compact.comp`, `lighting_gpu.rs`, and the cold-scoring paths under `examples/stress/src/lighting_ref/`. Master has moved the lighting shaders and the ref harness since the PR was cut. Untangling that on the PR would be a fix-up, which the drop rule forbids.

Lavapipe / 4070 spot-checks were not run: the tree did not compile as a clean merge.

## What to do next

Re-open cold scoring as a new branch off current master that touches only `examples/stress/src/lighting_ref/` and the ref tests, with no shader or `lighting_gpu.rs` changes unless a failing cold score proves they are required. Keep each checkpoint's renderer construct/teardown behind an explicit CLI flag so A/B cost stays optional.
