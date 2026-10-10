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
