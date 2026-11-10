#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! glTF skins, joint influences, and skeletal clips.

use genos_load::{
    load_gltf, sample_channel, ChannelPath, Interpolation, MemorySource, Support, IDENTITY_BIND,
};

fn no_files(_: &str) -> Result<Vec<u8>, genos_load::LoadError> {
    Err(genos_load::LoadError::Unrecognized)
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
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let total = u32::try_from(total).unwrap();
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
                        "POSITION": 0,
                        "NORMAL": 1,
                        "JOINTS_0": 2,
                        "WEIGHTS_0": 3,
                        "JOINTS_1": 4,
                        "WEIGHTS_1": 5
                    }},
                    "indices": 6,
                    "targets": [{{"POSITION": 0}}]
                }}]
            }}],
            "animations": [
                {{
                    "name": "move",
                    "samplers": [{{"input": 8, "output": 9, "interpolation": "LINEAR"}}],
                    "channels": [
                        {{"sampler": 0, "target": {{"node": 0, "path": "translation"}}}},
                        {{"sampler": 0, "target": {{"node": 0, "path": "weights"}}}},
                        {{"sampler": 0, "target": {{"node": 0, "path": "translation"}}, "interpolation": "ignored"}},
                        {{"sampler": 0, "target": {{"node": 2, "path": "translation"}}}}
                    ]
                }},
                {{
                    "name": "hold",
                    "samplers": [{{"input": 10, "output": 11, "interpolation": "STEP"}}],
                    "channels": [
                        {{"sampler": 0, "target": {{"node": 1, "path": "rotation"}}}}
                    ]
                }}
            ],
            "buffers": [{{"byteLength": {byte_length}}}],
            "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": {byte_length}}}],
            "accessors": [{accessors}]
        }}"#,
        accessors = acc.join(","),
    );
    let json = json.replacen(
        r#""samplers": [{"input": 8, "output": 9, "interpolation": "LINEAR"}]"#,
        r#""samplers": [{"input": 8, "output": 9, "interpolation": "LINEAR"}, {"input": 8, "output": 9, "interpolation": "CUBICSPLINE"}]"#,
        1,
    );
    let json = json.replacen(
        r#"{"sampler": 0, "target": {"node": 0, "path": "translation"}, "interpolation": "ignored"}"#,
        r#"{"sampler": 1, "target": {"node": 0, "path": "translation"}}"#,
        1,
    );
    glb(&json, &bin.0)
}

#[test]
fn a_skin_keeps_joints_influences_and_linear_and_step_clips() {
    let scene = load_gltf(&MemorySource::new(&skinned_glb()), &no_files).unwrap();
    assert_eq!(scene.skins.len(), 1);
    assert_eq!(scene.skins[0].name.as_deref(), Some("body"));
    let joints = &scene.skins[0].joints;
    assert_eq!(joints.len(), 2);
    assert_eq!(joints[0].name, "hips");
    assert_eq!(joints[0].parent, -1);
    assert_eq!(joints[0].translation, [0.0, 1.0, 0.0]);
    assert_eq!(joints[0].inverse_bind, IDENTITY_BIND);
    assert_eq!(joints[1].name, "knee");
    assert_eq!(joints[1].parent, 0);
    assert_eq!(joints[1].node, 1);
    assert_eq!(scene.instances.len(), 1);
    assert_eq!(scene.instances[0].skin, Some(0));

    let primitive = &scene.meshes[0].primitives[0];
    assert_eq!(primitive.joints.len(), 4);
    assert_eq!(primitive.joints[0], [0, 1, 0, 0]);
    assert_eq!(primitive.weights[0], [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(primitive.weights[1], [0.25, 0.75, 0.0, 0.0]);
    assert_eq!(primitive.joints[2], [0, 1, 0, 1]);
    assert_eq!(primitive.weights[2], [0.5, 0.25, 0.125, 0.125]);
    assert_eq!(primitive.weights[3], [1.0, 0.0, 0.0, 0.0]);
    assert!(scene
        .skin_notes
        .iter()
        .any(|note| note.contains("four largest")));
    assert!(scene
        .skin_notes
        .iter()
        .any(|note| note.contains("zero weight sum")));
    assert!(scene
        .skin_notes
        .iter()
        .any(|note| note.contains("omitted path weights")));
    assert!(scene
        .skin_notes
        .iter()
        .any(|note| note.contains("CUBICSPLINE")));
    assert!(scene
        .skin_notes
        .iter()
        .any(|note| note.contains("not in a skin")));

    let move_clip = scene.clips.iter().find(|clip| clip.name == "move").unwrap();
    assert_eq!(move_clip.channels.len(), 1);
    assert_eq!(move_clip.channels[0].path, ChannelPath::Translation);
    assert_eq!(move_clip.channels[0].interpolation, Interpolation::Linear);
    let mut mid = [0.0; 3];
    assert!(sample_channel(&move_clip.channels[0], 0.5, &mut mid));
    assert_eq!(mid, [1.0, 1.0, 0.0]);

    let hold = scene.clips.iter().find(|clip| clip.name == "hold").unwrap();
    assert_eq!(hold.channels[0].interpolation, Interpolation::Step);
    let mut before = [0.0; 4];
    let mut at_key = [0.0; 4];
    assert!(sample_channel(&hold.channels[0], 0.999, &mut before));
    assert!(sample_channel(&hold.channels[0], 1.0, &mut at_key));
    assert_eq!(before, [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(at_key, [0.0, 1.0, 0.0, 0.0]);
}

#[test]
fn import_marks_the_skin_used_and_the_morph_target_unsupported() {
    let doc = genos_load::import_bytes(&skinned_glb(), &no_files).unwrap();
    assert!(doc
        .notes
        .iter()
        .any(|note| note.feature == "skin" && note.support == Support::Used));
    assert!(doc
        .notes
        .iter()
        .any(|note| note.feature == "animation" && note.support == Support::Used));
    assert!(doc
        .notes
        .iter()
        .any(|note| note.feature == "morph target" && note.support == Support::LoadedUnsupported));
    assert!(!doc
        .notes
        .iter()
        .any(|note| note.feature == "skin" && note.support == Support::LoadedUnsupported));
}

#[test]
fn a_matrix_joint_keeps_its_translation_and_scale() {
    let json = r#"{
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "box", "matrix": [2,0,0,0, 0,2,0,0, 0,0,2,0, 1,2,3,1]}],
        "skins": [{"joints": [0]}]
    }"#;
    let scene = load_gltf(&MemorySource::new(json.as_bytes()), &no_files).unwrap();
    let joint = &scene.skins[0].joints[0];
    assert!((joint.translation[0] - 1.0).abs() < 1.0e-5);
    assert!((joint.translation[1] - 2.0).abs() < 1.0e-5);
    assert!((joint.translation[2] - 3.0).abs() < 1.0e-5);
    assert!((joint.scale[0] - 2.0).abs() < 1.0e-5);
    assert!((joint.scale[1] - 2.0).abs() < 1.0e-5);
    assert!((joint.scale[2] - 2.0).abs() < 1.0e-5);
    assert!((joint.rotation[3] - 1.0).abs() < 1.0e-5);
    assert!(joint.rotation[0].abs() < 1.0e-5);
}
