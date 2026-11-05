//! Runtime read of a cooked package (ADR 0035).
//!
//! The cook (`genos-cook`) writes the bytes. This module checks the manifest
//! and decodes `.gcook` geometry and `.gtex` block payloads. It does not
//! encode, simplify, or transcode.

use std::path::{Path, PathBuf};

use crate::{hex, object_path, sha256, GltfMesh, MeshInstance, Primitive};

const GCOOK_MAGIC: &[u8; 4] = b"GCK1";
const GCOOK_VERSION: u32 = 1;
const GTEX_MAGIC: &[u8; 4] = b"GTX1";
const GTEX_VERSION: u32 = 1;
const NONE: u32 = u32::MAX;
const MANIFEST_HEADER: &str = "genos-package 1";
const KTX_ID: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
const KTX_HEADER: usize = 12 + 9 * 4 + 4 * 4 + 2 * 8;
const KTX_LEVEL: usize = 3 * 8;

/// Why a cooked package could not be used.
#[derive(Debug)]
pub enum CookedError {
    Io(std::io::Error),
    /// Magic, version, length, or a hash did not match.
    Corrupt(&'static str),
}

impl std::fmt::Display for CookedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Corrupt(why) => write!(f, "corrupt package: {why}"),
        }
    }
}

impl std::error::Error for CookedError {}

impl From<std::io::Error> for CookedError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Geometry and instances from one `.gcook`. Materials stay in the source.
#[derive(Clone, Debug, PartialEq)]
pub struct CookedGeometry {
    pub asset_id: [u8; 32],
    pub meshes: Vec<GltfMesh>,
    pub instances: Vec<MeshInstance>,
}

/// One cooked image: KTX2 blocks, finest mip first. `format` is a `VkFormat`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookedImage {
    pub image: u32,
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
}

/// Textures from one `.gtex`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookedImages {
    pub asset_id: [u8; 32],
    pub images: Vec<CookedImage>,
}

/// `DIR/package.manifest`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub lines: Vec<String>,
}

impl Package {
    ///
    /// # Errors
    ///
    /// A missing header is corrupt.
    pub fn parse(text: &str) -> Result<Self, CookedError> {
        let mut lines = text.lines();
        if lines.next() != Some(MANIFEST_HEADER) {
            return Err(CookedError::Corrupt("manifest header"));
        }
        Ok(Self {
            lines: lines
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
        })
    }

    /// Read `dir/package.manifest`.
    ///
    /// # Errors
    ///
    /// A missing file or a bad header.
    pub fn open(dir: &Path) -> Result<Self, CookedError> {
        let text = std::fs::read_to_string(dir.join("package.manifest"))?;
        Self::parse(&text)
    }

    /// Every named record, texture record, and source object is present and
    /// hashes to the manifest. Returns the asset count.
    ///
    /// # Errors
    ///
    /// A missing file is I/O. A hash that differs is corrupt.
    pub fn verify(&self, dir: &Path) -> Result<usize, CookedError> {
        for line in &self.lines {
            let mut parts = line.split(' ');
            let asset = parts.next().ok_or(CookedError::Corrupt("manifest line"))?;
            let cook = parts
                .next()
                .and_then(|p| p.strip_prefix("cook="))
                .ok_or(CookedError::Corrupt("manifest line"))?;
            let inputs = parts
                .next()
                .and_then(|p| p.strip_prefix("inputs="))
                .ok_or(CookedError::Corrupt("manifest line"))?;
            let id = unhex(asset)?;
            if hex(&sha256(&std::fs::read(cooked_path(dir, &id))?)) != cook {
                return Err(CookedError::Corrupt(
                    "record hash differs from the manifest",
                ));
            }
            if let Some(gtex) = parts.next() {
                let gtex = gtex
                    .strip_prefix("gtex=")
                    .ok_or(CookedError::Corrupt("manifest line"))?;
                if hex(&sha256(&std::fs::read(textures_path(dir, &id))?)) != gtex {
                    return Err(CookedError::Corrupt(
                        "texture hash differs from the manifest",
                    ));
                }
            }
            for input in inputs.split(',') {
                let bytes = std::fs::read(object_path(dir, &unhex(input)?))?;
                if hex(&sha256(&bytes)) != input {
                    return Err(CookedError::Corrupt("object hash differs from its name"));
                }
            }
        }
        Ok(self.lines.len())
    }

