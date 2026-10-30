//! The offline cook (ADR 0020, docs/systems/assets/plan.md step 7).
//!
//! `cook` turns an [`ImportDoc`] into a [`Cooked`] record: the scene triangles
//! and instances, plus the existing `.gbake` key per mesh from
//! `genos_bake::load_or_bake`. The cook builds no SDF of its own.
//!
//! The record is a `.gcook` file named by the source `AssetId`. Its header lists
//! every input hash and the settings hash, so an unchanged input is skipped
//! without decoding it. A SHA-256 trailer covers the whole file, so a load that
//! returns a record has checked it. The runtime still loads source files; the
//! cooked path is opt-in and does not change a draw.

mod format;

use std::path::{Path, PathBuf};

use genos_bake::{load_or_bake, BakeMesh, BakeSettings};
use genos_load::{hex, sha256, ByteSource, GltfMesh, ImportDoc, LoadError, MeshInstance, Sha256};

pub use format::{decode, encode, Input, FORMAT_VERSION, MAGIC};

/// One cooked mesh: the glTF primitives as imported and the bake key.
#[derive(Clone, Debug, PartialEq)]
pub struct CookedMesh {
    pub mesh: GltfMesh,
    /// `BakeMesh::content_key` for the cook's bake settings.
    pub bake_key: u64,
}

/// A cooked source file.
#[derive(Clone, Debug, PartialEq)]
pub struct Cooked {
    pub asset_id: [u8; 32],
    /// [`settings_hash`] of the settings this record was cooked with.
    pub settings: [u8; 32],
    /// Every file the cook read, the source first.
    pub inputs: Vec<Input>,
    pub meshes: Vec<CookedMesh>,
    pub instances: Vec<MeshInstance>,
    /// Import notes the runtime does not use, one line each.
    pub unsupported: Vec<String>,
}

/// Why a cook or a cooked load failed.
#[derive(Debug)]
pub enum CookError {
    Load(LoadError),
    /// The import kept the file and has no drawable scene (an unsupported required extension).
    NoScene,
    Bake(genos_bake::BakeError),
    Io(std::io::Error),
    /// The `.gcook` bytes are not a record of this version, or its hash does not match.
    Corrupt(&'static str),
}

impl std::fmt::Display for CookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Load(e) => write!(f, "import: {e}"),
            Self::NoScene => f.write_str("import has no drawable scene"),
            Self::Bake(e) => write!(f, "bake: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Corrupt(why) => write!(f, "corrupt .gcook: {why}"),
        }
    }
}

impl std::error::Error for CookError {}

impl From<LoadError> for CookError {
    fn from(e: LoadError) -> Self {
        Self::Load(e)
    }
}

impl From<std::io::Error> for CookError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// SHA-256 of the format version and every bake setting that changes the result.
/// The cache mode and path are not part of it.
#[must_use]
pub fn settings_hash(s: &BakeSettings) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(&FORMAT_VERSION.to_le_bytes());
    for v in [s.voxels_per_side, s.voxel_min, s.voxel_max, s.card_texel] {
        h.update(&v.to_bits().to_le_bytes());
    }
    h.update(&s.card_max_texels.to_le_bytes());
    h.finish()
}

/// Cook an import. Each mesh goes through `load_or_bake`, so the bake cache is filled or reused.
///
/// # Errors
///
/// No scene, or a bake that fails, is an error.
pub fn cook(doc: &ImportDoc, settings: &BakeSettings) -> Result<Cooked, CookError> {
    let scene = doc.scene.as_ref().ok_or(CookError::NoScene)?;
    let mut meshes = Vec::with_capacity(scene.meshes.len());
    for (index, mesh) in scene.meshes.iter().enumerate() {
        let baked =
            load_or_bake(&BakeMesh::from_gltf(scene, index), settings).map_err(CookError::Bake)?;
        meshes.push(CookedMesh {
            mesh: mesh.clone(),
            bake_key: baked.key,
        });
    }
    let mut inputs = vec![Input {
        uri: String::new(),
        hash: doc.asset_id,
    }];
    inputs.extend(doc.externals.iter().map(|(uri, bytes)| Input {
        uri: uri.clone(),
        hash: sha256(bytes),
    }));
    Ok(Cooked {
        asset_id: doc.asset_id,
        settings: settings_hash(settings),
        inputs,
        meshes,
        instances: scene.instances.clone(),
        unsupported: doc
            .notes
            .iter()
            .filter(|n| n.support == genos_load::Support::LoadedUnsupported)
            .map(ToString::to_string)
            .collect(),
    })
}

/// Where the record for a source lives under a cook directory.
#[must_use]
pub fn cooked_path(out: &Path, asset_id: &[u8; 32]) -> PathBuf {
    out.join("cooked").join(format!("{}.gcook", hex(asset_id)))
}

/// What [`cook_file`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Cooked,
    UpToDate,
}

/// Cook a glTF file into `out`, unless the record there already matches every
/// input hash and the settings. The up-to-date check reads and hashes the
/// inputs; it does not decode them.
///
/// # Errors
///
/// A source that does not import, a failed bake, or a failed write.
pub fn cook_file(
    source: &Path,
    out: &Path,
    settings: &BakeSettings,
) -> Result<(Outcome, PathBuf), CookError> {
    let bytes = std::fs::read(source).map_err(|_| LoadError::NotFound {
        path: source.to_path_buf(),
    })?;
    let dir = source.parent().unwrap_or(Path::new("."));
    let path = cooked_path(out, &sha256(&bytes));
    if let Ok(existing) = std::fs::read(&path) {
        if up_to_date(&existing, &bytes, dir, settings) {
            return Ok((Outcome::UpToDate, path));
        }
    }
    let doc = genos_load::import_bytes(&bytes, &|uri| {
        genos_load::FileSource::new(dir.join(uri)).read_bytes()
    })?;
    let cooked = cook(&doc, settings)?;
    genos_load::store_import(out, &doc)?;
    genos_load::write_atomic(&path, &encode(&cooked))?;
    Ok((Outcome::Cooked, path))
}

fn up_to_date(existing: &[u8], source: &[u8], dir: &Path, settings: &BakeSettings) -> bool {
    let Ok(header) = format::decode_header(existing) else {
        return false;
    };
    header.settings == settings_hash(settings)
        && header.inputs.iter().all(|input| {
            let bytes = if input.uri.is_empty() {
                Some(source.to_vec())
            } else {
                std::fs::read(dir.join(&input.uri)).ok()
            };
            bytes.is_some_and(|b| sha256(&b) == input.hash)
        })
}

/// Read and check a `.gcook`: magic, version and the SHA-256 trailer.
///
/// # Errors
///
/// A read failure or a record that does not check.
pub fn load_cooked(path: &Path) -> Result<Cooked, CookError> {
    decode(&std::fs::read(path)?)
}
