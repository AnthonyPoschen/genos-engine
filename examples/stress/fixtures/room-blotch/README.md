# Room blotch bundle (blocked)

This file is the host copy. The cloud measurement did not accept a reference. There is no `.ref` in this directory. `room-a.UNACCEPTED.json` is the sidecar of a trace the judge rejected. Do not rename it to `room-a.json`, and do not place an open-floor reference here.

## Verdict

Class: `noisy-reference`. Not `physical-fail` and not `adr-pass`. The ADR limit stays 0.014. The gi-check room row stays 0.023. No mean was scored.

room-a CPU analytic trace, one attempt, no retry:

- noise `0.014457642` (bits `1013768127`), above `0.01`
- spp `72064`, under the `262144` cap, so the cap did not accept the file
- seconds `900.4743` (bits `1147215451`). The 900 s budget ended on the pass that crossed it. The clock was still at the cap. This is not a settled picture.
- size 320×180, 64 bounces, shapes, batch 64, file `748816` bytes
- `room-a.ref` sha256 `837b2bbdee61d3856b0d96248340d19843cee594a8047cf5d17987b1f57fe5ec`
- sidecar sha256 `dcba80712334a89ad7d3e33ae9bdd12608bebf84e0d522138dab7b72ae676384`
- native cache key `ec05ef86493fec68` (recorded only; the grade does not look it up)
- promote directory was not created

room-b was not started. room-c and close-0 were not traced. No GPU window was opened. `lifecycle_frames_presented` is absent. There is no settle frame count, because capture did not run.

`genos-stress room-blotch grade --refs` on the isolated directory exited 1. The first issue is the noise. The same verdict also lists `missing-reference` for room-b and `missing-capture`, because those steps were not run after the noise failure. `grade-verdict.json` is that output. Exit 0 is only `adr-pass`.

## Why the next step is not a renderer change

The estimator ran for the whole 900 s budget at 4 threads and was still noisier than 0.01, with samples left under the spp cap. A threshold change, a rename of a floor reference, or a second trace was not allowed. A later measurement, if one is authorized, is the same CPU estimator with a budget that can reach noise at or under 0.01. This bundle does not authorize that rerun.

## Source and binary

- Trace commit: `24a3d3594a0b5ab4b2e9248544e191b331071934`
- Parent: `779e9a67eb67b92d02428bb87af450e1b7adc723` on `cursor/stage2-energy-lamp-6773`
- Branch: `cursor/room-blotch-harness-c942`
- The grade checks the scene sha256 of the binary's `scenes/room.rhai`, not that HEAD equals the trace commit. This README is a later commit.
- scene sha256: `6ae0db79540c5431b8c3ea5e38c856888b76a875fade1b1d0bb3b8ab360ad180`
- rustc `1.99.1 (58cf8f9c3 2026-10-13)`
- binary: `target/release/genos-stress`
- binary sha256: `c33e1d65f3fe60bd633073619f5c1313e253cf4aab88c62841cc4ec94405274f`
- build: `cargo build -j 1 --release --locked -p genos-stress --bin genos-stress` (exit 0)

The room file's sun line is commented out. `--freeze-sun` holds `time_of_day` at the stress start hour (bits `1091441787`). Sun height reads directional lights only, so the freeze does not change this room's lamp light. E11's 0.014 is the W7 12.5 cm gi-check blotch. This run does not claim that score. No live cells-only picture was captured.

## Commands that produced this result

```sh
unset GENOS_REFERENCE_GPU GENOS_LIGHTING_ASYNC GENOS_LIGHTING_PRESET
unset GENOS_LIGHTING_WORLD GENOS_LIGHTING_WORLD_CELL GENOS_LIGHTING_SC_FLAGS GENOS_LIGHTING_TOGGLES
nice -n 19 env GENOS_REFERENCE_THREADS=4 timeout -k 10 960 \
  ./target/release/genos-stress room-blotch trace \
  --pose room-a --threads 4 \
  --out ./genos-debug/room-blotch/isolated \
  --promote ./genos-debug/room-blotch/promoted
```

Trace exit 1: `noisy-reference`. The isolated `.ref` stays under `genos-debug/` and is not an accepted operand. `target/reference-cache` was not written.

```sh
./target/release/genos-stress room-blotch grade \
  --refs ./genos-debug/room-blotch/isolated \
  --out ./genos-debug/room-blotch/grade
```

Grade exit 1. No `--capture`.

## Tests

`cargo test -j 1 -p genos-stress --bin genos-stress room_blotch` — 16 passed, exit 0.

`cargo clippy -j 1 -p genos-stress --bin genos-stress --all-targets --no-deps -- -D warnings` — exit 0.

The blotch function is unchanged. An 8×8 step (live 0.2/0.8, reference 0.5, s=4) scores `0.17401275553535756`. Identical planes and a uniform bias score under `1e-12`. `ROOM_BLOTCH_LIMIT` is still `0.023`. A mean of 0.020 is `physical-fail` even though the legacy word would be PASS.

Negative controls, each a different class from `physical-fail`:

- `missing_refs_do_not_become_a_physical_fail`
- `noisy_cap_floor_size_and_wrong_pose_stay_distinct` (noise 0.02 at 262144 spp, a 187216-byte floor file, a yaw-bit mismatch, a 4194304 max-spp sidecar, a zero trace clock, NaN, negative noise)
- `known_floor_hashes_are_refused_and_a_room_hash_is_not`
- `settle_timeout_is_unsettled_and_a_true_settle_at_900_is_not`
- `the_other_pose_and_a_short_wait_are_distinct`
- `resolved_flags_require_cells_only_sh_async_off_and_25_cm`
- `a_guard_hides_a_score`
- `legacy_limit_stays_0_023_and_is_not_the_adr_verdict`
- `the_script_has_two_poses_and_does_not_call_compare`
- `native_key_changes_with_time_and_sh_cells_and_grade_ignores_it`
- `an_accepted_pair_grades_only_through_blotch_and_does_not_write_the_cache`

## 4070

Do not grade this directory. It has no accepted reference. Do not point `VK_ICD_FILENAMES` at lavapipe on that machine.

```sh
unset VK_ICD_FILENAMES VK_DRIVER_FILES
vulkaninfo --summary
```

The device name must be the RTX 4070, discrete, and not lavapipe, llvmpipe, or device type 4. Then, from a clean checkout of the harness commit:

```sh
unset GENOS_REFERENCE_GPU GENOS_LIGHTING_ASYNC GENOS_LIGHTING_PRESET
cargo build -j 1 --release --locked -p genos-stress --bin genos-stress
```

A new trace uses the same flags as the cloud command above. Noise above 0.01 stays unaccepted, including when spp reaches 262144. `--capture` is only for a directory whose room-a and room-b files already pass the judge. The capture is one window: `--freeze-sun`, `--lighting-toggles cells_only`, `--lighting-cache-async off`, and `GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS=1` set on the child. `sh_cells` has no command-line flag. The child drops `GENOS_REFERENCE_GPU`, `GENOS_LIGHTING_ASYNC`, and the other lighting overrides listed in `docs/systems/lighting/room-blotch.md`.
