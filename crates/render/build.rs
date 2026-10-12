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
    // Item A: the hit-light variant. Not dispatched until a later step.
    let direct_queue = compile_with(
        "shaders/lighting_direct.comp",
        &out.join("lighting_direct_queue.comp.spv"),
        &["LIGHTING_HIT_QUEUE=1"],
    );
    // Direct pass that queues GI-hit misses. Bound only when the
    // hit-light toggle is on, so the shipped pipeline stays the variant above.
    let direct_enqueue = compile_with(
        "shaders/lighting_direct.comp",
        &out.join("lighting_direct_enqueue.comp.spv"),
        &["LIGHTING_HIT_ENQUEUE=1"],
    );
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
        "pub static LIGHTING_DIRECT_QUEUE_SPV: &[u8] = include_bytes!(\"{}\");\n",
        direct_queue.display()
    ));
    glue_text.push_str(&format!(
        "pub static LIGHTING_DIRECT_ENQUEUE_SPV: &[u8] = include_bytes!(\"{}\");\n",
        direct_enqueue.display()
    ));
    std::fs::write(&glue, glue_text).unwrap();
    for piece in [
        "lighting_common.glsl",
        "lighting_pack.glsl",
        "lighting_cache.glsl",
        "lighting_cache_slots.glsl",
        "lighting_direct_classify.glsl",
        "lighting_direct_shade.glsl",
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
    println!("cargo:rerun-if-env-changed=GENOS_LIGHTING_DIAG");
    println!("cargo:rerun-if-env-changed=GENOS_DP_DIVERGE");
}

fn compile(src: &str, dest: &std::path::Path) -> PathBuf {
    compile_with(src, dest, &[])
}

/// `defines` are `-D` names (`NAME` or `NAME=value`), after the measurement defines.
fn compile_with(src: &str, dest: &std::path::Path, defines: &[&str]) -> PathBuf {
    let mut cmd = Command::new("glslc");
    cmd.arg(src).arg("-o").arg(dest.to_str().unwrap());
    // Phase 0 counters. The default build leaves them compiled out.
    if std::env::var("GENOS_LIGHTING_DIAG").is_ok_and(|v| v == "1") {
        cmd.arg("-DLIGHTING_DIAG=1");
    }
    // Phase 0.3. 0 is the shipped shader. 1, 2 and 3 skip GI-hit shadow rays
    // and the picture is wrong; never leave this on.
    if let Ok(mode) = std::env::var("GENOS_DP_DIVERGE") {
        match mode.as_str() {
            "0" | "" => {}
            "1" | "2" | "3" => {
                cmd.arg(format!("-DDP_DIVERGE={mode}"));
            }
            other => panic!("GENOS_DP_DIVERGE must be 0, 1, 2 or 3, got {other}"),
        }
    }
    for define in defines {
        cmd.arg(format!("-D{define}"));
    }
    let status = cmd.status().expect("glslc");
    if !status.success() {
        panic!("glslc failed for {src}");
    }
    dest.to_path_buf()
}
