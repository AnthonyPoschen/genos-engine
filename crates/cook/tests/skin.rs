#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! `.gskin` cook, the `skin=` manifest token, and a trailer that fails verify.

use std::path::{Path, PathBuf};

use genos_bake::{BakeMode, BakeSettings};
use genos_cook::{cook_asset, load_cooked, package_files, Manifest, PackageFile, Threads};
use genos_load::{decode_skin, sample_channel, sha256, ChannelPath, Interpolation};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("genos-gskin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn settings(dir: &Path) -> BakeSettings {
    BakeSettings {
        mode: BakeMode::OnLoad,
        cache_dir: dir.join("bake"),
        ..BakeSettings::default()
    }
}

struct Bin(Vec<u8>);

impl Bin {
    fn align(&mut self) {
        while !self.0.len().is_multiple_of(4) {
            self.0.push(0);
        }
    }

    fn f32s(&mut self, values: &[f32]) -> usize {
        self.align();
        let offset = self.0.len();
        for value in values {
            self.0.extend(value.to_le_bytes());
        }
        offset
    }

    fn u16s(&mut self, values: &[u16]) -> usize {
        self.align();
        let offset = self.0.len();
        for value in values {
            self.0.extend(value.to_le_bytes());
        }
        offset
    }
}

fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
    let mut json = json.as_bytes().to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = u32::try_from(12 + 8 + json.len() + 8 + bin.len()).unwrap();
    let mut out = Vec::new();
    out.extend(0x4654_6C67u32.to_le_bytes());
    out.extend(2u32.to_le_bytes());
    out.extend(total.to_le_bytes());
    out.extend(u32::try_from(json.len()).unwrap().to_le_bytes());
    out.extend(0x4E4F_534Au32.to_le_bytes());
    out.extend(json);
    out.extend(u32::try_from(bin.len()).unwrap().to_le_bytes());
    out.extend(0x004E_4942u32.to_le_bytes());
    out.extend(bin);
    out
}

fn accessor(offset: usize, count: usize, ctype: &str, kind: &str) -> String {
    format!(
        r#"{{"bufferView":0,"byteOffset":{offset},"componentType":{ctype},"count":{count},"type":"{kind}"}}"#
    )
}

/// Hips, knee, four weights plus a fifth, one linear clip, one step clip, a morph.
fn skinned_glb() -> Vec<u8> {
    let mut bin = Bin(Vec::new());
    let positions = bin.f32s(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let normals = bin.f32s(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
    let joints_0 = bin.u16s(&[0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0]);
    let weights_0 = bin.f32s(&[
        1.0, 0.0, 0.0, 0.0, 0.25, 0.75, 0.0, 0.0, 0.5, 0.25, 0.125, 0.0625, 0.0, 0.0, 0.0, 0.0,
    ]);
    let joints_1 = bin.u16s(&[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
    let weights_1 = bin.f32s(&[
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.125, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    ]);
    let indices = bin.u16s(&[0, 1, 2]);
    let inverse = bin.f32s(&[
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);
    let move_times = bin.f32s(&[0.0, 1.0]);
    let move_values = bin.f32s(&[0.0, 1.0, 0.0, 2.0, 1.0, 0.0]);
    let step_times = bin.f32s(&[0.0, 1.0]);
    let step_values = bin.f32s(&[0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]);
    let byte_length = bin.0.len();
    let acc = [
        accessor(positions, 4, "5126", "VEC3"),
        accessor(normals, 4, "5126", "VEC3"),
        accessor(joints_0, 4, "5123", "VEC4"),
        accessor(weights_0, 4, "5126", "VEC4"),
        accessor(joints_1, 4, "5123", "VEC4"),
        accessor(weights_1, 4, "5126", "VEC4"),
        accessor(indices, 3, "5123", "SCALAR"),
        accessor(inverse, 2, "5126", "MAT4"),
        accessor(move_times, 2, "5126", "SCALAR"),
        accessor(move_values, 2, "5126", "VEC3"),
        accessor(step_times, 2, "5126", "SCALAR"),
        accessor(step_values, 2, "5126", "VEC4"),
    ];
    let json = format!(
        r#"{{
            "asset": {{"version": "2.0"}},
            "scene": 0,
            "scenes": [{{"nodes": [0]}}],
            "nodes": [
                {{"name": "hips", "translation": [0, 1, 0], "children": [1]}},
                {{"name": "knee", "translation": [0, -0.5, 0], "mesh": 0, "skin": 0}},
                {{"name": "prop", "translation": [1, 0, 0]}}
            ],
            "skins": [{{"name": "body", "joints": [0, 1], "inverseBindMatrices": 7}}],
            "meshes": [{{
                "name": "limb",
                "primitives": [{{
                    "attributes": {{
                        "POSITION": 0, "NORMAL": 1,
                        "JOINTS_0": 2, "WEIGHTS_0": 3,
                        "JOINTS_1": 4, "WEIGHTS_1": 5
                    }},
                    "indices": 6,
                    "targets": [{{"POSITION": 0}}]
                }}]
            }}],
            "animations": [
                {{
                    "name": "move",
                    "samplers": [
                        {{"input": 8, "output": 9, "interpolation": "LINEAR"}},
                        {{"input": 8, "output": 9, "interpolation": "CUBICSPLINE"}}
                    ],
                    "channels": [
                        {{"sampler": 0, "target": {{"node": 0, "path": "translation"}}}},
                        {{"sampler": 0, "target": {{"node": 0, "path": "weights"}}}},
                        {{"sampler": 1, "target": {{"node": 0, "path": "translation"}}}},
                        {{"sampler": 0, "target": {{"node": 2, "path": "translation"}}}}
                    ]
                }},
                {{
                    "name": "hold",
                    "samplers": [{{"input": 10, "output": 11, "interpolation": "STEP"}}],
                    "channels": [{{"sampler": 0, "target": {{"node": 1, "path": "rotation"}}}}]
                }}
            ],
            "buffers": [{{"byteLength": {byte_length}}}],
            "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": {byte_length}}}],
            "accessors": [{accessors}]
        }}"#,
        accessors = acc.join(","),
    );
    glb(&json, &bin.0)
}

