#![expect(
    clippy::manual_assert,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::panic,
    reason = "the build stops when a host tool or OUT_DIR is missing"
)]
#![allow(
    clippy::disallowed_methods,
    reason = "the build reads OUT_DIR from Cargo"
)]
#![expect(
    clippy::unwrap_used,
    reason = "the build stops when a host tool or OUT_DIR is missing"
)]

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "windows" {
        build_windows();
        return;
    }
    build_wayland();
}

fn build_windows() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if env == "msvc" {
        compile_msvc(&out);
    } else {
        compile_gnu(&out);
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=genos_win32");
    println!("cargo:rustc-link-lib=user32");
    println!("cargo:rustc-link-lib=gdi32");
    println!("cargo:rerun-if-changed=src/win32_shim.c");
    println!("cargo:rerun-if-changed=src/genos_pump.h");
}

fn compile_msvc(out: &std::path::Path) {
    let obj = out.join("win32_shim.obj");
    let lib = out.join("genos_win32.lib");
    let cc = std::env::var("CC").unwrap_or_else(|_| "cl".to_string());
    let obj_arg = format!("/Fo{}", obj.display());
    run(&cc, &["/nologo", "/c", "/W3", &obj_arg, "src/win32_shim.c"]);
    let archiver = std::env::var("AR").unwrap_or_else(|_| "lib".to_string());
    let lib_arg = format!("/OUT:{}", lib.display());
    let obj_path = obj.display().to_string();
    run(&archiver, &["/nologo", &lib_arg, &obj_path]);
}

fn compile_gnu(out: &std::path::Path) {
    let obj = out.join("win32_shim.o");
    let archive = out.join("libgenos_win32.a");
    let cc = gnu_tool("gcc");
    let archiver = gnu_tool("ar");
    let obj_path = obj.display().to_string();
    let archive_path = archive.display().to_string();
    run(
        &cc,
        &[
            "-c",
            "-Wall",
            "-Wextra",
            "-Werror",
            "src/win32_shim.c",
            "-o",
            &obj_path,
        ],
    );
    run(&archiver, &["crs", &archive_path, &obj_path]);
}

fn gnu_tool(name: &str) -> String {
    let from_env = if name == "ar" {
        std::env::var("AR").ok()
    } else {
        std::env::var("CC").ok()
    };
    if let Some(tool) = from_env {
        return tool;
    }
    let host = std::env::var("HOST").unwrap_or_default();
    if host.contains("windows") {
        return name.to_string();
    }
    let target = std::env::var("TARGET").unwrap_or_default();
    let prefix = if target.starts_with("i686") {
        "i686-w64-mingw32"
    } else {
        "x86_64-w64-mingw32"
    };
    let candidate = format!("{prefix}-{name}");
    if name == "ar" && Command::new(&candidate).arg("--version").output().is_err() {
        return format!("{prefix}-gcc-ar");
    }
    candidate
}

fn build_wayland() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let protocols = [
        "/usr/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml",
        "/usr/share/wayland-protocols/unstable/pointer-constraints/pointer-constraints-unstable-v1.xml",
        "/usr/share/wayland-protocols/unstable/relative-pointer/relative-pointer-unstable-v1.xml",
    ];
    let mut sources = Vec::new();
    for xml in protocols {
        sources.push(scan(xml, &out));
        println!("cargo:rerun-if-changed={xml}");
    }

    let include = out.to_str().unwrap();
    let shim_obj = out.join("wayland_shim.o");
    run(
        "cc",
        &[
            "-c",
            "-fPIC",
            "-I",
            include,
            "src/wayland_shim.c",
            "-o",
            shim_obj.to_str().unwrap(),
        ],
    );

    let mut compiled = vec![shim_obj];
    for source in &sources {
        let obj = source.with_extension("o");
        run(
            "cc",
            &[
                "-c",
                "-fPIC",
                "-I",
                include,
                source.to_str().unwrap(),
                "-o",
                obj.to_str().unwrap(),
            ],
        );
        compiled.push(obj);
    }

    let archive = out.join("libgenos_wayland.a");
    let mut args = vec!["crs".to_string(), archive.display().to_string()];
    args.extend(compiled.iter().map(|path| path.display().to_string()));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run("ar", &arg_refs);
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=genos_wayland");
    println!("cargo:rustc-link-lib=wayland-client");
    println!("cargo:rustc-link-lib=xkbcommon");
    println!("cargo:rerun-if-changed=src/wayland_shim.c");
    println!("cargo:rerun-if-changed=src/genos_pump.h");
}

fn scan(xml: &str, out: &std::path::Path) -> PathBuf {
    let stem = std::path::Path::new(xml)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap();
    let header = out.join(format!("{stem}-client-protocol.h"));
    let protocol = out.join(format!("{stem}-protocol.c"));
    run(
        "wayland-scanner",
        &["client-header", xml, header.to_str().unwrap()],
    );
    run(
        "wayland-scanner",
        &["private-code", xml, protocol.to_str().unwrap()],
    );
    protocol
}

fn run(cmd: &str, args: &[&str]) {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .unwrap_or_else(|err| panic!("failed to start {cmd}: {err}"));
    if !status.success() {
        panic!("{cmd} {args:?} failed with {status}");
    }
}
