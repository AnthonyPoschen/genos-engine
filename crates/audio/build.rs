#![allow(
    clippy::disallowed_methods,
    reason = "the build reads the target triple from Cargo"
)]

fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "linux" {
        println!("cargo:rustc-link-lib=asound");
    }
    if os == "windows" {
        println!("cargo:rustc-link-lib=ole32");
    }
    if os == "macos" {
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
    }
}
