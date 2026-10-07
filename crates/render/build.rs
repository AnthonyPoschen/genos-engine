use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let vert = compile("shaders/scene.vert", &out.join("scene.vert.spv"));
    let frag = compile("shaders/scene.frag", &out.join("scene.frag.spv"));
    let wire_vert = compile("shaders/wire.vert", &out.join("wire.vert.spv"));
    let wire = compile("shaders/wire.frag", &out.join("wire.frag.spv"));
    let comp = compile("shaders/light.comp", &out.join("light.comp.spv"));
    let audio = compile("shaders/transmit.comp", &out.join("transmit.comp.spv"));
    let fxaa = compile("shaders/fxaa.comp", &out.join("fxaa.comp.spv"));
    let ssaa = compile("shaders/ssaa.comp", &out.join("ssaa.comp.spv"));
    let glue = out.join("shaders.rs");
    std::fs::write(
        &glue,
        format!(
            "pub static VERT_SPV: &[u8] = include_bytes!(\"{}\");\npub static FRAG_SPV: &[u8] = include_bytes!(\"{}\");\npub static WIRE_VERT_SPV: &[u8] = include_bytes!(\"{}\");\npub static WIRE_FRAG_SPV: &[u8] = include_bytes!(\"{}\");\npub static COMP_SPV: &[u8] = include_bytes!(\"{}\");\npub static AUDIO_SPV: &[u8] = include_bytes!(\"{}\");\npub static FXAA_SPV: &[u8] = include_bytes!(\"{}\");\npub static SSAA_SPV: &[u8] = include_bytes!(\"{}\");\n",
            vert.display(),
            frag.display(),
            wire_vert.display(),
            wire.display(),
            comp.display(),
            audio.display(),
            fxaa.display(),
            ssaa.display()
        ),
    )
    .unwrap();
    println!("cargo:rerun-if-changed=shaders/scene.vert");
    println!("cargo:rerun-if-changed=shaders/scene.frag");
    println!("cargo:rerun-if-changed=shaders/wire.vert");
    println!("cargo:rerun-if-changed=shaders/wire.frag");
    println!("cargo:rerun-if-changed=shaders/light.comp");
    println!("cargo:rerun-if-changed=shaders/scene_rays.glsl");
    println!("cargo:rerun-if-changed=shaders/scene_data.glsl");
    println!("cargo:rerun-if-changed=shaders/tier.glsl");
    println!("cargo:rerun-if-changed=shaders/dither.glsl");
    println!("cargo:rerun-if-changed=shaders/transmit.comp");
    println!("cargo:rerun-if-changed=shaders/fxaa.comp");
    println!("cargo:rerun-if-changed=shaders/ssaa.comp");
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
