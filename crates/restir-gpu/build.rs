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
    compile(&out, "shaders/restir_initial.comp", "restir_initial.spv");
    compile(&out, "shaders/restir_temporal.comp", "restir_temporal.spv");
    compile(&out, "shaders/restir_spatial.comp", "restir_spatial.spv");
}

fn compile(out: &str, src: &str, spv_name: &str) {
    println!("cargo:rerun-if-changed={src}");
    let status = Command::new("glslc")
        .args(["--target-env=vulkan1.0", "-O", src, "-o"])
        .arg(format!("{out}/{spv_name}"))
        .status()
        .expect("glslc");
    if !status.success() {
        panic!("glslc failed for {src}");
    }
}
