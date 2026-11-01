# Lint rules

Date of this catalog: 2026-10-14.

Rules live in config, not in review comments. `./scripts/lint.sh` runs all of them. A new crate opts in with `[lints] workspace = true`, which every crate in this workspace already does.

Counts below are unique source locations from `cargo clippy --workspace --all-targets` on 2026-10-14 (a lib and its unit tests are not counted twice). The tree still contains those sites. Each site sits under a `#[expect]` or `#[cfg_attr(test, expect)]` with a `reason`, so the check is on and new files are not covered by the old expects. Removing an expect is the cleanup. Do not add a new expect to silence a new site.

`crates/render/src/lighting_gpu.rs`, `lighting_dispatch.rs`, and `aa_gpu.rs` are `include!`d into `gpu.rs`. Their expects are on `gpu.rs`, because an inner attribute in an included file is not at the top of the module.

## Workspace clippy

`Cargo.toml` sets `[workspace.lints.clippy]`. `pedantic` is `warn` at priority `-1`, so an explicit deny on one of its lints wins. `[workspace.lints.rust] warnings = "deny"` makes that warning fail `cargo clippy`. The lint script also passes `-- -D warnings`.

| Rule | Level | Sites | Files | Added | Why |
|---|---|---|---|---|---|
| `unwrap_used` | deny | 441 (143 test-only) | 57 | 2026-10-14 | A library path should return the error. `.unwrap()` hides it. |
| `expect_used` | deny | 386 (323 test-only) | 63 | 2026-10-14 | Same as unwrap, with a message. |
| `panic` | deny | 41 (19 test-only) | 20 | 2026-10-14 | `panic!` is not an error path. |
| `todo` | deny | 0 | 0 | 2026-10-14 | `todo!` does not ship. |
| `dbg_macro` | deny | 0 | 0 | 2026-10-14 | `dbg!` is not a log. |
| `wildcard_enum_match_arm` | deny | 37 | 17 | 2026-10-14 | `_` on an enum drops a new variant on the floor. |
| `as_conversions` | deny | 1737 (252 test-only) | 122 | 2026-10-14 | A raw `as` truncates or wraps. GPU buffer sizes are the case that matters. |
| `cast_possible_truncation` | warn | 443 lines | | 2026-10-14 | Lossy `as`. Warn on the backlog, deny a new line. See the burndown table. |
| `cast_possible_wrap` | warn | 77 lines | | 2026-10-14 | Same family. A line in more than one of these three counts once in the 560. |
| `cast_sign_loss` | warn | 196 lines | | 2026-10-14 | Same family. |
| `undocumented_unsafe_blocks` | deny | 125 | 12 | 2026-10-14 | Every `unsafe` block needs a `// SAFETY:` comment. Vulkan and OS FFI are the usual sites. |
| `pedantic` | warn | 3068 | (100 lint names) | 2026-10-14 | The extra clippy pass. Priority `-1` so the denies above still win. |
| `disallowed_methods` | deny | 24 | 7 | 2026-10-14 | See `clippy.toml`. |
| `disallowed_types` | deny | 4 | 1 | 2026-10-14 | See `clippy.toml`. |

Bad `unwrap` (the lint fails this):

```rust
let port = std::env::var("GENOS_MCP_PORT").unwrap();
```

Bad cast:

```rust
let bytes = words.len() as u32;
```

`todo` and `dbg_macro` have no backlog. A new `todo!` or `dbg!` fails the build.

### Accepted exceptions

- **Tests.** `unwrap`, `expect`, `panic`, `todo`, and `dbg` in test-only code use `reason = "a test aborts when a fixture fails"`.
- **Build scripts.** The same lints in `build.rs` use `reason = "the build stops when a host tool or OUT_DIR is missing"`.
- **`ash` in `genos-gpuref`.** `disallowed_types` on `ash::Entry`, `ash::Instance`, and `ash::Device` is expected there: `reason = "offline ray-query reference; the renderer loads libvulkan itself (ADR 0007)"`. Four sites, all in `crates/gpuref/src/lib.rs`.

### TODO: clippy backlog

Do not refactor these in a drive-by. Delete the `expect` when the sites in that file are gone.

