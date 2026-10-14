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
| `ash::Entry`, `ash::Instance`, `ash::Device` | 2026-10-14 | ADR 0007. The renderer loads `libvulkan` itself. | `fn open() -> ash::Device { unimplemented!() }` |

`lint/fail/` is not a workspace member, so these examples are not compiled with the workspace. The method and type bans are the `disallowed_*` rows in the table above, and the 24 `set_var` sites are the backlog.

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
| `mirror` | 23 | A CLI argument exists and wins when both are set. `detail` is the flag. |
| `todo` | 95 | Read today, and still has no argument. |
| `override` | 2 | `GENOS_MCP_PORT` (discovery directory is primary) and `GENOS_ENGINE_COMMIT` (stamp when git is missing). |
| `test` | 14 | Harness only. |
| `os` | 5 | `HOME`, `XDG_RUNTIME_DIR`, `XDG_STATE_HOME`, `WAYLAND_DISPLAY`, `VK_ICD_FILENAMES`. |
| `build` | 2 | `OUT_DIR`, `CARGO_CFG_TARGET_OS`. |

Do not add a new `kind = "todo"`. Add a `mirror` whose `detail` is the flag, or do not read the variable.

Bad example (`lint/fail/env_switch.rs`, the checker rejects this name):

```rust
let _ = std::env::var("GENOS_NOT_A_MIRROR");
```

### TODO: env switches without an argument

The 95 `kind = "todo"` rows are the list. The cluster called out in `docs/lighting/RENAMES.md` item 14 is still env-only: `GENOS_LIGHTING_URGENT`, `GENOS_LIGHTING_BORROW`, `GENOS_LIGHTING_MOVED_DROP`, `GENOS_LIGHTING_YOUNG_ONLY`, `GENOS_LIGHTING_CACHE_PICKS`, `GENOS_LIGHTING_CACHE_HZ`, `GENOS_LIGHTING_SC_SETTLE`, `GENOS_LIGHTING_WORLD_CAP`. Promote the live ones onto the settings value, the debug menu, and MCP, or delete them.

## Semgrep

`lint/semgrep/genos.yml`. `scripts/lint-semgrep.py` scans the repo (skipping `lint/fail`) and requires each rule to match its file in `lint/fail`. `lint/pass/output.rs` is the shape that should stay clean.

| Rule | Added | Why | Bad example | Backlog |
|---|---|---|---|---|
| `genos-no-home-path` | 2026-10-14 | Debug output stays in the working directory. Do not hard-code `/home`, `$HOME`, or `~/`, and do not read `HOME` to choose an output path. | `lint/fail/home_path.rs` | 7 sites. `crates/ui/src/omarchy.rs` reads `HOME` as the XDG fallback for the theme path and does not write there. `crates/scene/src/debug_settings.rs` uses a home-shaped string to prove `--out` wins. |
| `genos-debug-output-dir` | 2026-10-14 | Screenshots, dumps, and logs default to `./genos-debug/<kind>/`. `--out` overrides that. | `lint/fail/debug_output.rs` (`"lighting-dump"`) | 3 sites: `crates/render/src/lighting_gpu.rs` (`lighting-dump`), `crates/debug/src/tools.rs` (`target/reference-cache`), `crates/debug/src/reference.rs` (`target/reference-known`). |
| `genos-gpu-len-cast` | 2026-10-14 | `len() as u32` and `len() as u64` truncate a buffer length. Use `TryFrom`. This is the buffer case of `as_conversions`. | `lint/fail/gpu_len_cast.rs` | 82 sites in the 27 files on the rule's `paths.exclude`. New files are checked. |
| `genos-no-gi-v1-term` | 2026-10-14 | GI v1 names stay in the glossary Historical section. Do not put radiance cascade, radiance field, lightmap, or light probe back into Rust or shader code. | `lint/fail/gi_v1_term.rs` | 0. Two comments that still used the old names were reworded (`examples/camera/src/main.rs`, `crates/render/src/lighting_gpu.rs`). |
| `genos-no-hardcoded-display` | 2026-10-14 | HDMI-A-1 is the proof display and DP-1 is the main display. Only `--proof` selects a proof window, and it must not hard-code either connector. | `lint/fail/hardcoded_display.rs` | 0. |
| `genos-no-hyprctl` | 2026-10-14 | A proof run does not resize or focus unrelated windows. Shipping code does not spawn `hyprctl`. | `lint/fail/hyprctl.rs` | 6 call sites, all in the pointer-lock test in `crates/window/src/lib.rs`. |

### TODO: semgrep excludes

The excludes are the backlog. Delete a path from `paths.exclude` when that file is clean.

- Point `lighting-dump`, `target/reference-cache`, and `target/reference-known` at `./genos-debug/<kind>/` (or `--out`) and drop those three excludes.
- Replace `len() as u32` / `len() as u64` in the 27 excluded files, then drop the excludes. Start outside `crates/render/src/gpu.rs` so the lighting files stay stable.
- Move the `hyprctl` pointer-lock harness out of `crates/window/src/lib.rs`, then drop that exclude.
- `omarchy.rs` may keep reading `HOME` for the theme path. `debug_settings.rs` may keep the home-shaped fixture. Those two excludes stay until the reads move behind a named helper the rule can ignore.

## What this does not lint

GI v1 prose in `docs/` and in the Historical section of `docs/lighting/GLOSSARY.md` is the record. The semgrep rule covers Rust and shaders only. Pending renames in `docs/lighting/RENAMES.md` are not banned; the names are still the ones in the tree.
