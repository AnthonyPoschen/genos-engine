use std::path::PathBuf;
use std::process::Command;

fn main() {
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
}

fn scan(xml: &str, out: &std::path::Path) -> PathBuf {
    let stem = std::path::Path::new(xml)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap();
    let header = out.join(format!("{stem}-client-protocol.h"));
    let protocol = out.join(format!("{stem}-protocol.c"));
    run("wayland-scanner", &["client-header", xml, header.to_str().unwrap()]);
    run("wayland-scanner", &["private-code", xml, protocol.to_str().unwrap()]);
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
