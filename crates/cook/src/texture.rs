//! Cooked textures, `.gtex` (the cook side of plan steps 10 and 11).
//!
//! Every image of a glTF source gets its full mip chain from
//! [`genos_load::mip_chain`], in the space its material slots give it
//! ([`genos_load::image_spaces`]). Colour (base colour, emissive) is BC7 sRGB,
//! other data is BC7 unorm and normals are BC5 (X and Y; Z is rebuilt). Each
//! image is one KTX2. The encoder is `intel_tex_2` (Intel ISPC Texture
//! Compressor, basic profile, BC7 mode 2 left on). Edges are padded to whole
//! 4×4 blocks by repeating the last row and column.
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
const ENCODER: &str = "intel_tex_2 0.5 ispc bc7-basic mode2 bc5; genos_load mip_chain v1";

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
    let settings = bc7_level_settings(alpha);
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

/// Basic profile, with BC7 mode 2 left on.
///
/// `intel_tex_2` 0.5 skips mode 2 in `opaque_basic_settings` and
/// `alpha_basic_settings`. The SSE2, SSE4 and AVX kernels then encode the
/// 2×2 quad at 12 dB. AVX2 and AVX-512 still clear 40 dB with the skip, which
/// is why a GitHub-hosted runner passes and a machine without AVX2 does not.
/// Mode 2 is the 3-subset partition those older kernels need for that block.
fn bc7_level_settings(alpha: bool) -> bc7::EncodeSettings {
    let mut settings = if alpha {
        bc7::alpha_basic_settings()
    } else {
        bc7::opaque_basic_settings()
    };
    settings.skip_mode2 = false;
    settings
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

#[cfg(test)]
mod tests {
    use super::bc7_level_settings;

    #[test]
    fn bc7_mode2_stays_on() {
        assert!(!bc7_level_settings(false).skip_mode2);
        assert!(!bc7_level_settings(true).skip_mode2);
    }
}

/// The SSE2 kernel is what a CPU without AVX2 runs. `quad.png` is a 2×2 of
/// red, green, blue and white. With mode 2 skipped that kernel scores it at
/// 12 dB; with mode 2 on it is exact. GitHub-hosted runners have AVX2, so the
/// dispatch path there does not catch a regression.
#[cfg(all(test, target_arch = "x86_64"))]
mod sse2_quad {
    use super::bc7_level_settings;

    #[repr(C)]
    struct RgbaSurface {
        ptr: *mut u8,
        width: i32,
        height: i32,
        stride: i32,
    }

    #[repr(C)]
    struct Bc7Settings {
        mode_selection: [bool; 4],
        refine_iterations: [i32; 8],
        skip_mode2: bool,
        fast_skip_mode1: i32,
        fast_skip_mode3: i32,
        fast_skip_mode7: i32,
        mode45_channel0: i32,
        refine_iterations_channel: i32,
        channels: i32,
    }

    extern "C" {
        fn CompressBlocksBC7_ispc_sse2(
            src: *mut RgbaSurface,
            dst: *mut u8,
            settings: *mut Bc7Settings,
        );
    }

    fn to_i32(value: u32) -> i32 {
        i32::try_from(value).unwrap_or(0)
    }

    #[test]
    fn sse2_encodes_the_quad_above_the_floor() {
        // 2×2 red, green, blue, white, padded by repeating the last row and column.
        let corner = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let mut pixels = [0_u8; 64];
        for row in 0..4 {
            for column in 0..4 {
                let from = (row.min(1) * 2 + column.min(1)) * 4;
                let to = (row * 4 + column) * 4;
                pixels[to..to + 4].copy_from_slice(&corner[from..from + 4]);
            }
        }
        let want = bc7_level_settings(false);
        let mut settings = Bc7Settings {
            mode_selection: want.mode_selection,
            refine_iterations: [
                to_i32(want.refine_iterations[0]),
                to_i32(want.refine_iterations[1]),
                to_i32(want.refine_iterations[2]),
                to_i32(want.refine_iterations[3]),
                to_i32(want.refine_iterations[4]),
                to_i32(want.refine_iterations[5]),
                to_i32(want.refine_iterations[6]),
                to_i32(want.refine_iterations[7]),
            ],
            skip_mode2: want.skip_mode2,
            fast_skip_mode1: to_i32(want.fast_skip_threshold_mode1),
            fast_skip_mode3: to_i32(want.fast_skip_threshold_mode3),
            fast_skip_mode7: to_i32(want.fast_skip_threshold_mode7),
            mode45_channel0: to_i32(want.mode45_channel0),
            refine_iterations_channel: to_i32(want.refine_iterations_channel),
            channels: want.channels,
        };
        let mut surface = RgbaSurface {
            ptr: pixels.as_mut_ptr(),
            width: 4,
            height: 4,
            stride: 16,
        };
        let mut block = [0_u8; 16];
        // SAFETY: `pixels` is one tightly packed 4×4 RGBA block, `block` holds
        // the 16-byte BC7 output, and `settings` matches `bc7_enc_settings`.
        // `CompressBlocksBC7_ispc_sse2` is the SSE2 kernel in `intel_tex_2`.
        unsafe {
            CompressBlocksBC7_ispc_sse2(&raw mut surface, block.as_mut_ptr(), &raw mut settings);
        }
        let mut texels = [0_u8; 64];
        bcdec_rs::bc7(&block, &mut texels, 16);
        let mut sum = 0.0_f64;
        let mut count = 0.0_f64;
        for row in 0..2 {
            for column in 0..2 {
                for channel in 0..4 {
                    let got = f64::from(texels[(row * 4 + column) * 4 + channel]);
                    let src_byte = f64::from(corner[(row * 2 + column) * 4 + channel]);
                    let delta = got - src_byte;
                    sum += delta * delta;
                    count += 1.0;
                }
            }
        }
        let mse = sum / count;
        let psnr = if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0_f64 * 255.0 / mse).log10()
        };
        assert!(psnr >= 40.0, "sse2 quad {psnr:.1} dB");
    }
}