- `as_conversions`: 1737 sites. The GPU files are `crates/render/src/gpu.rs` (and the three files it includes), `pack.rs`, `occ_grid.rs`, `mesh_field.rs`. Pointer casts in those files are FFI. Length and size casts are the ones to replace with `TryFrom`.
- `undocumented_unsafe_blocks`: 125 sites in `crates/render/src/gpu.rs` (including `aa_gpu.rs`, `lighting_gpu.rs`, `lighting_dispatch.rs`, `debug_view.rs`), `crates/window/src/lib.rs`, `crates/audio/src/device.rs`, `crates/input/src/system.rs`, `crates/gpuref/src/lib.rs`, `crates/mcp/src/discover.rs`, `crates/physics/tests/bench.rs`, `examples/stress/src/lighting_ref/capture.rs`. Add `// SAFETY:` and drop the expect.
- `unwrap_used` / `expect_used` / `panic`: production sites are the totals above minus the test-only column. Build scripts stay allowed.
- `wildcard_enum_match_arm`: 37 sites, 17 files.
- `pedantic`: 3068 sites under per-lint expects. A file that expects `cast_possible_truncation` still fails a different pedantic lint it did not already expect.
- `disallowed_methods` (`std::env::set_var`): 24 sites in `debug/lighting/src/main.rs`, `examples/stress/src/main.rs`, `examples/stress/src/lighting_ref/capture.rs`, `crates/render/tests/lighting_frame.rs`, `crates/render/tests/lighting_sun.rs`, `crates/render/tests/lighting_toggles.rs`, `crates/debug/tests/lighting_mcp.rs`. These push a CLI flag into the environment so another crate will read it. Write the settings value instead.

One real bug found while turning the lints on: `crates/ui/src/profile.rs` computed `1.0_f32.min(plot_w.max(1.0))`, which is always `1.0`. The mark width is now `1.0_f32.min(plot_w)`.

## clippy.toml

| Banned item | Added | Why | Bad example |
|---|---|---|---|
| `std::env::set_var` | 2026-10-14 | A tool publishes intent with a command-line argument. The argument wins. Mutating the process environment is not that argument. | `std::env::set_var("GENOS_LIGHTING_PRESET", "high");` |
| `std::env::remove_var` | 2026-10-14 | Clearing the environment is the same kind of switch. | `std::env::remove_var("GENOS_LIGHTING_PRESET");` |
| `std::env::home_dir` | 2026-10-14 | Output lives under `./genos-debug/<kind>/`. Do not ask for a home directory. | `let root = std::env::home_dir().unwrap();` |
| `std::env::var` | 2026-10-14 | Library code hides a tuning knob from the caller and from tests. Read it in `main` and pass the config struct. | `std::env::var("GENOS_LIGHTING_TILE")` |
| `std::env::var_os` | 2026-10-14 | Same rule as `std::env::var`. | `std::env::var_os("GENOS_LIGHTING_DEBUG")` |
| `std::thread::sleep` | 2026-10-14 | A fixed sleep wastes time when the event is early and flakes when it is late. | `std::thread::sleep(Duration::from_millis(4000))` |
| `ash::Entry`, `ash::Instance`, `ash::Device` | 2026-10-14 | ADR 0007. The renderer loads `libvulkan` itself. | `fn open() -> ash::Device { unimplemented!() }` |

`lint/fail/` is not a workspace member, so those examples are not compiled with the workspace. `lint/fixture/fail` is its own package. `scripts/lint-clippy-fixture.py` expects clippy to reject `std::env::var`, `std::env::var_os`, and `std::thread::sleep` there, and expects `lint/fixture/pass` to stay clean. The 24 `set_var` sites are still the backlog for that method.

`std::env::var` and `std::env::var_os` are denied in every crate. Existing reads carry a function `#[allow(clippy::disallowed_methods, reason = "...")]`. A new read in a function that does not already have that allow fails. Build scripts are allowed at the file: Cargo passes `OUT_DIR` and the target triple. `GENOS_MCP_PORT` stays allowed on `listen_port` because the discovery directory is the primary path and that variable is the documented override. `HOME`, `XDG_STATE_HOME`, `XDG_RUNTIME_DIR`, and `WAYLAND_DISPLAY` stay allowed on the one function that reads them.

