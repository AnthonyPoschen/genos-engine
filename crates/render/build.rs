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
    write_shader_glue(&out, &raster, &media, &denoise, &lighting, &reflection_spv);
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
    texel_detail: PathBuf,
    texel_detail_ro: PathBuf,
    texel_ray_cut: PathBuf,
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
        texel_detail: compile_texel_detail(
            "shaders/texel_detail_prod.comp",
            &out.join("texel_detail_prod.comp.spv"),
            &[],
            true,
        ),
        // Same production function, compose's read-only specialization.
        // PROD_READONLY drops the writing rows and folds the fixture's one head
        // to literals. Dead arms are then removed, and large functions get
        // DontInline. Without that fold the reader was killed on a 16 GiB
        // lavapipe cgroup (anon-rss 14711896 kB) before its assertions.
        texel_detail_ro: compile_texel_detail(
            "shaders/texel_detail_prod.comp",
            &out.join("texel_detail_prod_ro.comp.spv"),
            &["-DLIGHTING_SC_WRITES=false", "-DPROD_READONLY=1"],
            true,
        ),
        // One specialization. No screen-probe include and no DontInline edit.
        texel_ray_cut: compile(
            "shaders/texel_ray_cut_prod.comp",
            &out.join("texel_ray_cut_prod.comp.spv"),
        ),
    };
    // Not embedded: catches errors in a piece no pass includes yet.
    let reflections_check = "shaders/reflections_check.comp";
    println!("cargo:rerun-if-changed={reflections_check}");
    compile(reflections_check, &out.join("reflections_check.comp.spv"));
    // Not embedded: GPU parity fixture for pbr_brdf.glsl (tests/pbr_parity.rs).
    let pbr_check = "shaders/pbr_check.comp";
    println!("cargo:rerun-if-changed={pbr_check}");
    println!("cargo:rerun-if-changed=shaders/pbr_brdf.glsl");
    compile(pbr_check, &out.join("pbr_check.comp.spv"));
    media
}

struct DenoiseModules {
    screen: PathBuf,
    atrous_plain: PathBuf,
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
    DenoiseModules {
        screen,
        atrous_plain,
    }
}

fn compile_lighting_modules(out: &std::path::Path) -> Vec<(&'static str, PathBuf)> {
    [
        ("GBUFFER_VERT_SPV", "gbuffer.vert"),
        ("GBUFFER_FRAG_SPV", "gbuffer.frag"),
        ("LIGHTING_COMPACT_SPV", "lighting_compact.comp"),
        ("LIGHTING_PLACE_SPV", "lighting_noop.comp"),
        ("LIGHTING_TRACE_SPV", "lighting_trace.comp"),
        ("LIGHTING_DIRECT_SPV", "lighting_direct.comp"),
        ("LIGHTING_GATHER_SPV", "lighting_noop.comp"),
        ("LIGHTING_FILTER_SPV", "lighting_noop.comp"),
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
        let dest = out.join(format!("{file}.spv"));
        // Every compose compile, flags on or off, loads the cache relight.
        // The neighbour walk is not in the module.
        let spv = if *file == "lighting_compose.comp" || *file == "lighting_sun_compose.comp" {
            compile_args(&src, &dest, &["-DGENOS_COMPOSE_NO_WALK=1"])
        } else {
            compile(&src, &dest)
        };
        (*name, spv)
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
            "pub static VERT_SPV: &[u8] = include_bytes!(\"{}\");\npub static FRAG_SPV: &[u8] = include_bytes!(\"{}\");\npub static DEPTH_MASK_SPV: &[u8] = include_bytes!(\"{}\");\npub static WIRE_VERT_SPV: &[u8] = include_bytes!(\"{}\");\npub static WIRE_FRAG_SPV: &[u8] = include_bytes!(\"{}\");\npub static AUDIO_SPV: &[u8] = include_bytes!(\"{}\");\npub static FXAA_SPV: &[u8] = include_bytes!(\"{}\");\npub static MEDIA_CHECK_SPV: &[u8] = include_bytes!(\"{}\");\npub static FROXEL_INJECT_SPV: &[u8] = include_bytes!(\"{}\");\npub static FROXEL_INTEGRATE_SPV: &[u8] = include_bytes!(\"{}\");\npub static TEXEL_DETAIL_PROD_SPV: &[u8] = include_bytes!(\"{}\");\npub static TEXEL_DETAIL_PROD_RO_SPV: &[u8] = include_bytes!(\"{}\");\npub static TEXEL_RAY_CUT_SPV: &[u8] = include_bytes!(\"{}\");\n",
            raster.vert.display(),
            raster.frag.display(),
            raster.depth_mask.display(),
            raster.wire_vert.display(),
            raster.wire.display(),
            raster.audio.display(),
            raster.fxaa.display(),
            media.check.display(),
            media.inject.display(),
            media.integrate.display(),
            media.texel_detail.display(),
            media.texel_detail_ro.display(),
            media.texel_ray_cut.display()
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
    let (live, spatial, wide) = compile_restir_live(out);
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
        "pub static RESTIR_SPATIAL_SPV: &[u8] = include_bytes!(\"{}\");\n",
        spatial.display()
    ));
    glue_text.push_str(&format!(
        "pub static RESTIR_SPATIAL_WIDE_SPV: &[u8] = include_bytes!(\"{}\");\n",
        wide.display()
    ));
    glue_text.push_str(&format!(
        "pub static LIGHTING_DIRECT_RESTIR_SPV: &[u8] = include_bytes!(\"{}\");\n",
        direct.display()
    ));
}