    /// The manifest line whose asset id is `id`, if the package lists it.
    #[must_use]
    pub fn line_for(&self, id: &[u8; 32]) -> Option<&str> {
        let key = hex(id);
        self.lines
            .iter()
            .find(|l| l.starts_with(&key))
            .map(String::as_str)
    }
}

#[must_use]
pub fn cooked_path(dir: &Path, asset_id: &[u8; 32]) -> PathBuf {
    dir.join("cooked").join(format!("{}.gcook", hex(asset_id)))
}

#[must_use]
pub fn textures_path(dir: &Path, asset_id: &[u8; 32]) -> PathBuf {
    dir.join("cooked").join(format!("{}.gtex", hex(asset_id)))
}

/// Decode a `.gcook` after checking its trailer.
///
/// # Errors
///
/// A bad magic, version, length, or trailer.
pub fn decode_geometry(bytes: &[u8]) -> Result<CookedGeometry, CookedError> {
    let body = checked_body(bytes)?;
    let mut r = R { b: body, at: 0 };
    if r.take(4)? != GCOOK_MAGIC {
        return Err(CookedError::Corrupt("gcook magic"));
    }
    if r.u32()? != GCOOK_VERSION {
        return Err(CookedError::Corrupt("gcook version"));
    }
    let asset_id = r.array()?;
    let _settings = r.array::<32>()?;
    for _ in 0..r.len()? {
        let _uri = r.str()?;
        let _hash = r.array::<32>()?;
    }
    let mut meshes = Vec::new();
    for _ in 0..r.len()? {
        let _bake_key = r.array::<8>()?;
        let name = r.opt_str()?;
        let mut primitives = Vec::new();
        for _ in 0..r.len()? {
            let material = r.u32()?;
            let positions = r.vecs::<3>()?;
            let normals = r.vecs::<3>()?;
            let texcoords = r.vecs::<2>()?;
            let mut texcoord_sets = Vec::new();
            for _ in 0..r.len()? {
                texcoord_sets.push(r.vecs::<2>()?);
            }
            let tangents = r.vecs::<4>()?;
            let mut indices = Vec::new();
            for _ in 0..r.len()? {
                indices.push(r.u32()?);
            }
            primitives.push(Primitive {
                positions,
                normals,
                texcoords,
                texcoord_sets,
                tangents,
                indices,
                material: (material != NONE)
                    .then(|| usize::try_from(material).ok())
                    .flatten(),
            });
        }
        meshes.push(GltfMesh { name, primitives });
    }
    let mut instances = Vec::new();
    for _ in 0..r.len()? {
        let mesh = r.len()?;
        let node_name = r.opt_str()?;
        let mut transform = [0.0; 16];
        for v in &mut transform {
            *v = f32::from_bits(r.u32()?);
        }
        instances.push(MeshInstance {
            mesh,
            node_name,
            transform,
        });
    }
    let notes = r.len()?;
    for _ in 0..notes {
        let _ = r.str()?;
    }
    if r.at != body.len() {
        return Err(CookedError::Corrupt("gcook trailing"));
    }
    Ok(CookedGeometry {
        asset_id,
        meshes,
        instances,
    })
}

/// Decode a `.gtex` after checking its trailer.
///
/// # Errors
///
/// A bad magic, version, trailer, or KTX2.
pub fn decode_images(bytes: &[u8]) -> Result<CookedImages, CookedError> {
    let body = checked_body(bytes)?;
    let mut r = R { b: body, at: 0 };
    if r.take(4)? != GTEX_MAGIC {
        return Err(CookedError::Corrupt("gtex magic"));
    }
    if r.u32()? != GTEX_VERSION {
        return Err(CookedError::Corrupt("gtex version"));
    }
    let asset_id = r.array()?;
    let _settings = r.array::<32>()?;
    for _ in 0..r.len()? {
        let _uri = r.str()?;
        let _hash = r.array::<32>()?;
    }
    let mut images = Vec::new();
    for _ in 0..r.len()? {
        let image = r.u32()?;
        let n = r.len()?;
        let ktx = r.take(n)?;
        let ktx = read_ktx2(ktx)?;
        images.push(CookedImage {
            image,
            format: ktx.format,
            width: ktx.width,
            height: ktx.height,
            levels: ktx.levels,
        });
    }
    if r.at != body.len() {
        return Err(CookedError::Corrupt("gtex trailing"));
    }
    Ok(CookedImages { asset_id, images })
}

