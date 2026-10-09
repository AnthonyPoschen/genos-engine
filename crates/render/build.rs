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
    // Not embedded: catches errors in GI v2 shader pieces no pass includes yet.
    compile("shaders/gi2_check.comp", &out.join("gi2_check.comp.spv"));
    let gi2: Vec<(&str, PathBuf)> = [
        ("GBUFFER_FRAG_SPV", "gbuffer.frag"),
        ("GI2_PLACE_SPV", "gi2_place.comp"),
        ("GI2_TRACE_SPV", "gi2_trace.comp"),
        ("GI2_LIGHT_SPV", "gi2_light.comp"),
        ("GI2_GATHER_SPV", "gi2_gather.comp"),
        ("GI2_COMPOSE_SPV", "gi2_compose.comp"),
    ]
    .iter()
    .map(|(name, file)| {
        let src = format!("shaders/{file}");
        println!("cargo:rerun-if-changed={src}");
        (*name, compile(&src, &out.join(format!("{file}.spv"))))
    })
    .collect();
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
    let mut glue_text = std::fs::read_to_string(&glue).unwrap();
    for (name, path) in &gi2 {
        glue_text.push_str(&format!(
            "pub static {name}: &[u8] = include_bytes!(\"{}\");\n",
            path.display()
        ));
    }
    std::fs::write(&glue, glue_text).unwrap();
    for piece in ["gi2_common.glsl", "gi2_pack.glsl"] {
        println!("cargo:rerun-if-changed=shaders/{piece}");
    }
    println!("cargo:rerun-if-changed=shaders/scene.vert");
    println!("cargo:rerun-if-changed=shaders/scene.frag");
    println!("cargo:rerun-if-changed=shaders/wire.vert");
    println!("cargo:rerun-if-changed=shaders/wire.frag");
    println!("cargo:rerun-if-changed=shaders/light.comp");
    println!("cargo:rerun-if-changed=shaders/scene_rays.glsl");
    println!("cargo:rerun-if-changed=shaders/mesh_field.glsl");
    println!("cargo:rerun-if-changed=shaders/gi2_check.comp");
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
