use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let vert = compile("shaders/scene.vert", &out.join("scene.vert.spv"));
    let frag = compile("shaders/overlay.frag", &out.join("overlay.frag.spv"));
    let wire_vert = compile("shaders/wire.vert", &out.join("wire.vert.spv"));
    let wire = compile("shaders/wire.frag", &out.join("wire.frag.spv"));
    let audio = compile("shaders/transmit.comp", &out.join("transmit.comp.spv"));
    let fxaa = compile("shaders/fxaa.comp", &out.join("fxaa.comp.spv"));
    // Not embedded: catches errors in GI v2 shader pieces no pass includes yet.
    compile(
        "shaders/lighting_check.comp",
        &out.join("lighting_check.comp.spv"),
    );
    let atrous_tile = compile(
        "shaders/lighting_atrous_tile.comp",
        &out.join("lighting_atrous_tile.comp.spv"),
    );
    // High's a-trous. Half, adaptive, and tiles are compiled out (S79).
    let atrous_plain = compile_define(
        "shaders/lighting_atrous.comp",
        &out.join("lighting_atrous_plain.comp.spv"),
        "LIGHTING_DENOISE_PLAIN",
    );
    println!("cargo:rerun-if-changed=shaders/lighting_atrous_tile.comp");
    println!("cargo:rerun-if-changed=shaders/lighting_atrous_tile_unroll.glsl");
    let lighting: Vec<(&str, PathBuf)> = [
        ("GBUFFER_VERT_SPV", "gbuffer.vert"),
        ("GBUFFER_FRAG_SPV", "gbuffer.frag"),
        ("LIGHTING_COMPACT_SPV", "lighting_compact.comp"),
        ("LIGHTING_PLACE_SPV", "lighting_place.comp"),
        ("LIGHTING_TRACE_SPV", "lighting_trace.comp"),
        ("LIGHTING_DIRECT_SPV", "lighting_direct.comp"),
        ("LIGHTING_GATHER_SPV", "lighting_gather.comp"),
        ("LIGHTING_FILTER_SPV", "lighting_filter.comp"),
        ("LIGHTING_CACHE_SPV", "lighting_cache.comp"),
        ("LIGHTING_COMPOSE_SPV", "lighting_compose.comp"),
        ("LIGHTING_ATROUS_SPV", "lighting_atrous.comp"),
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
            "pub static VERT_SPV: &[u8] = include_bytes!(\"{}\");\npub static FRAG_SPV: &[u8] = include_bytes!(\"{}\");\npub static WIRE_VERT_SPV: &[u8] = include_bytes!(\"{}\");\npub static WIRE_FRAG_SPV: &[u8] = include_bytes!(\"{}\");\npub static AUDIO_SPV: &[u8] = include_bytes!(\"{}\");\npub static FXAA_SPV: &[u8] = include_bytes!(\"{}\");\n",
            vert.display(),
            frag.display(),
            wire_vert.display(),
            wire.display(),
            audio.display(),
            fxaa.display()
        ),
    )
    .unwrap();
    let mut glue_text = std::fs::read_to_string(&glue).unwrap();
    for (name, path) in &lighting {
        glue_text.push_str(&format!(
            "pub static {name}: &[u8] = include_bytes!(\"{}\");\n",
            path.display()
        ));
    }
    glue_text.push_str(&format!(
        "pub static LIGHTING_ATROUS_TILE_SPV: &[u8] = include_bytes!(\"{}\");\n",
        atrous_tile.display()
    ));
    glue_text.push_str(&format!(
        "pub static LIGHTING_ATROUS_PLAIN_SPV: &[u8] = include_bytes!(\"{}\");\n",
        atrous_plain.display()
    ));
    std::fs::write(&glue, glue_text).unwrap();
    for piece in [
        "lighting_common.glsl",
        "lighting_pack.glsl",
        "lighting_cache.glsl",
        "lighting_cache_slots.glsl",
    ] {
        println!("cargo:rerun-if-changed=shaders/{piece}");
    }
    println!("cargo:rerun-if-changed=shaders/scene.vert");
    println!("cargo:rerun-if-changed=shaders/overlay.frag");
    println!("cargo:rerun-if-changed=shaders/wire.vert");
    println!("cargo:rerun-if-changed=shaders/wire.frag");
    println!("cargo:rerun-if-changed=shaders/scene_rays.glsl");
    println!("cargo:rerun-if-changed=shaders/mesh_field.glsl");
    println!("cargo:rerun-if-changed=shaders/lighting_check.comp");
    println!("cargo:rerun-if-changed=shaders/scene_data.glsl");
    println!("cargo:rerun-if-changed=shaders/dither.glsl");
    println!("cargo:rerun-if-changed=shaders/transmit.comp");
    println!("cargo:rerun-if-changed=shaders/fxaa.comp");
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

/// glslc's wrapper does not forward `-D`. The plain a-trous kernel needs the macro.
fn compile_define(src: &str, dest: &std::path::Path, define: &str) -> PathBuf {
    let status = Command::new("glslangValidator")
        .args([
            "-V",
            "--target-env",
            "vulkan1.3",
            &format!("-D{define}"),
            src,
            "-o",
            dest.to_str().unwrap(),
        ])
        .status()
        .expect("glslangValidator");
    if !status.success() {
        panic!("glslangValidator failed for {src} -D{define}");
    }
    dest.to_path_buf()
}
