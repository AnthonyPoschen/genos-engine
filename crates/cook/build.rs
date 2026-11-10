#![allow(
    clippy::disallowed_methods,
    reason = "the build reads the target OS from Cargo"
)]

//! `intel_tex_2` ships a prebuilt C++ object (its ASTC entry points) that needs
//! the C++ runtime for exception personality, and it does not ask for one. Link it.

fn main() {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        // The runtime soname, so no C++ dev package is needed to build.
        Ok("linux") => println!("cargo:rustc-link-lib=dylib:+verbatim=libstdc++.so.6"),
        Ok("macos" | "ios") => println!("cargo:rustc-link-lib=c++"),
        _ => {}
    }
}