struct Ktx {
    format: u32,
    width: u32,
    height: u32,
    levels: Vec<Vec<u8>>,
}

fn read_ktx2(b: &[u8]) -> Result<Ktx, CookedError> {
    if b.get(..12) != Some(&KTX_ID[..]) {
        return Err(CookedError::Corrupt("ktx2 identifier"));
    }
    let format = word(b, 12)?;
    if word(b, 44)? != 0 {
        return Err(CookedError::Corrupt("ktx2 supercompression"));
    }
    let count = usize::try_from(word(b, 40)?.max(1)).unwrap_or(0);
    let mut levels = Vec::new();
    for i in 0..count {
        let at = KTX_HEADER + KTX_LEVEL * i;
        let offset = dword(b, at)?;
        let len = dword(b, at + 8)?;
        let end = offset
            .checked_add(len)
            .ok_or(CookedError::Corrupt("ktx2 level"))?;
        levels.push(
            b.get(offset..end)
                .ok_or(CookedError::Corrupt("ktx2 level"))?
                .to_vec(),
        );
    }
    Ok(Ktx {
        format,
        width: word(b, 20)?,
        height: word(b, 24)?,
        levels,
    })
}

fn word(b: &[u8], at: usize) -> Result<u32, CookedError> {
    b.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(CookedError::Corrupt("ktx2 truncated"))
}

fn dword(b: &[u8], at: usize) -> Result<usize, CookedError> {
    b.get(at..at + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .and_then(|v| usize::try_from(v).ok())
        .ok_or(CookedError::Corrupt("ktx2 truncated"))
}

fn checked_body(bytes: &[u8]) -> Result<&[u8], CookedError> {
    let split = bytes
        .len()
        .checked_sub(32)
        .ok_or(CookedError::Corrupt("short"))?;
    let (body, trailer) = bytes.split_at(split);
    if sha256(body) != trailer {
        return Err(CookedError::Corrupt("content hash mismatch"));
    }
    Ok(body)
}

fn unhex(s: &str) -> Result<[u8; 32], CookedError> {
    let mut out = [0u8; 32];
    if s.len() != 64 {
        return Err(CookedError::Corrupt("hash length"));
    }
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2).unwrap_or("zz"), 16)
            .map_err(|_| CookedError::Corrupt("hash digit"))?;
    }
    Ok(out)
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CookedError> {
        let end = self
            .at
            .checked_add(n)
            .ok_or(CookedError::Corrupt("short"))?;
        let s = self
            .b
            .get(self.at..end)
            .ok_or(CookedError::Corrupt("short"))?;
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, CookedError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| CookedError::Corrupt("short"))?,
        ))
    }
    fn len(&mut self) -> Result<usize, CookedError> {
        let n = u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| CookedError::Corrupt("short"))?,
        );
        usize::try_from(n).map_err(|_| CookedError::Corrupt("length"))
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], CookedError> {
        self.take(N)?
            .try_into()
            .map_err(|_| CookedError::Corrupt("short"))
    }
    fn str(&mut self) -> Result<String, CookedError> {
        let n = self.len()?;
        let s = self.take(n)?;
        String::from_utf8(s.to_vec()).map_err(|_| CookedError::Corrupt("utf8"))
    }
    fn opt_str(&mut self) -> Result<Option<String>, CookedError> {
        let some = self.take(1)?[0] != 0;
        let s = self.str()?;
        Ok(some.then_some(s))
    }
    fn vecs<const N: usize>(&mut self) -> Result<Vec<[f32; N]>, CookedError> {
        let n = self.len()?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let mut v = [0.0; N];
            for x in &mut v {
                *x = f32::from_bits(self.u32()?);
            }
            out.push(v);
        }
        Ok(out)
    }
}