These files already `expect` `disallowed_methods` for `set_var`, so a new `env::var` in the same file is not gated yet: `examples/stress/src/main.rs`, `examples/stress/src/lighting_ref/capture.rs`, `debug/lighting/src/main.rs`, `crates/render/tests/lighting_frame.rs`, `crates/render/tests/lighting_sun.rs`, `crates/render/tests/lighting_toggles.rs`, `crates/debug/tests/lighting_mcp.rs`. They are binaries and tests. The renderer library is gated per function.

### TODO: renderer tuning reads

The renderer still reads 35 environment names itself. Each one should move onto the lighting config struct. Thirteen already have a CLI flag; the read should use the struct the flag writes. Twenty-two have no flag yet. Do not add a new `kind = "todo"` row while moving them: add the flag, then mark the name `mirror`.

`GENOS_LIGHTING_SC_LOCAL` left the renderer when PR #45 moved mover attachment onto `surface_cache.attach_movers`. `apply_env` still reads it, and it reads the five new mirrors below. That function already allows `disallowed_methods`. The new names are mirrors, not a new todo:

| Variable | Where | CLI |
|---|---|---|
| `GENOS_LIGHTING_SURFACE_CACHE_ATTACH_MOVERS` | `crates/scene/src/lighting.rs:1244` | `--surface-cache-attach-movers` |
| `GENOS_LIGHTING_SC_LOCAL` | `crates/scene/src/lighting.rs:1245` | `--surface-cache-attach-movers` (previous name) |
| `GENOS_LIGHTING_SURFACE_CACHE_BLEND` | `crates/scene/src/lighting.rs:1246` | `--surface-cache-blend` |
| `GENOS_LIGHTING_SURFACE_CACHE_FIREFLY_SIGMA` | `crates/scene/src/lighting.rs:1247` | `--surface-cache-firefly-sigma` |
| `GENOS_LIGHTING_SURFACE_CACHE_RAY_BUDGET` | `crates/scene/src/lighting.rs:1248` | `--surface-cache-ray-budget` |
| `GENOS_LIGHTING_SURFACE_CACHE_DORMANT_RAYS` | `crates/scene/src/lighting.rs:1249` | `--surface-cache-dormant-rays` |
| `GENOS_LIGHTING_DIRECT_SPECULAR` | `crates/scene/src/lighting.rs` | `--lighting-direct-specular` |

