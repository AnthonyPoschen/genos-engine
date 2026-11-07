# Cache-energy precompute

Intent: the stage-2 acceptance trace stays 600 s, noise 0.02, and 4096 spp. The hall lit 64-bounce reference did not meet 0.02 inside that budget (160×90, 672 spp, noise 0.032146, about 610 s). The saved `.ref` has no sample sums, so nothing resumes it. This note is a harness-local way to trace the same setup for longer, then copy a converged file into the acceptance cache. It is not a new renderer, cache format, or subsystem. Stage 2 stays unaccepted until the acceptance tests pass.

## What is shared

`acceptance_setup` in `examples/stress/tests/cache_energy.rs` is the only constructor. It uses the harness hall (or the other acceptance scenes), `RefSetup::from_camera` at 160×90 with a 32-wide batch, bounce count, noise 0.02, and 4096 spp. `trace_acceptance` calls it with 600 s and `genos_debug::reference::render_cached` on `target/reference-cache`.

`precompute_setup` calls `acceptance_setup` and changes `seconds` only. The seconds must be finite and in `(0, 2400]`. The estimator, the noise target, the spp ceiling, CPU analytic shapes (`triangles == false`, no material overrides, `specular == false`), and the full cached picture (linear radiance plus hit) stay as they are.

The first offline entry is the lit hall at 64 bounces. Its camera is the harness pose `(14.5, 1.7, 23.5)`, yaw `-0.3554`, pitch `0.08`. That yaw is not `View::Hall` (`--view hall`). The route does not call the stress reference CLI.

## Key and provenance

`render_cached` names the file from `cache_key`. The key hashes the scene `Debug` text, eye, yaw, pitch, width, height, and bounce count. Seconds are not in the key. Triangles and a GPU trace add their own suffixes. An integrator version is not in the key: a file from another commit can collide. That is the existing format. This change does not put a version in it.

The key is printed in hex. Provenance is a sidecar next to the isolated `.ref`, not a change to the `.ref` bytes. It records the source commit, a clean worktree, the scene name, the pose bits, the full setup, the key, and the noise and spp that came back. A dirty tree is refused before the trace, so the binary matches `HEAD`.

## Isolation and promotion

The trace writes under `target/cache-energy/precompute-isolated/<commit>/<key>/`. It does not read `target/reference-cache` first. An isolated `.ref` with missing provenance, a different commit, or a setup that is not this acceptance setup is refused before `render_cached`, which would otherwise return a converged file with no question. A same-commit file whose provenance matches is reused only with that fact printed. The cache still cannot continue a sum; a short file is traced again from the first sample.

Promotion copies into the acceptance cache only when all of these hold: the live precompute setup matches acceptance except seconds; both keys match the file name; the commit is this clean `HEAD`; noise is finite and at most 0.02; spp is in `1..=4096`; linear samples and hits are finite and non-empty; the file loads through the existing reader and matches that picture. A noisy cap, a NaN, an empty picture, or a wrong pose or commit leaves the acceptance cache untouched. A converged trace that used the final pass at the spp or time cap is valid. Acceptance still calls the 600 s setup. `render_cached` will then reuse the promoted file because the key matches and the noise is low enough.

`research_continue` stays outside the acceptance cache and still cannot fill it.

## What this does not fix

A `.ref` already sitting in `target/reference-cache` with noise at or under 0.02 is still accepted by `render_cached`, even if an older integrator wrote it. This route does not delete that file and does not treat it as proof. It overwrites the key only after an isolated trace at this commit passes the gates. Until that promotion, stage 2 is unaccepted.

## Command

The next trace is this process, not a hand-aimed CLI. It is the lit hall, 64 bounces, at most 2400 s. It is not an acceptance run.

```sh
CACHE_ENERGY_PRECOMPUTE_SECONDS=2400 cargo test --release -p genos-stress --test cache_energy -- --ignored --exact --nocapture --test-threads=1 precompute_hall_lit64
```

Acceptance ignores that variable. `acceptance_setup` stays 600 s. The variable is only the bound for this ignored test, and a value outside `(0, 2400]` is refused before device work and before the trace.
