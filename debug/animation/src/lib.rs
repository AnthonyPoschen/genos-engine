//! Import dump for `genos-animation`. Headless; no window and no device.

use genos_json::{array, float, int, object, string, Value};
use genos_load::{
    decode_skin, import_bytes, ChannelPath, Clip, CookedError, DecodedSkin, ImportDoc,
    Interpolation, LoadError, Skin,
};

/// One JSON object: skins, clips, influence counts, and the notes the import wrote.
///
/// # Errors
///
/// A file the glTF decoder refuses is [`LoadError`].
pub fn dump_bytes(bytes: &[u8]) -> Result<Value, LoadError> {
    let doc = import_bytes(bytes, &|_uri| Err(LoadError::Unrecognized))?;
    Ok(dump(&doc))
}

/// The Low settings row, then the 32×60×52,356 byte total.
#[must_use]
pub fn settings_text() -> String {
    let settings = genos_scene::AnimationSettings::preset(genos_scene::AnimQuality::Low);
    let bytes = genos_scene::AnimationSettings::skin_bytes(32, 60, 52_356);
    format!("{}\nskin_bytes={bytes}", settings.to_text())
}

/// Apply one `key=value` knob to the Low row and return the text.
///
/// # Errors
///
/// A line that is not `key=value`, or a key the table does not know.
pub fn apply_setting(line: &str) -> Result<String, String> {
    let mut settings = genos_scene::AnimationSettings::preset(genos_scene::AnimQuality::Low);
    let Some((key, value)) = line.split_once('=') else {
        return Err(format!("use key=value, got {line}"));
    };
    settings.set_key(key, value)?;
    Ok(settings.to_text())
}

/// Body X after a one-metre root translation over 60 ticks. The pose root stays at rest.
#[must_use]
pub fn replay_root_x() -> f32 {
    use genos_load::{Channel, ChannelPath, Clip, Interpolation};
    use genos_math::Vec3;
    use genos_physics::Body;
    use genos_scene::{
        tick, AnimCharacter, AnimFrame, AnimGraph, AnimQuality, AnimationSettings, PoseSource,
        SIM_HZ,
    };
    let clip = Clip {
        name: "move".into(),
        duration: 1.0,
        skin: 0,
        animation: 0,
        channels: vec![Channel {
            joint: 0,
            path: ChannelPath::Translation,
            interpolation: Interpolation::Linear,
            times: vec![0.0, 1.0],
            values: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        }],
    };
    let rest_t = [[0.0, 0.0, 0.0]];
    let rest_r = [[0.0, 0.0, 0.0, 1.0]];
    let rest_s = [[1.0, 1.0, 1.0]];
    let Ok(graph) = AnimGraph::parse(r#"{"states":[{"name":"move","clip":0}]}"#) else {
        return 0.0;
    };
    let posed = PoseSource {
        clips: std::slice::from_ref(&clip),
        rest_t: &rest_t,
        rest_r: &rest_r,
        rest_s: &rest_s,
    };
    let mut character = AnimCharacter::bind(1, AnimationSettings::preset(AnimQuality::Low));
    let mut body = Body::sphere(Vec3::ZERO, 1.0, 0.5);
    tick(
        &graph,
        &posed,
        &mut character,
        AnimFrame {
            tick: 0,
            distance_m: 0.0,
        },
        Some(&mut body),
        &mut [],
    );
    tick(
        &graph,
        &posed,
        &mut character,
        AnimFrame {
            tick: SIM_HZ,
            distance_m: 0.0,
        },
        Some(&mut body),
        &mut [],
    );
    body.position.x
}

/// One JSON object for a `.gskin`: joints, clips, instance indices, and notes.
///
/// # Errors
///
/// A bad magic, version, or trailer is [`CookedError`].
pub fn dump_gskin(bytes: &[u8]) -> Result<Value, CookedError> {
    Ok(gskin_value(&decode_skin(bytes)?))
}

fn gskin_value(skin: &DecodedSkin) -> Value {
    let influences: usize = skin
        .primitives
        .iter()
        .map(|primitive| primitive.joints.len())
        .sum();
    object([
        ("skins", array(skin.skins.iter().map(skin_value).collect())),
        ("clips", array(skin.clips.iter().map(clip_value).collect())),
        (
            "instances",
            array(
                skin.instance_skins
                    .iter()
                    .map(|index| int(i64::from(*index)))
                    .collect(),
            ),
        ),
        (
            "influences",
            int(i64::try_from(influences).unwrap_or(i64::MAX)),
        ),
        (
            "notes",
            array(skin.notes.iter().cloned().map(string).collect()),
        ),
        (
            "clip_bytes",
            int(i64::try_from(skin.clip_bytes).unwrap_or(i64::MAX)),
        ),
    ])
}

