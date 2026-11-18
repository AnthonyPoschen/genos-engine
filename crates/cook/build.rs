#![allow(
    clippy::disallowed_methods,
    reason = "the build reads the target OS from Cargo"
)]

//! `intel_tex_2` ships a prebuilt C++ object (its ASTC entry points) that needs
//! the C++ runtime for exception personality, and it does not ask for one. Link it.

fn main() {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        // The runtime soname, so no C++ dev package is needed to build.
        // rustc links with lld, which does not search gcc's C++ directory, so
        // the meshoptimizer objects (`-lstdc++`) need that directory too.
        Ok("linux") => {
            println!("cargo:rustc-link-lib=dylib:+verbatim=libstdc++.so.6");
            if let Some(dir) = stdcpp_dir() {
                println!("cargo:rustc-link-search=native={dir}");
            }
        }
        Ok("macos" | "ios") => println!("cargo:rustc-link-lib=c++"),
        _ => {}
    }
}

/// Directory of `libstdc++.so`, from `g++ -print-file-name`. Empty when g++ is absent.
fn stdcpp_dir() -> Option<String> {
    let output = std::process::Command::new("g++")
        .arg("-print-file-name=libstdc++.so")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = std::path::Path::new(path.trim());
    path.parent()
        .filter(|dir| dir.join("libstdc++.so").is_file() || dir.join("libstdc++.a").is_file())
        .map(|dir| dir.display().to_string())
}
