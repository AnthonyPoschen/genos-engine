//! Import dump for `genos-animation`. Headless; no window and no device.

use genos_json::{array, float, int, object, string, Value};
use genos_load::{import_bytes, ChannelPath, Clip, ImportDoc, Interpolation, LoadError, Skin};

/// One JSON object: skins, clips, influence counts, and the notes the import wrote.
///
/// # Errors
///
/// A file the glTF decoder refuses is [`LoadError`].
pub fn dump_bytes(bytes: &[u8]) -> Result<Value, LoadError> {
    let doc = import_bytes(bytes, &|_uri| Err(LoadError::Unrecognized))?;
    Ok(dump(&doc))
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
    use super::dump_bytes;

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
}
