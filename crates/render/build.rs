#![expect(clippy::expect_used, reason = "the build stops when a host tool or OUT_DIR is missing")]
#![expect(clippy::format_push_string, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_assert, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::panic, reason = "the build stops when a host tool or OUT_DIR is missing")]
#![allow(
    clippy::disallowed_methods,
    reason = "the build reads OUT_DIR from Cargo"
)]
#![expect(clippy::unwrap_used, reason = "the build stops when a host tool or OUT_DIR is missing")]

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let vert = compile("shaders/scene.vert", &out.join("scene.vert.spv"));
    let frag = compile("shaders/overlay.frag", &out.join("overlay.frag.spv"));
    let depth_mask = compile("shaders/depth_mask.frag", &out.join("depth_mask.frag.spv"));
    let wire_vert = compile("shaders/wire.vert", &out.join("wire.vert.spv"));
    let wire = compile("shaders/wire.frag", &out.join("wire.frag.spv"));
    let audio = compile("shaders/transmit.comp", &out.join("transmit.comp.spv"));
    let fxaa = compile("shaders/fxaa.comp", &out.join("fxaa.comp.spv"));
    // Not embedded: catches errors in pieces no pass includes yet.
    for file in ["lighting_check.comp", "media_check.comp"] {
        let src = format!("shaders/{file}");
        println!("cargo:rerun-if-changed={src}");
        compile(&src, &out.join(format!("{file}.spv")));
    }
    // High's a-trous. Adaptive is compiled out of the hash pipeline (S79).
    // The screen launch calls that kernel with the adaptive bit clear: high
    // does not set it, and the branch is the same test the plain pipeline
    // compiles out. glslc does not take -D. glslangValidator --target-env
    // vulkan1.3 emits SPIR-V 1.6. The instance is Vulkan 1.0 (API_VERSION in
    // gpu.rs). A 1.6 module is illegal there: validation reports
    // VUID-VkShaderModuleCreateInfo-pCode-08737, and the NVIDIA compiler logs
    // "NVVM compilation failed: 3" (IR version mismatch) and can smash the
    // process heap, which glibc reports at exit as corrupted size vs. prev_size.
    // vulkan1.0 keeps this kernel on SPIR-V 1.0, the same version as glslc.
    let screen = compile(
        "shaders/lighting_screen.comp",
        &out.join("lighting_screen.comp.spv"),
    );
    let atrous_plain = compile_define(
        "shaders/lighting_atrous.comp",
        &out.join("lighting_atrous_plain.comp.spv"),
        "LIGHTING_DENOISE_PLAIN",
    );
    // glslc forwards -D. The plain direct module is the pre-reservoir shade:
    // the reservoir block is not in the SPIR-V, so a quiet few-lamp view holds
    // at the same cost as the shade before ReSTIR. glslangValidator is not used
    // here; that path emitted a newer SPIR-V for another kernel and faulted.
    let direct_plain = compile_glslc_define(
        "shaders/lighting_direct.comp",
        &out.join("lighting_direct_plain.comp.spv"),
        "LIGHTING_DIRECT_PLAIN",
    );
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
        // Compiled into the binary. The pipeline is created the first time a
        // small sun step asks for it, not at startup, so the nine lighting
        // pipelines and their compile stay the lighting set.
        ("SUN_COMPOSE_SPV", "lighting_sun_compose.comp"),
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
            "pub static VERT_SPV: &[u8] = include_bytes!(\"{}\");\npub static FRAG_SPV: &[u8] = include_bytes!(\"{}\");\npub static DEPTH_MASK_SPV: &[u8] = include_bytes!(\"{}\");\npub static WIRE_VERT_SPV: &[u8] = include_bytes!(\"{}\");\npub static WIRE_FRAG_SPV: &[u8] = include_bytes!(\"{}\");\npub static AUDIO_SPV: &[u8] = include_bytes!(\"{}\");\npub static FXAA_SPV: &[u8] = include_bytes!(\"{}\");\n",
            vert.display(),
            frag.display(),
            depth_mask.display(),
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
        "pub static LIGHTING_ATROUS_PLAIN_SPV: &[u8] = include_bytes!(\"{}\");\n",
        atrous_plain.display()
    ));
    glue_text.push_str(&format!(
        "pub static LIGHTING_DIRECT_PLAIN_SPV: &[u8] = include_bytes!(\"{}\");\n",
        direct_plain.display()
    ));
    glue_text.push_str(&format!(
        "pub static LIGHTING_SCREEN_SPV: &[u8] = include_bytes!(\"{}\");\n",
        screen.display()
    ));
    std::fs::write(&glue, glue_text).unwrap();
    for piece in [
        "lighting_common.glsl",
        "lighting_pack.glsl",
        "lighting_cache.glsl",
        "lighting_cache_slots.glsl",
        "lighting_lamp_pick.glsl",
        "lighting_sun_compose.comp",
        "lighting_screen.comp",
        "media.glsl",
    ] {
        println!("cargo:rerun-if-changed=shaders/{piece}");
    }
    println!("cargo:rerun-if-changed=shaders/scene.vert");
    println!("cargo:rerun-if-changed=shaders/overlay.frag");
    println!("cargo:rerun-if-changed=shaders/depth_mask.frag");
    println!("cargo:rerun-if-changed=shaders/material_atlas.glsl");
    println!("cargo:rerun-if-changed=shaders/wire.vert");
    println!("cargo:rerun-if-changed=shaders/wire.frag");
    println!("cargo:rerun-if-changed=shaders/scene_rays.glsl");
    println!("cargo:rerun-if-changed=shaders/mesh_field.glsl");
    println!("cargo:rerun-if-changed=shaders/scene_data.glsl");
    println!("cargo:rerun-if-changed=shaders/dither.glsl");
    println!("cargo:rerun-if-changed=shaders/transmit.comp");
    println!("cargo:rerun-if-changed=shaders/fxaa.comp");
}

fn compile(src: &str, dest: &std::path::Path) -> PathBuf {
    compile_args(src, dest, &[])
}

/// `glslc -D`. The macro has to change the module: a ignored `-D` would ship
/// the reservoir shade under the plain name.
fn compile_glslc_define(src: &str, dest: &std::path::Path, define: &str) -> PathBuf {
    let flag = format!("-D{define}");
    compile_args(src, dest, &[&flag])
}

fn compile_args(src: &str, dest: &std::path::Path, extra: &[&str]) -> PathBuf {
    let mut args = vec![src];
    args.extend(extra);
    args.extend(["-o", dest.to_str().unwrap()]);
    let status = Command::new("glslc").args(&args).status().expect("glslc");
    if !status.success() {
        panic!("glslc failed for {src} {extra:?}");
    }
    dest.to_path_buf()
}

/// glslc's wrapper does not forward `-D`. The plain a-trous kernel needs the macro.
fn compile_define(src: &str, dest: &std::path::Path, define: &str) -> PathBuf {
    let status = Command::new("glslangValidator")
        .args([
            "-V",
            "--target-env",
            "vulkan1.0",
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
