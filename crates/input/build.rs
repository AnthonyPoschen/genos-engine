#![expect(clippy::manual_assert, reason = "a failed host tool stops the build")]
#![allow(
    clippy::disallowed_methods,
    reason = "the build reads OUT_DIR from Cargo"
)]
#![expect(clippy::panic, reason = "the build stops when a host tool or OUT_DIR is missing")]
#![expect(clippy::unwrap_used, reason = "the build stops when a host tool or OUT_DIR is missing")]

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/gamecontroller.m");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os != "macos" {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let obj = out.join("gamecontroller.o");
    let archive = out.join("libgenos_gamecontroller.a");
    let cc = std::env::var("CC").unwrap_or_else(|_| "clang".to_string());
    let archiver = std::env::var("AR").unwrap_or_else(|_| "ar".to_string());
    let obj_path = obj.display().to_string();
    let archive_path = archive.display().to_string();
    run(
        &cc,
        &[
            "-c",
            "-fno-objc-arc",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-x",
            "objective-c",
            "src/gamecontroller.m",
            "-o",
            &obj_path,
        ],
    );
    run(&archiver, &["crs", &archive_path, &obj_path]);
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=genos_gamecontroller");
    println!("cargo:rustc-link-lib=framework=GameController");
    println!("cargo:rustc-link-lib=framework=Foundation");
}

fn run(program: &str, args: &[&str]) {
    let status = Command::new(program)
        .args(args)
        .status()
        .unwrap_or_else(|err| panic!("{program} failed to start: {err}"));
    if !status.success() {
        panic!("{program} {args:?} failed with {status}");
    }
}