| Variable | Where | CLI today |
|---|---|---|
| `GENOS_GPU_DISPATCH` | `crates/render/src/gpu.rs:2315` | none |
| `GENOS_GPU_TIMES` | `crates/render/src/gpu.rs:1448` | `--gpu-times` |
| `GENOS_GI_METRICS` | `crates/render/src/gpu.rs:1550` | `--gi-metrics` |
| `GENOS_LIGHTING_ASYNC` | `crates/scene/src/lighting.rs:1529` | `--lighting-cache-async` |
| `GENOS_LIGHTING_ATROUS_DIRTY` | `crates/scene/src/lighting.rs:1529` | `--lighting-denoise-dirty` |
| `GENOS_LIGHTING_BORROW` | `crates/render/src/lighting_gpu.rs:2246` | none |
| `GENOS_LIGHTING_CACHE_HZ` | `crates/scene/src/lighting.rs:1529` | `--lighting-cache-hz` |
| `GENOS_LIGHTING_CACHE_PICKS` | `crates/scene/src/lighting.rs:1529` | `--lighting-cache-picks` |
| `GENOS_LIGHTING_CATCH_UP` | `crates/render/src/lighting_gpu.rs:3250` | none |
| `GENOS_LIGHTING_COMPOSE_ALL` | `crates/render/src/lighting_gpu.rs:2286` | `--full-effort` |
| `GENOS_LIGHTING_DEBUG` | `crates/render/src/lighting_gpu.rs:3254` | `--lighting-debug` |
| `GENOS_LIGHTING_DUMP` | `crates/render/src/lighting_gpu.rs:2669` | none |
| `GENOS_LIGHTING_DUMP_DIR` | `crates/render/src/lighting_gpu.rs:2723` | none |
| `GENOS_LIGHTING_FILL` | `crates/scene/src/lighting.rs:1529` | `--lighting-fill` |
| `GENOS_LIGHTING_FULL_EFFORT` | `crates/render/src/lighting_gpu.rs:3243` | `--lighting-full-effort` |
| `GENOS_LIGHTING_LAMP_PICK_DIAG` | `crates/render/src/lighting_dispatch.rs:91`, `crates/render/src/gpu.rs:2205` | none |
| `GENOS_LIGHTING_MOVED_DROP` | `crates/render/src/lighting_gpu.rs:2244` | none |
| `GENOS_LIGHTING_SC_FLAGS` | `crates/scene/src/lighting.rs:1529` | `--lighting-sc-flags` |
| `GENOS_LIGHTING_SC_SETTLE` | `crates/scene/src/lighting.rs:1529` | `--lighting-sc-settle` |
| `GENOS_LIGHTING_SC_TRACE` | `crates/render/src/lighting_gpu.rs:1725`, `crates/render/src/lighting_gpu.rs:2934` | none |
| `GENOS_LIGHTING_SUN` | `crates/render/src/lighting_gpu.rs:3240` | `--lighting-sun` |
| `GENOS_LIGHTING_SUN_BANDS` | `crates/render/src/lighting_gpu.rs:3246` | `--lighting-sun-bands` |
| `GENOS_LIGHTING_TILE` | `crates/scene/src/lighting.rs:1529` | `--lighting-probe-tile` |
| `GENOS_LIGHTING_TOGGLES` | `crates/scene/src/lighting.rs:1529` | `--lighting-toggles` |
| `GENOS_LIGHTING_UNFILLED` | `crates/render/src/unfilled.rs:193` | `--unfilled` |
| `GENOS_LIGHTING_UNFILLED_DUMP` | `crates/render/src/unfilled.rs:195` | `--unfilled-dump` |
| `GENOS_LIGHTING_UNFILLED_TINT` | `crates/render/src/unfilled.rs:194` | `--unfilled-tint` |
| `GENOS_LIGHTING_URGENT` | `crates/render/src/lighting_gpu.rs:2245` | none |
| `GENOS_LIGHTING_WORLD_CAP` | `crates/scene/src/lighting.rs:1529` | `--lighting-world-cap` |
| `GENOS_LIGHTING_YOUNG_ONLY` | `crates/scene/src/lighting.rs:1529` | `--lighting-young-only` |
| `GENOS_SCENE_DELTA` | `crates/render/src/lib.rs:56` | none |
| `GENOS_UNFILLED_MAX_AGE_SECS` | `crates/render/src/unfilled.rs:198` | `--unfilled-max-age-secs` |
| `GENOS_UNFILLED_MAX_BYTES` | `crates/render/src/unfilled.rs:197` | `--unfilled-max-bytes` |
| `GENOS_UNFILLED_MAX_FOLDERS` | `crates/render/src/unfilled.rs:196` | `--unfilled-max-folders` |
| `GENOS_UNFILLED_SESSION_CAP` | `crates/render/src/unfilled.rs:199` | `--unfilled-session-cap` |

`crates/scene/src/lighting.rs` `apply_env` still reads the same names into `LightingSettings`. That function is allowed until `main` passes the struct in. It is not a second copy of this list.

### TODO: fixed sleeps

`std::thread::sleep` is denied. Existing calls are allowed on the function.

The pointer-lock test in `crates/window/src/lib.rs` (`the_first_click_locks_the_pointer_and_escape_releases_it`) waits on the clock instead of a window event:

| Line | Wait |
|---|---|
| 451 | 40 ms, polling for the toplevel |
| 466 | 300 ms after the pointer move |
| 479 | 5 s after the click, for the lock |
| 482 | 4 s holding Escape |
| 484 | 4 s after the key release |

Replace the 4 s and 5 s waits with an event or a timeout wait. The 40 ms and 300 ms waits in that test go with them.

Other allowed sleeps, same rule, shorter: `crates/audio/src/device.rs` (5 ms at 334, 10 ms at 369, 10 ms at 513), `crates/mcp/src/http.rs` (20 ms at 128), `crates/mcp/src/command.rs` (1 ms at 153, test), `crates/mcp/tests/live_session.rs` (10 ms at 344, test).

## Lossy casts

`cast_possible_truncation`, `cast_possible_wrap`, and `cast_sign_loss` are `warn` (added 2026-10-14). `as_conversions` stays deny. A lossy `as` truncates, wraps, or drops the sign. Use `From` or `TryFrom`.

