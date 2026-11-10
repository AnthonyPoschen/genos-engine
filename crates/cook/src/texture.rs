//! Cooked textures, `.gtex` (the cook side of plan steps 10 and 11).
//!
//! Every image of a glTF source gets its full mip chain from
//! [`genos_load::mip_chain`], in the space its material slots give it
//! ([`genos_load::image_spaces`]). Colour (base colour, emissive) is BC7 sRGB,
//! other data is BC7 unorm and normals are BC5 (X and Y; Z is rebuilt). Each
//! image is one KTX2. The encoder is `intel_tex_2` (Intel ISPC Texture
//! Compressor, "basic" profile). Edges are padded to whole 4×4 blocks by
//! repeating the last row and column.
//!
//! The record: `GTX1`, version, asset id, settings hash, inputs, then each
//! texture (image index, KTX2 bytes), then a SHA-256 of every byte before it.
//! Its key and up-to-date rule are those of `.gcook`. Nothing at runtime reads
//! it yet.

use std::path::{Path, PathBuf};

use genos_load::{
    hex, image_spaces, mip_chain, sha256, ByteSource, Image, ImportDoc, LoadError, Mip, MipSpace,
    Sha256,
};
use intel_tex_2::{bc5, bc7, RgSurface, RgbaSurface};

use crate::format::{checked_body, R, W};
use crate::ktx2::{self, BlockFormat, Ktx2};
use crate::{inputs_match, inputs_of, map_by_cost, CookError, Input, Outcome, Threads};

pub const TEXTURE_MAGIC: &[u8; 4] = b"GTX1";
pub const TEXTURE_VERSION: u32 = 1;
const ENCODER: &str = "intel_tex_2 0.5 ispc bc7-basic bc5; genos_load mip_chain v1";

/// One cooked image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookedTexture {
    /// Index into the glTF images.
    pub image: u32,
    pub ktx2: Ktx2,
}

/// The cooked textures of a source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookedTextures {
    pub asset_id: [u8; 32],
    pub settings: [u8; 32],
    pub inputs: Vec<Input>,
    pub textures: Vec<CookedTexture>,
}

/// SHA-256 of the record version and the encoder and filter it was cooked with.
#[must_use]
pub fn texture_settings_hash() -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(TEXTURE_MAGIC);
    h.update(&TEXTURE_VERSION.to_le_bytes());
    h.update(ENCODER.as_bytes());
    h.finish()
}

fn format_for(space: MipSpace) -> BlockFormat {
    match space {
        MipSpace::Srgb => BlockFormat::Bc7Srgb,
        MipSpace::Linear => BlockFormat::Bc7Unorm,
        MipSpace::Normal => BlockFormat::Bc5Unorm,
    }
}

/// The level copied to whole blocks, `channels` per texel, last row and column repeated.
fn padded(mip: &Mip, channels: usize) -> (Vec<u8>, u32, u32) {
    let (w, h) = (mip.width.max(1), mip.height.max(1));
    let (pw, ph) = (w.div_ceil(4) * 4, h.div_ceil(4) * 4);
    let size = |v: u32| usize::try_from(v).unwrap_or(0);
    let (wu, hu, pwu, phu) = (size(w), size(h), size(pw), size(ph));
    let mut out = Vec::with_capacity(pwu * phu * channels);
    for y in 0..phu {
        let row = y.min(hu - 1);
        for x in 0..pwu {
            let at = (row * wu + x.min(wu - 1)) * 4;
            let texel = mip.pixels.get(at..at + 4).unwrap_or(&[0; 4]);
            out.extend_from_slice(&texel[..channels]);
        }
    }
    (out, pw, ph)
}

fn encode_level(mip: &Mip, format: BlockFormat, alpha: bool) -> Vec<u8> {
    if format == BlockFormat::Bc5Unorm {
        let (data, width, height) = padded(mip, 2);
        return bc5::compress_blocks(&RgSurface {
            data: &data,
            width,
            height,
            stride: width * 2,
        });
    }
    let (data, width, height) = padded(mip, 4);
    let settings = if alpha {
        bc7::alpha_basic_settings()
    } else {
        bc7::opaque_basic_settings()
    };
    bc7::compress_blocks(
        &settings,
        &RgbaSurface {
            data: &data,
            width,
            height,
            stride: width * 4,
        },
    )
}

/// Mips and blocks for one image in the given space.
#[must_use]
pub fn encode_image(image: &Image, space: MipSpace) -> Ktx2 {
    let format = format_for(space);
    let alpha = image.pixels.iter().skip(3).step_by(4).any(|&a| a != 255);
    Ktx2 {
        format,
        width: image.width,
        height: image.height,
        levels: mip_chain(image, space)
            .iter()
            .map(|mip| encode_level(mip, format, alpha))
            .collect(),
    }
}

/// Cook every image of an import on every core.
///
/// # Errors
///
/// An import with no scene.
pub fn cook_textures(doc: &ImportDoc) -> Result<CookedTextures, CookError> {
    cook_textures_with(doc, Threads::all())
}

