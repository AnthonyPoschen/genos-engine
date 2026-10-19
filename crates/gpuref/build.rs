#![expect(clippy::expect_used, reason = "the build stops when a host tool or OUT_DIR is missing")]
#![expect(clippy::manual_assert, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![allow(
    clippy::disallowed_methods,
    reason = "the build reads OUT_DIR from Cargo"
)]
#![expect(clippy::panic, reason = "the build stops when a host tool or OUT_DIR is missing")]

use std::process::Command;

fn main() {
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let src = "shaders/path.comp";
    println!("cargo:rerun-if-changed={src}");
    let status = Command::new("glslc")
        .args(["--target-env=vulkan1.2", "-O", src, "-o"])
        .arg(format!("{out}/path.spv"))
        .status()
        .expect("glslc");
    if !status.success() {
        panic!("glslc failed for {src}");
    }
}