`scripts/lint-clippy.py` runs `cargo clippy --workspace --all-targets -- -D warnings` and `--force-warn` on these three, so the existing `expect` lines do not hide them from the ceiling. `lint/lossy-cast-baseline.txt` is the per-file count of unique lines. A line that trips more than one of the three counts once. A file above its count fails. A file with no row starts at zero, so the first lossy cast there fails. `genos-math` is the crate already at zero.

Measured 2026-10-14: **560** unique lines. Per lint, before collapsing a line that trips two: truncation 443, sign loss 196, wrap 77.

| Crate | src | tests | examples | Total |
|---|---:|---:|---:|---:|
| `render` | 147 | 47 | 0 | 194 |
| `load` | 64 | 5 | 0 | 69 |
| `debug` | 63 | 0 | 0 | 63 |
| `examples/stress` | 43 | 10 | 0 | 53 |
| `lighting-ref` | 36 | 0 | 0 | 36 |
| `ui` | 17 | 11 | 0 | 28 |
| `audio` | 6 | 8 | 3 | 17 |
| `debug/lighting` | 17 | 0 | 0 | 17 |
| `bake` | 16 | 0 | 0 | 16 |
| `gpuref` | 13 | 0 | 0 | 13 |
| `physics` | 10 | 0 | 0 | 10 |
| `mesh` | 5 | 2 | 0 | 7 |
| `examples/camera` | 7 | 0 | 0 | 7 |
| `window` | 6 | 0 | 0 | 6 |
| `mcp` | 4 | 1 | 0 | 5 |
| `scene` | 3 | 2 | 0 | 5 |
| `audio-check` | 0 | 5 | 0 | 5 |
| `load-check` | 0 | 3 | 0 | 3 |
| `input` | 2 | 0 | 0 | 2 |
| `json` | 2 | 0 | 0 | 2 |
| `debug/scene-lib` | 2 | 0 | 0 | 2 |
| `math` | 0 | 0 | 0 | 0 |
| **Total** | **463** | **94** | **3** | **560** |

`examples/stress` and the `debug/` tools are binaries. Their `src` column is that package, not `crates/*/src`. The 394 lines under `crates/*/src` are the library burndown (463 minus the 69 binary-package lines).

Bad cast (the lint warns, and a new line fails the ceiling):

```rust
let bytes = words.len() as u32;
```

### TODO: lossy-cast burndown

Delete the casts, then lower that file's count in `lint/lossy-cast-baseline.txt` in the same change. Do not raise a count to silence a new cast. Start outside `crates/render/src/gpu.rs` so the lighting files stay stable. The GPU length case is also `genos-gpu-len-cast`.

## Shader constants

`scripts/lint-shader-constants.py`, added 2026-10-14. In `*.glsl`, `*.comp`, `*.vert`, and `*.frag`, a fractional literal that is not exactly `0` or `1` must sit on a `const` or `#define` line. `0.0`, `1.0`, and `-1.0` stay allowed. Integers stay allowed. `lint/fail/shader_magic.comp` is the bad example. `lint/pass/shader_named.glsl` is the shape that stays clean.

`lint/shader-magic-baseline.txt` is the per-file count. A higher count fails. Measured 2026-10-14, after merging gi-v2: **891** literals, 26 files.

| File | Literals |
|---|---:|
| `crates/render/shaders/lighting_cache.glsl` | 338 |
| `crates/render/shaders/lighting_cache.comp` | 89 |
| `crates/render/shaders/lighting_common.glsl` | 46 |
| `crates/render/shaders/scene_rays.glsl` | 46 |
| `crates/render/shaders/lighting_direct.comp` | 42 |
| `crates/render/shaders/lighting_compact.comp` | 41 |
| `crates/render/shaders/lighting_compose.comp` | 48 |
| `crates/render/shaders/lighting_sun_compose.comp` | 47 |
| `crates/render/shaders/fxaa.comp` | 30 |
| `crates/render/shaders/lighting_gather.comp` | 28 |
| `crates/gpuref/shaders/path.comp` | 19 |
| `crates/render/shaders/mesh_field.glsl` | 18 |
| `crates/render/shaders/gbuffer.frag` | 14 |
| `crates/render/shaders/lighting_trace.comp` | 12 |
| `crates/render/shaders/lighting_filter.comp` | 11 |
| `crates/render/shaders/lighting_screen.comp` | 10 |
| `crates/render/shaders/material_atlas.glsl` | 10 |
| `crates/render/shaders/overlay.frag` | 10 |
| `crates/render/shaders/lighting_place.comp` | 8 |
| `crates/render/shaders/lighting_pack.glsl` | 6 |
| `crates/render/shaders/lighting_lamp_pick.glsl` | 5 |
| `crates/render/shaders/dither.glsl` | 4 |
| `crates/render/shaders/lighting_check.comp` | 4 |
| `crates/render/shaders/wire.frag` | 3 |
| `crates/render/shaders/lighting_atrous.comp` | 1 |
| `crates/render/shaders/transmit.comp` | 1 |

