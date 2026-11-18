//! Instance SSBO stride must match `InstanceRec` and come from one shared include.
//!
//! When `InstanceRec` grew to 144 B (object id) but `scene.vert` still used a
//! 128 B local `struct Instance`, depth and picture disagreed: every instance
//! after the first read the wrong model matrix, triangles vanished, and the
//! sky showed through (NVIDIA). Lavapipe often zeros OOB reads, so the hole
//! did not show there. Validation layers report 0 errors (in-bounds misread).

#![expect(clippy::expect_used, reason = "a missing field fails the gate")]
#![expect(clippy::panic, reason = "a missing field fails the gate")]

use std::mem::{offset_of, size_of};

/// Mirror of `InstanceRec` in `gpu.rs` — keep field order identical.
#[repr(C)]
struct InstanceRec {
    model: [f32; 16],
    color: [f32; 4],
    atlas: [f32; 4],
    normal_atlas: [f32; 4],
    extra: [f32; 4],
    ids: [u32; 4],
}

const HOST_BYTES: usize = 144;

fn shared_instance_src() -> &'static str {
    include_str!("../shaders/instance_ssbo.glsl")
}

/// std430 size of the shared GLSL `struct Instance` by walking its fields.
fn glsl_instance_layout(src: &str) -> (usize, Vec<(&'static str, usize)>) {
    let start = src
        .find("struct Instance")
        .expect("instance_ssbo.glsl: no struct Instance");
    let body = &src[start..];
    let end = body.find('}').expect("instance_ssbo.glsl: no closing brace");
    let block = &body[..=end];
    // Field order and std430 sizes (mat4=64, vec4/uvec4=16).
    let want = [
        ("mat4 model", 64usize),
        ("vec4 color", 16),
        ("vec4 atlas", 16),
        ("vec4 normal_atlas", 16),
        ("vec4 extra", 16),
        ("uvec4 ids", 16),
    ];
    let mut offset = 0usize;
    let mut offsets = Vec::new();
    let mut cursor = 0usize;
    for (name, size) in want {
        let at = block[cursor..]
            .find(name)
            .unwrap_or_else(|| panic!("instance_ssbo.glsl missing `{name}`"));
        cursor += at + name.len();
        offsets.push((name, offset));
        offset += size;
    }
    (offset, offsets)
}

#[test]
fn shared_instance_matches_host_instance_rec() {
    assert_eq!(size_of::<InstanceRec>(), HOST_BYTES);
    assert_eq!(offset_of!(InstanceRec, model), 0);
    assert_eq!(offset_of!(InstanceRec, color), 64);
    assert_eq!(offset_of!(InstanceRec, atlas), 80);
    assert_eq!(offset_of!(InstanceRec, normal_atlas), 96);
    assert_eq!(offset_of!(InstanceRec, extra), 112);
    assert_eq!(offset_of!(InstanceRec, ids), 128);

    let (glsl_size, offsets) = glsl_instance_layout(shared_instance_src());
    assert_eq!(
        glsl_size, HOST_BYTES,
        "GLSL Instance std430 size {glsl_size} != InstanceRec {HOST_BYTES}"
    );
    for (name, off) in &offsets {
        let host = match *name {
            "mat4 model" => offset_of!(InstanceRec, model),
            "vec4 color" => offset_of!(InstanceRec, color),
            "vec4 atlas" => offset_of!(InstanceRec, atlas),
            "vec4 normal_atlas" => offset_of!(InstanceRec, normal_atlas),
            "vec4 extra" => offset_of!(InstanceRec, extra),
            "uvec4 ids" => offset_of!(InstanceRec, ids),
            other => panic!("unexpected field {other}"),
        };
        assert_eq!(*off, host, "{name}: GLSL offset {off} != host {host}");
    }
}

#[test]
fn vertex_shaders_include_shared_instance_and_do_not_redeclare_it() {
    let verts = [
        ("scene.vert", include_str!("../shaders/scene.vert")),
        ("gbuffer.vert", include_str!("../shaders/gbuffer.vert")),
        (
            "reflections_portal.vert",
            include_str!("../shaders/reflections_portal.vert"),
        ),
    ];
    for (name, src) in verts {
        assert!(
            src.contains("#include \"instance_ssbo.glsl\""),
            "{name}: must #include \"instance_ssbo.glsl\""
        );
        assert!(
            !src.contains("struct Instance"),
            "{name}: must not declare its own struct Instance (use instance_ssbo.glsl)"
        );
    }
    // No other shader under shaders/ may redefine the SSBO Instance record.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders");
    for entry in std::fs::read_dir(&dir).expect("shaders/") {
        let entry = entry.expect("dirent");
        let path = entry.path();
        let Some(ext) = path.extension() else {
            continue;
        };
        if ext != "vert" && ext != "frag" && ext != "comp" && ext != "glsl" {
            continue;
        }
        if path.file_name().and_then(|s| s.to_str()) == Some("instance_ssbo.glsl") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read shader");
        assert!(
            !text.contains("struct Instance"),
            "{}: declares struct Instance; use instance_ssbo.glsl instead",
            path.display()
        );
    }
}