/// The JSON object [`dump_bytes`] prints, from an import that already resolved URIs.
#[must_use]
pub fn dump(doc: &ImportDoc) -> Value {
    let Some(scene) = doc.scene.as_ref() else {
        return object([
            ("skins", array(Vec::new())),
            ("clips", array(Vec::new())),
            ("influences", int(0)),
            ("instances", array(Vec::new())),
            ("skin_notes", array(Vec::new())),
            ("notes", notes_of(doc)),
        ]);
    };
    let influences: usize = scene
        .meshes
        .iter()
        .flat_map(|mesh| mesh.primitives.iter())
        .map(|primitive| primitive.joints.len())
        .sum();
    object([
        ("skins", array(scene.skins.iter().map(skin_value).collect())),
        ("clips", array(scene.clips.iter().map(clip_value).collect())),
        (
            "influences",
            int(i64::try_from(influences).unwrap_or(i64::MAX)),
        ),
        (
            "instances",
            array(
                scene
                    .instances
                    .iter()
                    .map(|instance| {
                        object([
                            (
                                "skin",
                                instance.skin.map_or(Value::Null, |index| {
                                    int(i64::try_from(index).unwrap_or(i64::MAX))
                                }),
                            ),
                            (
                                "node",
                                instance.node_name.clone().map_or(Value::Null, string),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "skin_notes",
            array(scene.skin_notes.iter().cloned().map(string).collect()),
        ),
        ("notes", notes_of(doc)),
    ])
}

fn notes_of(doc: &ImportDoc) -> Value {
    array(
        doc.notes
            .iter()
            .map(|note| string(note.to_string()))
            .collect(),
    )
}

fn skin_value(skin: &Skin) -> Value {
    object([
        ("name", skin.name.clone().map_or(Value::Null, string)),
        (
            "joints",
            array(
                skin.joints
                    .iter()
                    .map(|joint| {
                        object([
                            ("name", string(joint.name.clone())),
                            ("parent", int(i64::from(joint.parent))),
                            ("node", int(i64::try_from(joint.node).unwrap_or(i64::MAX))),
                            ("translation", floats(&joint.translation)),
                            ("rotation", floats(&joint.rotation)),
                            ("scale", floats(&joint.scale)),
                            ("inverse_bind", floats(&joint.inverse_bind)),
                            ("ancestor", floats(&joint.ancestor)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "notes",
            array(skin.notes.iter().cloned().map(string).collect()),
        ),
    ])
}

fn clip_value(clip: &Clip) -> Value {
    object([
        ("name", string(clip.name.clone())),
        ("duration", float(f64::from(clip.duration))),
        ("skin", int(i64::try_from(clip.skin).unwrap_or(i64::MAX))),
        (
            "animation",
            int(i64::try_from(clip.animation).unwrap_or(i64::MAX)),
        ),
        (
            "channels",
            array(
                clip.channels
                    .iter()
                    .map(|channel| {
                        object([
                            ("joint", int(i64::from(channel.joint))),
                            ("path", string(path_name(channel.path))),
                            ("interpolation", string(interp_name(channel.interpolation))),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn floats(values: &[f32]) -> Value {
    array(
        values
            .iter()
            .copied()
            .map(|value| float(f64::from(value)))
            .collect(),
    )
}

fn path_name(path: ChannelPath) -> &'static str {
    match path {
        ChannelPath::Translation => "translation",
        ChannelPath::Rotation => "rotation",
        ChannelPath::Scale => "scale",
    }
}

fn interp_name(interpolation: Interpolation) -> &'static str {
    match interpolation {
        Interpolation::Linear => "linear",
        Interpolation::Step => "step",
    }
}

#[cfg(test)]
#[expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]
mod tests {
    use super::{dump_bytes, dump_gskin};

    const FIXTURE: &str = r#"{
        "asset": {"version": "2.0"},
        "nodes": [
            {"name": "hips"},
            {"name": "knee"}
        ],
        "skins": [{"joints": [0, 1], "skeleton": 0}]
    }"#;

    #[test]
    fn a_skin_dump_names_the_joints_and_the_missing_bind() {
        let value = dump_bytes(FIXTURE.as_bytes()).expect("dump");
        let text = genos_json::encode(&value);
        assert!(text.contains("\"name\":\"hips\""));
        assert!(text.contains("\"parent\":-1"));
        assert!(text.contains("inverse bind missing"));
        assert!(text.contains("skins[0] skin: used"));
    }

    #[test]
    fn a_gskin_dump_names_the_parent_and_the_clip_bytes() {
        use genos_load::{
            encode_skin, Channel, ChannelPath, Clip, Interpolation, Joint, Skin, SkinCook,
            SkinPrimitive, IDENTITY_BIND,
        };
        let cook = SkinCook {
            asset_id: [1; 32],
            settings: [2; 32],
            inputs: vec![(String::new(), [1; 32])],
            skins: vec![Skin {
                name: Some("body".into()),
                joints: vec![Joint {
                    name: "hips".into(),
                    parent: -1,
                    inverse_bind: IDENTITY_BIND,
                    translation: [0.0, 1.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                    ancestor: IDENTITY_BIND,
                    node: 4,
                }],
                notes: Vec::new(),
            }],
            clips: vec![Clip {
                name: "move".into(),
                duration: 1.0,
                skin: 0,
                animation: 0,
                channels: vec![Channel {
                    joint: 0,
                    path: ChannelPath::Translation,
                    interpolation: Interpolation::Linear,
                    times: vec![0.0],
                    values: vec![0.0, 0.0, 0.0],
                }],
            }],
            instance_skins: vec![Some(0)],
            mesh_skin: vec![Some(0)],
            primitives: vec![SkinPrimitive {
                joints: vec![[0, 0, 0, 0]],
                weights: vec![[1.0, 0.0, 0.0, 0.0]],
            }],
            primitive_mesh: vec![0],
            notes: Vec::new(),
        };
        let encoded = encode_skin(&cook).expect("encode");
        let text = genos_json::encode(&dump_gskin(&encoded.bytes).expect("dump"));
        assert!(text.contains("\"name\":\"hips\""));
        assert!(text.contains("\"parent\":-1"));
        assert!(text.contains("\"clip_bytes\":"));
        assert!(text.contains("linear"));
    }

    #[test]
    fn settings_text_names_the_low_row_and_replay_moves_one_metre() {
        let text = super::settings_text();
        assert!(text.contains("max_skinned=32"));
        assert!(text.contains("skin_bytes=67107840"));
        let edited = super::apply_setting("ik=on").expect("set");
        assert!(edited.contains("ik=on"));
        assert!(super::apply_setting("nope").is_err());
        let moved = super::replay_root_x();
        assert!((moved - 1.0).abs() < 1.0e-4);
    }
}
