#![allow(
    clippy::disallowed_methods,
    reason = "the build reads OUT_DIR from Cargo"
)]
#![expect(
    clippy::expect_used,
    reason = "the build stops when a host tool or OUT_DIR is missing"
)]
#![expect(
    clippy::panic,
    reason = "the build stops when glslc rejects the initial-candidate shader"
)]
#![allow(
    clippy::manual_assert,
    reason = "a failed glslc stops the build with the shader path"
)]

use std::process::Command;

fn main() {
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let src = "shaders/restir_initial.comp";
    println!("cargo:rerun-if-changed={src}");
    let status = Command::new("glslc")
        .args(["--target-env=vulkan1.0", "-O", src, "-o"])
        .arg(format!("{out}/restir_initial.spv"))
        .status()
        .expect("glslc");
    if !status.success() {
        panic!("glslc failed for {src}");
    }
}