#[test]
fn a_skinned_source_writes_gskin_and_a_bad_trailer_fails_verify() {
    let dir = temp("skin");
    let src = dir.join("src");
    let out = dir.join("out");
    std::fs::create_dir_all(&src).unwrap();
    let source = src.join("limb.glb");
    std::fs::write(&source, skinned_glb()).unwrap();
    let cooked = cook_asset(&source, &out, &settings(&dir), false, Threads::ONE).unwrap();
    let (_, skin_path) = cooked.skin.unwrap();
    let bytes = std::fs::read(&skin_path).unwrap();
    let decoded = decode_skin(&bytes).unwrap();
    let joints = &decoded.skins[0].joints;
    assert_eq!(joints[0].name, "hips");
    assert_eq!(joints[0].parent, -1);
    assert_eq!(joints[1].name, "knee");
    assert_eq!(joints[1].parent, 0);
    assert_eq!(joints[0].inverse_bind[0], 1.0);
    assert_eq!(joints[0].ancestor, genos_load::IDENTITY_BIND);
    assert_eq!(joints[1].ancestor, genos_load::IDENTITY_BIND);
    assert_eq!(joints[1].inverse_bind[5], 1.0);

    let move_clip = decoded
        .clips
        .iter()
        .find(|clip| clip.name == "move")
        .unwrap();
    assert_eq!(move_clip.channels.len(), 1);
    assert_eq!(move_clip.channels[0].path, ChannelPath::Translation);
    assert_eq!(move_clip.channels[0].interpolation, Interpolation::Linear);
    let mut mid = [0.0; 3];
    assert!(sample_channel(&move_clip.channels[0], 0.5, &mut mid));
    assert_eq!(mid, [1.0, 1.0, 0.0]);

    let hold = decoded
        .clips
        .iter()
        .find(|clip| clip.name == "hold")
        .unwrap();
    let mut before = [0.0; 4];
    let mut at_key = [0.0; 4];
    assert!(sample_channel(&hold.channels[0], 0.999, &mut before));
    assert!(sample_channel(&hold.channels[0], 1.0, &mut at_key));
    assert_eq!(before, [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(at_key, [0.0, 1.0, 0.0, 0.0]);

    let primitive = &decoded.primitives[0];
    assert_eq!(primitive.joints[2], [0, 1, 0, 1]);
    assert_eq!(primitive.weights[2], [0.5, 0.25, 0.125, 0.125]);
    assert!(decoded
        .notes
        .iter()
        .any(|note| note.contains("four largest")));
    assert!(decoded.notes.iter().all(|note| !note.contains("morph")));
    let record = load_cooked(&cooked.record.1).unwrap();
    assert!(record.unsupported.iter().any(|note| note.contains("morph")));

    let skin_hash = sha256(&bytes);
    let text = package_files(&[PackageFile {
        cooked: &record,
        gtex: None,
        skin: Some(skin_hash),
    }])
    .text();
    assert!(text.contains(" skin="));
    std::fs::write(out.join("package.manifest"), &text).unwrap();
    Manifest::parse(&text).unwrap().verify(&out).unwrap();

    let clip_mb = f64::from(u32::try_from(decoded.clip_bytes).unwrap_or(u32::MAX))
        / f64::from(1024u32 * 1024);
    eprintln!("anim.clip_mb {clip_mb}");

    let mut broken = bytes;
    let last = broken.len() - 1;
    broken[last] ^= 0xff;
    std::fs::write(&skin_path, &broken).unwrap();
    assert!(decode_skin(&broken).is_err());
    assert!(Manifest::parse(&text).unwrap().verify(&out).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_rigid_source_has_no_gskin() {
    let dir = temp("rigid");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/fixtures/cube.gltf");
    let out = dir.join("out");
    let cooked = cook_asset(&fixture, &out, &settings(&dir), false, Threads::ONE).unwrap();
    assert!(cooked.skin.is_none());
    let any_skin = std::fs::read_dir(out.join("cooked"))
        .unwrap()
        .flatten()
        .any(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("gskin"));
    assert!(!any_skin);
    let again = cook_asset(&fixture, &out, &settings(&dir), false, Threads::ONE).unwrap();
    assert!(again.skin.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