Bad shader (the check fails this):

```glsl
float shade(float x) {
    return x * 0.25 + 0.1;
}
```

### TODO: name the shader numbers

Move each literal onto a `const` or `#define` and lower that file's count. A new shader with a bare fraction fails immediately. Start outside `lighting_cache.glsl`.

## cargo-deny

`deny.toml`, added 2026-10-14. `cargo deny check` covers advisories, bans, licenses, and sources.

Banned, because ADR 0007 and ADR 0009 already rejected them: `winit`, `sdl2`, `sdl3`, `glfw`, `wgpu`, `vulkano`, `x11rb`, `cpal`, `rodio`, `tokio`, `pyo3`, `cpython`, `mlua`, `rlua`, `mlua-sys`. `ash` is banned except as a direct dependency of `genos-gpuref` (`wrappers`).

`wildcards = "deny"` rejects a `*` version requirement. `allow-wildcard-paths` leaves in-workspace `path` dependencies alone. They do not name a crates.io version. Unpublished workspace crates are ignored by the license check (`private.ignore`). `RUSTSEC-2026-0249` (unmaintained `smartstring`) is ignored because `rhai` 1.26.0, pinned by ADR 0009, is the parent. Do not bump Rhai to clear it.

Bad dependency (the ban fails this):

```toml
[dependencies]
winit = "0.30"
```

## Environment registry

`lint/env_mirrors.toml`, checked by `scripts/lint-env-mirrors.py`. Added 2026-10-14.

A `GENOS_*` or `OCC_*` string in Rust must be listed. Kinds:

| Kind | Count | Meaning |
|---|---|---|
| `mirror` | 93 | A CLI argument exists and wins when both are set. `detail` is the flag. |
| `todo` | 82 | Read today, and still has no argument. |
| `override` | 4 | `GENOS_MCP_PORT` (discovery directory is primary), `GENOS_ENGINE_COMMIT` (stamp when git is missing), `GENOS_DEBUG_KEYS` (a file replaces built-in chords), and `GENOS_MENU_LAYOUT` (a file replaces `debug/lighting/menu.txt`). |
| `test` | 14 | Harness only. |
| `os` | 5 | `HOME`, `XDG_RUNTIME_DIR`, `XDG_STATE_HOME`, `WAYLAND_DISPLAY`, `VK_ICD_FILENAMES`. |
| `build` | 3 | `OUT_DIR`, `CARGO_CFG_TARGET_OS`, and `GENOS_HDR_COMPOSE` (the HDR compose shader compile). |

Do not add a new `kind = "todo"`. Add a `mirror` whose `detail` is the flag, or do not read the variable.

Bad example (`lint/fail/env_switch.rs`, the checker rejects this name):

```rust
let _ = std::env::var("GENOS_NOT_A_MIRROR");
```

### TODO: env switches without an argument

The 82 `kind = "todo"` rows are the list. The cluster called out in `docs/lighting/RENAMES.md` item 14 is still env-only: `GENOS_LIGHTING_URGENT`, `GENOS_LIGHTING_BORROW`, `GENOS_LIGHTING_MOVED_DROP`. Young-only, cache picks, cache Hz, surface cache settle and world cap are now `LightingSettings.experiments` keys. Promote the live ones onto the settings value, the debug menu, and MCP, or delete them.

## Semgrep

`lint/semgrep/genos.yml`. `scripts/lint-semgrep.py` scans the repo (skipping `lint/fail`) and requires each rule to match its file in `lint/fail`. `lint/pass/output.rs` is the shape that should stay clean.