fn compile_restir_live(out: &std::path::Path) -> (PathBuf, PathBuf, PathBuf) {
    println!("cargo:rerun-if-changed=shaders/restir_live.comp");
    for piece in [
        "../restir-gpu/shaders/restir_tree.glsl",
        "../restir-gpu/shaders/restir_sample.glsl",
        "../restir-gpu/shaders/restir_merge.glsl",
        "../restir-gpu/shaders/restir_spatial_ops.glsl",
    ] {
        println!("cargo:rerun-if-changed={piece}");
    }
    let live = out.join("restir_live.comp.spv");
    let spatial = out.join("restir_spatial.comp.spv");
    let wide = out.join("restir_spatial_wide.comp.spv");
    compile_restir_variant("shaders/restir_live.comp", &live, &[]);
    compile_restir_variant(
        "shaders/restir_live.comp",
        &spatial,
        &["-DRESTIR_SPATIAL_ONLY", "-DRESTIR_SPATIAL_CAP"],
    );
    compile_restir_variant(
        "shaders/restir_live.comp",
        &wide,
        &["-DRESTIR_SPATIAL_ONLY"],
    );
    (live, spatial, wide)
}

fn compile_restir_variant(src: &str, dest: &std::path::Path, defines: &[&str]) -> PathBuf {
    let mut args = vec![
        "-V".to_string(),
        "--target-env".to_string(),
        "vulkan1.0".to_string(),
        "-I../restir-gpu/shaders".to_string(),
    ];
    args.extend(defines.iter().map(|d| (*d).to_string()));
    args.push(src.to_string());
    args.push("-o".to_string());
    args.push(dest.to_str().unwrap().to_string());
    let status = Command::new("glslangValidator")
        .args(&args)
        .status()
        .expect("glslangValidator");
    if !status.success() {
        panic!("glslangValidator failed for {src} {defines:?}");
    }
    dest.to_path_buf()
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
        "lighting_screen_probe.glsl",
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
        "texel_detail_prod.comp",
        "texel_ray_cut_prod.comp",
        "froxel_compose.glsl",
        "lighting_check.comp",
        "lighting_sh_drain.comp",
        "scene.vert",
        "overlay.frag",
        "depth_mask.frag",
        "material_atlas.glsl",
        "gbuffer_layout.glsl",
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
        "pbr_brdf.glsl",
        "pbr_check.comp",
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
    let compose_hdr = compile_defines(
        "shaders/lighting_compose.comp",
        &out.join("lighting_compose_hdr.comp.spv"),
        &["GENOS_HDR_COMPOSE", "GENOS_COMPOSE_NO_WALK"],
    );
    // #142: the compose writes, run just before the read-only compose.
    // The walk stays out of this compile. Maintenance enqueues. The drain gathers.
    let maint = compile_args(
        "shaders/lighting_compose.comp",
        &out.join("lighting_compose_maint.comp.spv"),
        &["-DGENOS_COMPOSE_MAINT", "-DGENOS_COMPOSE_NO_WALK=1"],
    );
    glue_text.push_str(&format!(
        "pub static LIGHTING_COMPOSE_MAINT_SPV: &[u8] = include_bytes!(\"{}\");\n",
        maint.display()
    ));
    // The neighbour walk lives here, after maintenance and before compose.
    let drain = compile(
        "shaders/lighting_sh_drain.comp",
        &out.join("lighting_sh_drain.comp.spv"),
    );
    glue_text.push_str(&format!(
        "pub static LIGHTING_SH_DRAIN_SPV: &[u8] = include_bytes!(\"{}\");\n",
        drain.display()
    ));
    let sun_hdr = compile_defines(
        "shaders/lighting_sun_compose.comp",
        &out.join("lighting_sun_compose_hdr.comp.spv"),
        &["GENOS_HDR_COMPOSE", "GENOS_COMPOSE_NO_WALK"],
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

/// The lavapipe fixture only. Inlining the compose function at every mover
/// shade makes the compiler heap exceed a 16 GB software-test budget.
/// `DontInline` does not change the GLSL those functions run. The fixture
/// keeps the neighbour walk. Every compose compile defines
/// `GENOS_COMPOSE_NO_WALK` and does not.
fn compile_texel_detail(
    src: &str,
    dest: &std::path::Path,
    extra: &[&str],
    dont_inline: bool,
) -> PathBuf {
    let path = compile_args(src, dest, extra);
    if extra.iter().any(|flag| flag.contains("PROD_READONLY")) {
        fold_readonly_module(&path);
    }
    if !dont_inline {
        return path;
    }
    let mut bytes = std::fs::read(&path).expect("texel detail spirv");
    let mut i = 5usize;
    let n = bytes.len() / 4;
    let mut fn_ctrl: Option<usize> = None;
    let mut fn_start = 0usize;
    while i < n {
        let word = u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().expect("spirv word"));
        let count = usize::try_from(word >> 16).expect("spirv word count");
        let op = word & 0xffff;
        if count == 0 || i + count > n {
            panic!("texel detail spirv is truncated");
        }
        if op == 54 && count >= 4 {
            fn_ctrl = Some(i + 3);
            fn_start = i;
        } else if op == 56 {
            // Only the large functions. Marking every helper DontInline keeps a
            // separate copy of each and the compiler heap does not fit.
            if let Some(ctrl_i) = fn_ctrl {
                if i - fn_start > 1000 {
                    let at = ctrl_i * 4;
                    let ctrl =
                        u32::from_le_bytes(bytes[at..at + 4].try_into().expect("spirv control"));
                    let ctrl = (ctrl & !1) | 2;
                    bytes[at..at + 4].copy_from_slice(&ctrl.to_le_bytes());
                }
            }
            fn_ctrl = None;
        }
        i += count;
    }
    std::fs::write(&path, &bytes).expect("write texel detail spirv");
    path
}

const OP_CONSTANT_TRUE: u32 = 41;
const OP_CONSTANT_FALSE: u32 = 42;
const OP_CONSTANT: u32 = 43;
const OP_CONSTANT_NULL: u32 = 46;
const OP_FUNCTION: u32 = 54;
const OP_FUNCTION_END: u32 = 56;
const OP_FUNCTION_CALL: u32 = 57;
const OP_LOAD: u32 = 61;
const OP_STORE: u32 = 62;
const OP_ACCESS_CHAIN: u32 = 65;
const OP_COPY_OBJECT: u32 = 83;
const OP_RETURN: u32 = 253;
const OP_RETURN_VALUE: u32 = 254;

fn spirv_words(bytes: &[u8]) -> Vec<u32> {
    if !bytes.len().is_multiple_of(4) {
        panic!("spirv length {} is not a multiple of 4", bytes.len());
    }
    let (chunks, _) = bytes.as_chunks::<4>();
    chunks
        .iter()
        .map(|chunk| u32::from_le_bytes(*chunk))
        .collect()
}

fn spirv_bytes(words: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn spirv_instruction(words: &[u32], index: usize) -> (usize, u32) {
    let word = words.get(index).copied().expect("spirv word");
    let count = usize::try_from(word >> 16).expect("spirv word count");
    if count == 0 || index + count > words.len() {
        panic!("readonly spirv is truncated");
    }
    (count, word & 0xffff)
}

fn module_constants(words: &[u32]) -> std::collections::BTreeSet<u32> {
    let mut constants = std::collections::BTreeSet::new();
    let mut index = 5usize;
    while index < words.len() {
        let (count, op) = spirv_instruction(words, index);
        if op == OP_FUNCTION {
            break;
        }
        if matches!(
            op,
            OP_CONSTANT | OP_CONSTANT_TRUE | OP_CONSTANT_FALSE | OP_CONSTANT_NULL
        ) && count >= 3
        {
            constants.insert(words[index + 2]);
        }
        index += count;
    }
    constants
}

/// Functions whose body only returns one module-scope constant. A call to one
/// of these is the constant: both arms of the caller's branch stay in the
/// module until that call is a copy.
fn trivial_returns(
    words: &[u32],
    constants: &std::collections::BTreeSet<u32>,
) -> std::collections::BTreeMap<u32, u32> {
    let mut found = std::collections::BTreeMap::new();
    let mut index = 5usize;
    while index < words.len() {
        let (count, op) = spirv_instruction(words, index);
        if op != OP_FUNCTION || count < 3 {
            index += count;
            continue;
        }
        let id = words[index + 2];
        index += count;
        let mut returned: Option<u32> = None;
        let mut rejected = false;
        let mut ended = false;
        while index < words.len() {
            let (inner, inner_op) = spirv_instruction(words, index);
            if matches!(
                inner_op,
                OP_LOAD | OP_STORE | OP_ACCESS_CHAIN | OP_FUNCTION_CALL | OP_RETURN
            ) {
                rejected = true;
            }
            if inner_op == OP_RETURN_VALUE {
                if returned.is_some() {
                    rejected = true;
                }
                returned = Some(words[index + 1]);
            }
            index += inner;
            if inner_op == OP_FUNCTION_END {
                ended = true;
                break;
            }
        }
        if ended && !rejected {
            if let Some(value) = returned {
                if constants.contains(&value) {
                    found.insert(id, value);
                }
            }
        }
    }
    found
}

fn inline_constant_returns(words: &mut [u32]) -> usize {
    let constants = module_constants(words);
    let trivial = trivial_returns(words, &constants);
    let mut replaced = 0usize;
    let mut index = 5usize;
    while index < words.len() {
        let (count, op) = spirv_instruction(words, index);
        if op == OP_FUNCTION_CALL && count == 4 {
            let callee = words[index + 3];
            if let Some(constant) = trivial.get(&callee) {
                words[index] = (4 << 16) | OP_COPY_OBJECT;
                words[index + 3] = *constant;
                replaced += 1;
            }
        }
        index += count;
    }
    replaced
}

/// The read-only fixture uses one head. The shader macros turn that head into
/// literals, but glslc still leaves the helpers that *call* those literals as
/// real functions, so both arms survive. Copy the constant returns, then let
/// spirv-opt drop the dead arms. `simplify-instructions` stays off: it can
/// reassociate floats, and the fixture's thresholds are frozen.
fn fold_readonly_module(path: &std::path::Path) {
    let folded = path.with_extension("fold.spv");
    let mut bytes = std::fs::read(path).expect("read readonly spirv");
    for _ in 0..8 {
        let mut words = spirv_words(&bytes);
        let replaced = inline_constant_returns(&mut words);
        bytes = spirv_bytes(&words);
        std::fs::write(path, &bytes).expect("write readonly spirv");
        let status = Command::new("spirv-opt")
            .args([
                "--ccp",
                "--eliminate-dead-code-aggressive",
                "--eliminate-dead-functions",
                "--eliminate-dead-branches",
                path.to_str().unwrap(),
                "-o",
                folded.to_str().unwrap(),
            ])
            .status()
            .expect("spirv-opt (package spirv-tools)");
        if !status.success() {
            panic!("spirv-opt failed for {}", path.display());
        }
        bytes = std::fs::read(&folded).expect("read folded spirv");
        if replaced == 0 {
            break;
        }
    }
    let _ = std::fs::remove_file(&folded);
    std::fs::write(path, &bytes).expect("write folded spirv");
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
    compile_defines(src, dest, &[define])
}

fn compile_defines(src: &str, dest: &std::path::Path, defines: &[&str]) -> PathBuf {
    let flags: Vec<String> = defines.iter().map(|define| format!("-D{define}")).collect();
    let mut args = vec![
        "-V".to_string(),
        "--target-env".to_string(),
        "vulkan1.0".to_string(),
    ];
    args.extend(flags);
    args.push(src.to_string());
    args.push("-o".to_string());
    args.push(dest.to_str().unwrap().to_string());
    let status = Command::new("glslangValidator")
        .args(&args)
        .status()
        .expect("glslangValidator");
    if !status.success() {
        panic!("glslangValidator failed for {src} {defines:?}");
    }
    dest.to_path_buf()
}
