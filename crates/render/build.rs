use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let vert = compile("shaders/scene.vert", &out.join("scene.vert.spv"));
    let frag = compile("shaders/scene.frag", &out.join("scene.frag.spv"));
    let glue = out.join("shaders.rs");
    std::fs::write(
        &glue,
        format!(
            "pub static VERT_SPV: &[u8] = include_bytes!(\"{}\");\npub static FRAG_SPV: &[u8] = include_bytes!(\"{}\");\n",
            vert.display(),
            frag.display()
        ),
    )
    .unwrap();
    println!("cargo:rerun-if-changed=shaders/scene.vert");
    println!("cargo:rerun-if-changed=shaders/scene.frag");
}

fn compile(src: &str, dest: &std::path::Path) -> PathBuf {
    let status = Command::new("glslc")
        .args([src, "-o", dest.to_str().unwrap()])
        .status()
        .expect("glslc");
    if !status.success() {
        panic!("glslc failed for {src}");
    }
    dest.to_path_buf()
}
