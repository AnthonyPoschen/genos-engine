//! Import-time data for GI v2: a sparse signed distance field and surface cache
//! cards per mesh, cached on disk by content hash.
//!
//! When the bake runs is a setting ([`BakeMode`]): ahead of time by the
//! `genos-bake` tool (prebuilt, the editor default) or at first load (cached on
//! the player's disk). See `docs/systems/gi-v2-design.md`, "Scene representation".

mod cards;
mod format;
mod mesh;
mod sdf;

use std::path::{Path, PathBuf};

pub use cards::{Card, CardTexel, Cards};
pub use mesh::{BakeMaterial, BakeMesh};
pub use sdf::{MeshSdf, SdfBrick, BRICK};

/// When SDFs and cards are built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeMode {
    /// Built ahead of time by `genos-bake`; a missing bake is an error the caller reports.
    Prebuilt,
    /// Built at first load when the cache has none, then cached.
    OnLoad,
}

impl BakeMode {
    /// `GENOS_BAKE=prebuilt|onload`; the default is `Prebuilt` (the editor's choice).
    pub fn from_env() -> BakeMode {
        match std::env::var("GENOS_BAKE").as_deref() {
            Ok("onload") | Ok("on-load") | Ok("load") => BakeMode::OnLoad,
            _ => BakeMode::Prebuilt,
        }
    }
}

/// What to build and where the cache lives.
#[derive(Clone, Debug, PartialEq)]
pub struct BakeSettings {
    pub mode: BakeMode,
    pub cache_dir: PathBuf,
    /// SDF voxel = largest side / this, clamped to `voxel_min..=voxel_max` (metres).
    pub voxels_per_side: f32,
    pub voxel_min: f32,
    pub voxel_max: f32,
    /// Card texel size in metres (fixed per mesh, never by camera distance).
    pub card_texel: f32,
    /// Largest card edge in texels.
    pub card_max_texels: u32,
}

impl Default for BakeSettings {
    fn default() -> Self {
        Self {
            mode: BakeMode::from_env(),
            cache_dir: std::env::var_os("GENOS_BAKE_CACHE")
                .map_or_else(|| PathBuf::from("target/bake-cache"), PathBuf::from),
            voxels_per_side: 64.0,
            voxel_min: 0.02,
            voxel_max: 0.25,
            card_texel: 0.06,
            card_max_texels: 512,
        }
    }
}

/// The baked data of one mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct Baked {
    pub key: u64,
    pub sdf: MeshSdf,
    pub cards: Cards,
}

/// Why no bake came back.
#[derive(Debug, PartialEq)]
pub enum BakeError {
    /// `Prebuilt` mode and the cache has no bake for this mesh: run `genos-bake`.
    Missing {
        path: PathBuf,
    },
    Io(String),
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { path } => {
                write!(f, "no prebuilt bake at {} (run genos-bake)", path.display())
            }
            Self::Io(e) => write!(f, "bake cache: {e}"),
        }
    }
}

impl std::error::Error for BakeError {}

/// Build the SDF and cards now, with no cache.
pub fn bake(mesh: &BakeMesh, settings: &BakeSettings) -> Baked {
    Baked {
        key: mesh.content_key(settings),
        sdf: sdf::build(mesh, settings),
        cards: cards::build(mesh, settings),
    }
}

/// The cache file for a mesh under these settings.
pub fn cache_path(mesh: &BakeMesh, settings: &BakeSettings) -> PathBuf {
    settings
        .cache_dir
        .join(format!("{:016x}.gbake", mesh.content_key(settings)))
}

/// Read the cached bake, or build and store it when the mode allows.
pub fn load_or_bake(mesh: &BakeMesh, settings: &BakeSettings) -> Result<Baked, BakeError> {
    let path = cache_path(mesh, settings);
    if let Some(baked) = read(&path) {
        return Ok(baked);
    }
    match settings.mode {
        BakeMode::Prebuilt => Err(BakeError::Missing { path }),
        BakeMode::OnLoad => {
            let baked = bake(mesh, settings);
            write(&path, &baked)?;
            Ok(baked)
        }
    }
}

/// Build and store, whatever the cache holds (the `genos-bake` tool).
pub fn rebake(mesh: &BakeMesh, settings: &BakeSettings) -> Result<(Baked, PathBuf), BakeError> {
    let path = cache_path(mesh, settings);
    let baked = bake(mesh, settings);
    write(&path, &baked)?;
    Ok((baked, path))
}

/// A stored bake, or None when absent, unreadable or of another format version.
pub fn read(path: &Path) -> Option<Baked> {
    format::decode(&std::fs::read(path).ok()?)
}

pub fn write(path: &Path, baked: &Baked) -> Result<(), BakeError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| BakeError::Io(e.to_string()))?;
    }
    // Write then rename, so a reader never sees half a file.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, format::encode(baked)).map_err(|e| BakeError::Io(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| BakeError::Io(e.to_string()))
}
