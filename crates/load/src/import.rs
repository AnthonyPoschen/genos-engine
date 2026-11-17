//! The import document: the source bytes, the runtime projection the glTF
//! decoder already returns, and one note per feature (docs/systems/assets/plan.md, step 1).
//!
//! `load_gltf` is unchanged and stays strict. The import keeps a file whose
//! required extension the draw cannot use, with `scene: None`, so a later cook
//! can read it without opening the source again.

use std::cell::RefCell;
use std::path::Path;

use genos_json::Value;

use crate::gltf::{decode_gltf, split_glb, SUPPORTED_EXTENSIONS};
use crate::{sha256, ByteSource, FileSource, GltfScene, LoadError, Sha256};

/// Whether the runtime reads a feature the import found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Used,
    LoadedUnsupported,
}

/// One line of `import_report`: where, what, and whether the runtime uses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub path: String,
    pub feature: String,
    pub support: Support,
}

impl std::fmt::Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self.support {
            Support::Used => "used",
            Support::LoadedUnsupported => "loaded, unsupported",
        };
        write!(f, "{} {}: {s}", self.path, self.feature)
    }
}

/// An imported source file.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportDoc {
    /// SHA-256 of [`Self::source`].
    pub asset_id: [u8; 32],
    pub source: Vec<u8>,
    /// Files the source names by relative URI (buffers, images), in first-read order.
    pub externals: Vec<(String, Vec<u8>)>,
    /// `None` when the file is readable and the draw cannot use it (an unsupported required extension).
    pub scene: Option<GltfScene>,
    pub notes: Vec<Note>,
    /// Raw JSON of each `LoadedUnsupported` glTF feature, keyed by its note path.
    pub kept: Vec<(String, String)>,
}

impl ImportDoc {
    /// SHA-256 over the source and every external file it read. A change in any
    /// input changes this hash; the cook keys on it.
    #[must_use]
    pub fn inputs_hash(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(&self.asset_id);
        for (uri, bytes) in &self.externals {
            h.update(&sha256(uri.as_bytes()));
            h.update(&sha256(bytes));
        }
        h.finish()
    }

    /// Count of notes the runtime does not use.
    #[must_use]
    pub fn unsupported(&self) -> usize {
        self.notes
            .iter()
            .filter(|n| n.support == Support::LoadedUnsupported)
            .count()
    }
}

/// Import a `.gltf` or `.glb`. `resolve` reads a relative URI, as for [`crate::load_gltf`].
pub fn import_bytes(
    bytes: &[u8],
    resolve: &dyn Fn(&str) -> Result<Vec<u8>, LoadError>,
) -> Result<ImportDoc, LoadError> {
    let externals = RefCell::new(Vec::<(String, Vec<u8>)>::new());
    let recording = |uri: &str| {
        let data = resolve(uri)?;
        let mut seen = externals.borrow_mut();
        if !seen.iter().any(|(u, _)| u == uri) {
            seen.push((uri.to_string(), data.clone()));
        }
        Ok(data)
    };
    let decoded = decode_gltf(bytes, &recording);
    let (json, _) = split_glb(bytes)?;
    let text = std::str::from_utf8(json).map_err(|_| LoadError::Unrecognized)?;
    let doc = genos_json::parse(text).map_err(|_| LoadError::Unrecognized)?;

    let mut notes = Vec::new();
    let mut kept = Vec::new();
    let mut unsupported = |path: String, feature: &str, raw: &Value| {
        kept.push((path.clone(), genos_json::encode(raw)));
        notes.push(Note {
            path,
            feature: feature.to_string(),
            support: Support::LoadedUnsupported,
        });
    };
    let mut blocked = false;
    for (i, ext) in list(&doc, "extensionsRequired").iter().enumerate() {
        let name = ext.as_str().unwrap_or("?");
        if !SUPPORTED_EXTENSIONS.contains(&name) {
            blocked = true;
            unsupported(format!("extensionsRequired[{i}]"), name, ext);
        }
    }
    for (key, feature) in [
        ("skins", "skin"),
        ("animations", "animation"),
        ("cameras", "camera"),
    ] {
        for (i, v) in list(&doc, key).iter().enumerate() {
            unsupported(format!("{key}[{i}]"), feature, v);
        }
    }
    for (m, mesh) in list(&doc, "meshes").iter().enumerate() {
        for (p, prim) in list(mesh, "primitives").iter().enumerate() {
            if let Some(targets) = prim.get("targets") {
                unsupported(
                    format!("meshes[{m}].primitives[{p}].targets"),
                    "morph target",
                    targets,
                );
            }
        }
    }

    let scene = match decoded {
        Ok(scene) => {
            for (key, feature, n) in [
                ("meshes", "mesh", scene.meshes.len()),
                ("materials", "material", scene.materials.len()),
                ("images", "image", scene.images.len()),
            ] {
                notes.extend((0..n).map(|i| Note {
                    path: format!("{key}[{i}]"),
                    feature: feature.to_string(),
                    support: Support::Used,
                }));
            }
            Some(scene)
        }
        Err(LoadError::Unrecognized) if blocked => None,
        Err(e) => return Err(e),
    };
    Ok(ImportDoc {
        asset_id: sha256(bytes),
        source: bytes.to_vec(),
        externals: externals.into_inner(),
        scene,
        notes,
        kept,
    })
}

/// Import a glTF file; relative URIs are read from the file's directory.
pub fn import_file(path: impl AsRef<Path>) -> Result<ImportDoc, LoadError> {
    let path = path.as_ref();
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let bytes = FileSource::new(path).read_bytes()?;
    import_bytes(&bytes, &|uri| FileSource::new(dir.join(uri)).read_bytes())
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).unwrap_or(&[])
}
