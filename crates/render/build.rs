#![expect(
    clippy::expect_used,
    reason = "the build stops when a host tool or OUT_DIR is missing"
)]
#![expect(
    clippy::format_push_string,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
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
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let raster = compile_raster_modules(&out);
    let reflection_spv = reflection_shaders(&out);
    let media = compile_media_modules(&out);
    let denoise = compile_denoise_modules(&out);
    let lighting = compile_lighting_modules(&out);
    write_shader_glue(
        &out,
        &raster,
        &media,
        &denoise,
        &lighting,
        &reflection_spv,
    );
    rerun_shader_pieces();
}

struct RasterModules {
    vert: PathBuf,
    frag: PathBuf,
    depth_mask: PathBuf,
    wire_vert: PathBuf,
    wire: PathBuf,
    audio: PathBuf,
    fxaa: PathBuf,
}

fn compile_raster_modules(out: &std::path::Path) -> RasterModules {
    RasterModules {
        vert: compile("shaders/scene.vert", &out.join("scene.vert.spv")),
        frag: compile("shaders/overlay.frag", &out.join("overlay.frag.spv")),
        depth_mask: compile("shaders/depth_mask.frag", &out.join("depth_mask.frag.spv")),
        wire_vert: compile("shaders/wire.vert", &out.join("wire.vert.spv")),
        wire: compile("shaders/wire.frag", &out.join("wire.frag.spv")),
        audio: compile("shaders/transmit.comp", &out.join("transmit.comp.spv")),
        fxaa: compile("shaders/fxaa.comp", &out.join("fxaa.comp.spv")),
    }
}

struct MediaModules {
    check: PathBuf,
    inject: PathBuf,
    integrate: PathBuf,
}

/// Media and froxel modules, and the two compile-only checks that sit with them.
fn compile_media_modules(out: &std::path::Path) -> MediaModules {
    // Compile check only: no pass includes lighting_check.comp yet.
    let _lighting_check = compile(
        "shaders/lighting_check.comp",
        &out.join("lighting_check.comp.spv"),
    );
    let media = MediaModules {
        check: compile(
            "shaders/media_check.comp",
            &out.join("media_check.comp.spv"),
        ),
        inject: compile(
            "shaders/froxel_inject.comp",
            &out.join("froxel_inject.comp.spv"),
        ),
        integrate: compile(
            "shaders/froxel_integrate.comp",
            &out.join("froxel_integrate.comp.spv"),
        ),
    };
    // Not embedded: catches errors in a piece no pass includes yet.
    let reflections_check = "shaders/reflections_check.comp";
    println!("cargo:rerun-if-changed={reflections_check}");
    compile(
        reflections_check,
        &out.join("reflections_check.comp.spv"),
    );
    media
}

struct DenoiseModules {
    screen: PathBuf,
    atrous_plain: PathBuf,
    direct_plain: PathBuf,
}

fn compile_denoise_modules(out: &std::path::Path) -> DenoiseModules {
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
    DenoiseModules {
        screen,
        atrous_plain,
        direct_plain,
    }
}

fn compile_lighting_modules(out: &std::path::Path) -> Vec<(&'static str, PathBuf)> {
    [
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
        ("HDR_AGX_SPV", "hdr_agx.comp"),
        ("HDR_BLOOM_SPV", "hdr_bloom.comp"),
        ("HDR_METER_SPV", "hdr_meter.comp"),
    ]
    .iter()
    .map(|(name, file)| {
        let src = format!("shaders/{file}");
        println!("cargo:rerun-if-changed={src}");
        (*name, compile(&src, &out.join(format!("{file}.spv"))))
    })
    .collect()
}

fn write_shader_glue(
    out: &std::path::Path,
    raster: &RasterModules,
    media: &MediaModules,
    denoise: &DenoiseModules,
    lighting: &[(&str, PathBuf)],
    reflection_spv: &[(&str, PathBuf)],
) {
    let glue = out.join("shaders.rs");
    std::fs::write(
        &glue,
        format!(
            "pub static VERT_SPV: &[u8] = include_bytes!(\"{}\");\npub static FRAG_SPV: &[u8] = include_bytes!(\"{}\");\npub static DEPTH_MASK_SPV: &[u8] = include_bytes!(\"{}\");\npub static WIRE_VERT_SPV: &[u8] = include_bytes!(\"{}\");\npub static WIRE_FRAG_SPV: &[u8] = include_bytes!(\"{}\");\npub static AUDIO_SPV: &[u8] = include_bytes!(\"{}\");\npub static FXAA_SPV: &[u8] = include_bytes!(\"{}\");\npub static MEDIA_CHECK_SPV: &[u8] = include_bytes!(\"{}\");\npub static FROXEL_INJECT_SPV: &[u8] = include_bytes!(\"{}\");\npub static FROXEL_INTEGRATE_SPV: &[u8] = include_bytes!(\"{}\");\n",
            raster.vert.display(),
            raster.frag.display(),
            raster.depth_mask.display(),
            raster.wire_vert.display(),
            raster.wire.display(),
            raster.audio.display(),
            raster.fxaa.display(),
            media.check.display(),
            media.inject.display(),
            media.integrate.display()
        ),
    )
    .unwrap();
    let mut glue_text = std::fs::read_to_string(&glue).unwrap();
    for (name, path) in lighting {
        glue_text.push_str(&format!(
            "pub static {name}: &[u8] = include_bytes!(\"{}\");\n",
            path.display()
        ));
    }
    glue_text.push_str(&format!(
        "pub static LIGHTING_ATROUS_PLAIN_SPV: &[u8] = include_bytes!(\"{}\");\n",
        denoise.atrous_plain.display()
    ));
    glue_text.push_str(&format!(
        "pub static LIGHTING_DIRECT_PLAIN_SPV: &[u8] = include_bytes!(\"{}\");\n",
        denoise.direct_plain.display()
    ));
    glue_text.push_str(&format!(
        "pub static LIGHTING_SCREEN_SPV: &[u8] = include_bytes!(\"{}\");\n",
        denoise.screen.display()
    ));
    for (name, path) in reflection_spv {
        glue_text.push_str(&format!(
            "pub static {name}: &[u8] = include_bytes!(\"{}\");\n",
            path.display()
        ));
    }
    append_hdr_compose(&mut glue_text, out);
    append_restir(&mut glue_text, out);
    std::fs::write(&glue, glue_text).unwrap();
}