| Rule | Added | Why | Bad example | Backlog |
|---|---|---|---|---|
| `genos-no-home-path` | 2026-10-14 | Debug output stays in the working directory. Do not hard-code `/home`, `$HOME`, or `~/`, and do not read `HOME` to choose an output path. | `lint/fail/home_path.rs` | 7 sites. `crates/ui/src/omarchy.rs` reads `HOME` as the XDG fallback for the theme path and does not write there. `crates/scene/src/debug_settings.rs` uses a home-shaped string to prove `--out` wins. |
| `genos-debug-output-dir` | 2026-10-14 | Screenshots, dumps, and logs default to `./genos-debug/<kind>/`. `--out` overrides that. | `lint/fail/debug_output.rs` (`"lighting-dump"`) | 3 sites: `crates/render/src/lighting_gpu.rs` (`lighting-dump`), `crates/debug/src/tools.rs` (`target/reference-cache`), `crates/debug/src/reference.rs` (`target/reference-known`). |
| `genos-gpu-len-cast` | 2026-10-14 | `len() as u32` and `len() as u64` truncate a buffer length. Use `TryFrom`. This is the buffer case of `as_conversions`. | `lint/fail/gpu_len_cast.rs` | 82 sites in the 27 files on the rule's `paths.exclude`. New files are checked. |
| `genos-no-gi-v1-term` | 2026-10-14 | GI v1 names stay in the glossary Historical section. Do not put radiance cascade, radiance field, lightmap, light probe, product "GI v2", `archive-v1.md`, ADR 0008, or `gi_v2_floor` back into Rust or shader code. | `lint/fail/gi_v1_term.rs` | 0. Extended 2026-10-14 for leftover product labels and deleted paths. |
| `genos-no-hardcoded-display` | 2026-10-14 | HDMI-A-1 is the proof display and DP-1 is the main display. Only `--proof` selects a proof window, and it must not hard-code either connector. | `lint/fail/hardcoded_display.rs` | 0. |
| `genos-no-hyprctl` | 2026-10-14 | A proof run does not resize or focus unrelated windows. Shipping code does not spawn `hyprctl`. | `lint/fail/hyprctl.rs` | 6 call sites, all in the pointer-lock test in `crates/window/src/lib.rs`. |
| `genos-no-process-exit` | 2026-10-26 | The process leaves through `genos_window::exit`, after the app lifecycle has destroyed the device. That function flushes and calls `_exit`. `std::process::exit` skips the lifecycle and runs the driver atexit handler. | `lint/fail/process_exit.rs` | 0. The one exit is `crates/window/src/lifecycle.rs`. |
| `genos-no-device-teardown` | 2026-10-26 | Fence drain and device destruction stay on the app lifecycle. Do not call `vkDeviceWaitIdle` or abandon the device from a binary. A `drop` of the renderer is refused by `ShutdownPermit` and by `Gpu`'s `Drop`, which panics instead of waiting idle. | `lint/fail/device_teardown.rs` | The Vulkan load and the idle path stay in `crates/render/src/gpu.rs` and `crates/render/src/lighting_gpu.rs`. `crates/gpuref` is the offline reference. |
| `genos-stream-no-wait` | 2026-10-26 | Streaming functions poll the transfer fence, including helpers such as `transfer_signaled`. They do not call `wait_for_fences`, `queue_wait_idle`, `vkWaitForFences`, `vkQueueWaitIdle`, `wait_fences`, `idle_device`, or `device_wait`. | `lint/fail/stream_wait.rs` | 0. |

### TODO: semgrep excludes

The excludes are the backlog. Delete a path from `paths.exclude` when that file is clean.

- Point `lighting-dump`, `target/reference-cache`, and `target/reference-known` at `./genos-debug/<kind>/` (or `--out`) and drop those three excludes.
- Replace `len() as u32` / `len() as u64` in the 27 excluded files, then drop the excludes. Start outside `crates/render/src/gpu.rs` so the lighting files stay stable.
- Move the `hyprctl` pointer-lock harness out of `crates/window/src/lib.rs`, then drop that exclude.
- `omarchy.rs` may keep reading `HOME` for the theme path. `debug_settings.rs` may keep the home-shaped fixture. Those two excludes stay until the reads move behind a named helper the rule can ignore.

## What this does not lint

GI v1 prose in `docs/` and in the Historical section of `docs/lighting/GLOSSARY.md` is the record. The semgrep rule covers Rust and shaders only. Pending renames in `docs/lighting/RENAMES.md` are not banned; the names are still the ones in the tree.
