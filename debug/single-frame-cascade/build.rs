#![expect(clippy::expect_used, reason = "the build stops when a host tool or OUT_DIR is missing")]
#![expect(clippy::panic, reason = "the build stops when a host tool or OUT_DIR is missing")]
#![allow(clippy::disallowed_methods, reason = "the build reads OUT_DIR from Cargo")]

use std::process::Command;

fn main() {
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let src = "shaders/cone.comp";
    println!("cargo:rerun-if-changed={src}");
    let status = Command::new("glslc")
        .args(["--target-env=vulkan1.2", "-O", src, "-o"])
        .arg(format!("{out}/cone.spv"))
        .status()
        .expect("glslc");
    if !status.success() {
        panic!("glslc failed for {src}");
    }
}
