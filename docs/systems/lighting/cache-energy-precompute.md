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

The trace writes under `target/cache-energy/precompute-isolated/<commit>/<key>/`. It does not read `target/reference-cache` first. An isolated `.ref` with missing provenance, a different commit, or a setup that is not this acceptance setup is refused before `render_cached`, which would otherwise return the saved file with no question. A same-commit file whose provenance matches is reused, and that fact is printed. `render_cached` returns the saved file when its noise is at or under 0.02 or its spp is already at the setup cap, so a capped noisy file is kept and is not traced again. The cache still cannot continue a sum. A file under the cap whose noise is above 0.02 is traced again from the first sample.

Promotion copies into the acceptance cache only when all of these hold: the live precompute setup matches acceptance except seconds; both keys match the file name; the commit is this clean `HEAD`; noise is in `0..=0.02`; spp is in `1..=4096`; linear samples and hits are finite and non-empty; the file loads through the existing reader and matches that picture. The copy stages the `.ref` and the sidecar under temporary names in the acceptance directory, checks that the staged `.ref` loads and matches the isolated bytes, then renames the `.ref` and then the sidecar. A failure before the `.ref` rename leaves the existing pair byte for byte. The two renames are not one transaction. If the `.ref` rename has succeeded and the sidecar rename fails, the error is returned and that new `.ref` may remain beside the previous sidecar. The picture was already checked before that rename: finite samples, a hit, and noise in `0..=0.02`. `render_cached` reads the `.ref` only. It does not read the sidecar. A noisy cap, a NaN, a negative noise, an empty picture, or a wrong pose or commit leaves the acceptance cache untouched. A converged trace that used the final pass at the spp or time cap is valid. Acceptance still calls the 600 s setup. `render_cached` will then reuse the promoted file because the key matches and the noise is in band, or because spp is already at the acceptance cap.

## Open floor sample cap

The hall path above does not move `max_spp`. The open floor at 64 bounces can finish 4096 spp while the mean relative standard error is still above 0.02. The saved file has no sample sums, so that file cannot be continued. `render` already takes `max_spp` from the setup. The floor offline trace sets that cap explicitly, in `1..=16384`, and the seconds in `(0, 2400]`. 16384 and 2400 s are execution limits. They are not a noise target and not a prediction that the picture will converge.

Acceptance stays 600 s, 4096 spp, noise `0..=0.02`, batch 32, the CPU analytic picture, the full linear-plus-hit cache, the floor pose, and 64 bounces. The cache key still hashes scene, eye, yaw, pitch, size, and bounce count. It does not hash the sample cap or the seconds. A file from another budget is not trusted. The sidecar's `offline_max_spp` is read as one integer through the harness JSON field reader. A different integer, including a decimal prefix of `16384` such as `1`, `16`, `163`, or `1638`, is refused before `render_cached`. That refusal does not rewrite the isolated `.ref` or its sidecar. The sidecar is metadata. It is not a proof of the samples.

The isolated sidecar records the trace clock in `seconds`. A reused `.ref` has no clock (the loader reports 0), so the previous sidecar clock is kept when it is a positive finite number. If that clock is missing, `seconds` is null, not 0. `promoted` is written after the acceptance copy and is true only when that copy succeeded.

An spp above 4096 is copied only when that explicit cap is inside 16384 and the picture is finite, has a hit, and noise is in `0..=0.02`. A noisy picture at the cap is blocked. The acceptance copy uses the same stage-then-rename as the hall. A failure before the `.ref` rename leaves the acceptance pair byte for byte, including a file that was already there. A sidecar rename that fails after that `.ref` rename can leave the converged `.ref` in place. There is no second trace and no pick among replicates.

```sh
CACHE_ENERGY_FLOOR_MAX_SPP=16384 CACHE_ENERGY_FLOOR_SECONDS=2400 cargo test --release -p genos-stress --test cache_energy -- --ignored --exact --nocapture --test-threads=1 precompute_floor_b64
```

Acceptance ignores both variables. A missing value, a value outside the range, or a number that does not fit the type is refused before the trace.

`research_continue` stays outside the acceptance cache and still cannot fill it.

## What this does not fix

A `.ref` already sitting in `target/reference-cache` with noise at or under 0.02 is still accepted by `render_cached`, even if an older integrator wrote it. This route does not delete that file and does not treat it as proof. It overwrites the key only after an isolated trace at this commit passes the gates. Until that promotion, stage 2 is unaccepted.

## Command

The next trace is this process, not a hand-aimed CLI. It is the lit hall, 64 bounces, at most 2400 s. It is not an acceptance run.

```sh
CACHE_ENERGY_PRECOMPUTE_SECONDS=2400 cargo test --release -p genos-stress --test cache_energy -- --ignored --exact --nocapture --test-threads=1 precompute_hall_lit64
```

Acceptance ignores that variable. `acceptance_setup` stays 600 s. The variable is only the bound for this ignored test, and a value outside `(0, 2400]` is refused before device work and before the trace.
