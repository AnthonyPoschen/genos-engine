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
    std::fs::write(&glue, glue_text).unwrap();
    for piece in [
        "lighting_common.glsl",
        "lighting_pack.glsl",
        "lighting_cache.glsl",
        "lighting_cache_slots.glsl",
        "lighting_work.glsl",
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
    // The picture compose returns before the regime count and the sun patch.
    // Mark those functions so a driver that inlines the entry point does not
    // fold them into the picture dispatch. A full compose then stays the base shade.
    if src.ends_with("lighting_compose.comp") {
        mark_dont_inline(dest);
    }
    dest.to_path_buf()
}

/// SPIR-V function control DontInline. Best-effort: a compiler that already
/// inlined the function leaves no `OpFunction` to mark.
fn mark_dont_inline(path: &std::path::Path) {
    const DONT_INLINE: u32 = 2;
    const OP_NAME: u32 = 5;
    const OP_FUNCTION: u32 = 54;
    let prefixes = [
        "lighting_compose_extra",
        "lighting_work_count_px",
        "lighting_work_px_tag",
        "lighting_work_tube",
    ];
    let mut bytes = std::fs::read(path).expect("read spir-v");
    if bytes.len() < 20 || bytes.len() % 4 != 0 {
        return;
    }
    let mut words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if words[0] != 0x0723_0203 {
        return;
    }
    let mut names: Vec<(u32, String)> = Vec::new();
    let mut i = 5;
    while i < words.len() {
        let wc = (words[i] >> 16) as usize;
        let op = words[i] & 0xffff;
        if wc == 0 || i + wc > words.len() {
            break;
        }
        if op == OP_NAME && wc >= 3 {
            let mut raw = Vec::with_capacity((wc - 2) * 4);
            for w in &words[i + 2..i + wc] {
                raw.extend_from_slice(&w.to_le_bytes());
            }
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            if let Ok(name) = std::str::from_utf8(&raw[..end]) {
                names.push((words[i + 1], name.to_string()));
            }
        }
        i += wc;
    }
    i = 5;
    while i < words.len() {
        let wc = (words[i] >> 16) as usize;
        let op = words[i] & 0xffff;
        if wc == 0 || i + wc > words.len() {
            break;
        }
        if op == OP_FUNCTION && wc >= 5 {
            let id = words[i + 2];
            if names.iter().any(|(nid, name)| {
                *nid == id && prefixes.iter().any(|p| name.starts_with(p))
            }) {
                words[i + 3] |= DONT_INLINE;
            }
        }
        i += wc;
    }
    bytes.clear();
    for w in words {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("write spir-v");
}