/// [`cook_textures`] with the images spread over `threads`, largest first. The
/// record is the same for any thread count.
///
/// # Errors
///
/// An import with no scene.
pub fn cook_textures_with(doc: &ImportDoc, threads: Threads) -> Result<CookedTextures, CookError> {
    let scene = doc.scene.as_ref().ok_or(CookError::NoScene)?;
    let spaces = image_spaces(scene.images.len(), &scene.materials);
    let jobs: Vec<(usize, &Image, MipSpace)> = scene
        .images
        .iter()
        .zip(spaces)
        .enumerate()
        .map(|(i, (image, space))| (i, image, space))
        .collect();
    let textures = map_by_cost(
        &jobs,
        threads,
        |job| job.1.pixels.len(),
        |&(i, image, space)| CookedTexture {
            image: u32::try_from(i).unwrap_or(u32::MAX),
            ktx2: encode_image(image, space),
        },
    );
    Ok(CookedTextures {
        asset_id: doc.asset_id,
        settings: texture_settings_hash(),
        inputs: inputs_of(doc),
        textures,
    })
}

#[must_use]
pub fn encode_textures(t: &CookedTextures) -> Vec<u8> {
    let mut w = W(Vec::new());
    w.0.extend_from_slice(TEXTURE_MAGIC);
    w.u32(TEXTURE_VERSION);
    w.0.extend_from_slice(&t.asset_id);
    w.0.extend_from_slice(&t.settings);
    w.len(t.inputs.len());
    for input in &t.inputs {
        w.str(&input.uri);
        w.0.extend_from_slice(&input.hash);
    }
    w.len(t.textures.len());
    for tex in &t.textures {
        w.u32(tex.image);
        let bytes = ktx2::write(&tex.ktx2);
        w.len(bytes.len());
        w.0.extend_from_slice(&bytes);
    }
    let trailer = sha256(&w.0);
    w.0.extend_from_slice(&trailer);
    w.0
}

/// Asset id, settings hash and inputs.
type Header = ([u8; 32], [u8; 32], Vec<Input>);

fn header(r: &mut R) -> Result<Header, CookError> {
    if r.take(4)? != TEXTURE_MAGIC {
        return Err(CookError::Corrupt("magic"));
    }
    if r.u32()? != TEXTURE_VERSION {
        return Err(CookError::Corrupt("version"));
    }
    let asset_id = r.array()?;
    let settings = r.array()?;
    let mut inputs = Vec::new();
    for _ in 0..r.len()? {
        let uri = r.str()?;
        inputs.push(Input {
            uri,
            hash: r.array()?,
        });
    }
    Ok((asset_id, settings, inputs))
}

/// Decode and check a `.gtex`.
///
/// # Errors
///
/// A wrong magic or version, a trailer that does not match, or a bad KTX2.
pub fn decode_textures(bytes: &[u8]) -> Result<CookedTextures, CookError> {
    let mut r = R {
        b: checked_body(bytes)?,
        at: 0,
    };
    let (asset_id, settings, inputs) = header(&mut r)?;
    let mut textures = Vec::new();
    for _ in 0..r.len()? {
        let image = r.u32()?;
        let n = r.len()?;
        textures.push(CookedTexture {
            image,
            ktx2: ktx2::read(r.take(n)?)?,
        });
    }
    if r.at != r.b.len() {
        return Err(CookError::Corrupt("trailing bytes"));
    }
    Ok(CookedTextures {
        asset_id,
        settings,
        inputs,
        textures,
    })
}

/// Where the texture record for a source lives under a cook directory.
#[must_use]
pub fn textures_path(out: &Path, asset_id: &[u8; 32]) -> PathBuf {
    out.join("cooked").join(format!("{}.gtex", hex(asset_id)))
}

/// Cook a glTF file's textures into `out`, unless the record there already
/// matches every input hash and the encoder settings.
///
/// # Errors
///
/// A source that does not import, or a failed write.
pub fn cook_textures_file(source: &Path, out: &Path) -> Result<(Outcome, PathBuf), CookError> {
    let bytes = std::fs::read(source).map_err(|_| LoadError::NotFound {
        path: source.to_path_buf(),
    })?;
    let dir = source.parent().unwrap_or(Path::new("."));
    let path = textures_path(out, &sha256(&bytes));
    if textures_fresh(&path, &bytes, dir) {
        return Ok((Outcome::UpToDate, path));
    }
    let doc = genos_load::import_bytes(&bytes, &|uri| {
        genos_load::FileSource::new(dir.join(uri)).read_bytes()
    })?;
    genos_load::write_atomic(&path, &encode_textures(&cook_textures(&doc)?))?;
    Ok((Outcome::Cooked, path))
}

/// The record at `path` checks and matches every input hash and the encoder.
pub(crate) fn textures_fresh(path: &Path, source: &[u8], dir: &Path) -> bool {
    std::fs::read(path).is_ok_and(|existing| {
        checked_body(&existing)
            .and_then(|body| header(&mut R { b: body, at: 0 }))
            .is_ok_and(|(_, settings, inputs)| {
                settings == texture_settings_hash() && inputs_match(&inputs, source, dir)
            })
    })
}

/// Read and check a `.gtex`.
///
/// # Errors
///
/// A read failure or a record that does not check.
pub fn load_textures(path: &Path) -> Result<CookedTextures, CookError> {
    decode_textures(&std::fs::read(path)?)
}