fn append_restir(glue_text: &mut String, out: &std::path::Path) {
    let live = compile_restir_live(out);
    let direct = compile_glslc_define(
        "shaders/lighting_direct.comp",
        &out.join("lighting_direct_restir.comp.spv"),
        "GENOS_RESTIR_DI",
    );
    glue_text.push_str(&format!(
        "pub static RESTIR_LIVE_SPV: &[u8] = include_bytes!(\"{}\");\n",
        live.display()
    ));
    glue_text.push_str(&format!(
        "pub static LIGHTING_DIRECT_RESTIR_SPV: &[u8] = include_bytes!(\"{}\");\n",
        direct.display()
    ));
}

fn compile_restir_live(out: &std::path::Path) -> PathBuf {
    println!("cargo:rerun-if-changed=shaders/restir_live.comp");
    for piece in [
        "../restir-gpu/shaders/restir_tree.glsl",
        "../restir-gpu/shaders/restir_sample.glsl",
        "../restir-gpu/shaders/restir_merge.glsl",
        "../restir-gpu/shaders/restir_spatial_ops.glsl",
    ] {
        println!("cargo:rerun-if-changed={piece}");
    }
    let dest = out.join("restir_live.comp.spv");
    compile_args(
        "shaders/restir_live.comp",
        &dest,
        &["-I", "../restir-gpu/shaders"],
    )
}

fn reflection_shaders(out: &std::path::Path) -> Vec<(&'static str, PathBuf)> {
    [
        ("REFLECTION_FRAME_SPV", "reflections_frame.comp"),
        ("REFLECTION_WATER_SPV", "reflections_water.frag"),
        ("REFLECTION_ALBEDO_SPV", "reflections_albedo.frag"),
        ("REFLECTION_QUAD_SPV", "reflections_quad.vert"),
        ("REFLECTION_PORTAL_SPV", "reflections_portal.vert"),
        ("REFLECTION_COMPOSITE_SPV", "reflections_composite.frag"),
        ("REFLECTION_GGX_SPV", "reflections_ggx.frag"),
    ]
    .into_iter()
    .map(|(name, file)| {
        let src = format!("shaders/{file}");
        (name, compile(&src, &out.join(format!("{file}.spv"))))
    })
    .collect()
}

fn rerun_shader_pieces() {
    for piece in [
        "lighting_common.glsl",
        "lighting_pack.glsl",
        "lighting_cache.glsl",
        "lighting_cache_slots.glsl",
        "lighting_lamp_pick.glsl",
        "lighting_sun_compose.comp",
        "lighting_screen.comp",
        "media.glsl",
        "media_check.comp",
        "froxel_common.glsl",
        "froxel_inject.comp",
        "froxel_integrate.comp",
        "froxel_compose.glsl",
        "lighting_check.comp",
        "scene.vert",
        "overlay.frag",
        "depth_mask.frag",
        "material_atlas.glsl",
        "wire.vert",
        "wire.frag",
        "scene_rays.glsl",
        "mesh_field.glsl",
        "reflections.glsl",
        "reflections_frame.comp",
        "reflections_water.frag",
        "reflections_albedo.frag",
        "reflections_quad.vert",
        "reflections_portal.vert",
        "reflections_composite.frag",
        "reflections_ggx.frag",
        "reflections_check.comp",
        "scene_data.glsl",
        "dither.glsl",
        "hdr_shoulder.glsl",
        "hdr_meter.comp",
        "hdr_display.glsl",
        "hdr_agx.comp",
        "transmit.comp",
        "fxaa.comp",
    ] {
        println!("cargo:rerun-if-changed=shaders/{piece}");
    }
}

/// HDR compose stores scene radiance. The plain compile does not declare it.
fn append_hdr_compose(glue_text: &mut String, out: &std::path::Path) {
    let compose_hdr = compile_define(
        "shaders/lighting_compose.comp",
        &out.join("lighting_compose_hdr.comp.spv"),
        "GENOS_HDR_COMPOSE",
    );
    let sun_hdr = compile_define(
        "shaders/lighting_sun_compose.comp",
        &out.join("lighting_sun_compose_hdr.comp.spv"),
        "GENOS_HDR_COMPOSE",
    );
    glue_text.push_str(&format!(
        "pub static LIGHTING_COMPOSE_HDR_SPV: &[u8] = include_bytes!(\"{}\");\n",
        compose_hdr.display()
    ));
    glue_text.push_str(&format!(
        "pub static SUN_COMPOSE_HDR_SPV: &[u8] = include_bytes!(\"{}\");\n",
        sun_hdr.display()
    ));
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
