//! One glTF metallic-roughness shade.
//!
//! The bake, the G-buffer's atlas and mean, and the path tracer all call this.
//! Diffuse albedo is base colour times (1 − metallic). The real-time picture
//! stays Lambert: callers multiply by their existing BRDF weight. The specular
//! lobe is not evaluated here.
//!
//! Departure from glTF 2.0 Appendix B and from Filament: no specular term.
//! Departure from the glTF sampler default: a missing mag filter stays nearest,
//! which is what the bake already did. The spec leaves that default to the
//! runtime. A file that sets 9728 or 9729 is honoured.

use crate::pages::miss_colour;
use crate::{Image, PbrMaterial, TextureSlot};

const MIRRORED_REPEAT: u32 = 33_648;
const CLAMP_TO_EDGE: u32 = 33_071;
const LINEAR: u32 = 9_729;

/// Linear values of one material at one UV.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadeSample {
    /// Base colour, factor times texture, linear. Not yet scaled by metal.
    pub base: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    /// 1 when the material has no occlusion texture.
    pub occlusion: f32,
    /// Base alpha times the base-colour texel alpha.
    pub alpha: f32,
    /// Base-colour texel alpha before the factor. 1 when that slot is empty.
    pub texel_alpha: f32,
    /// Tangent-space normal. (0, 0, 1) when the material has no normal map.
    pub normal_ts: [f32; 3],
    /// `emissiveTexture × emissiveFactor × emissiveStrength`. May exceed 1.
    pub emissive: [f32; 3],
    pub unlit: bool,
}

impl ShadeSample {
    /// A paint colour with no metal and no emission. Analytic hits use this.
    #[must_use]
    pub fn paint(albedo: [f32; 3]) -> Self {
        Self {
            base: albedo,
            metallic: 0.0,
            roughness: 1.0,
            occlusion: 1.0,
            alpha: 1.0,
            texel_alpha: 1.0,
            normal_ts: [0.0, 0.0, 1.0],
            emissive: [0.0; 3],
            unlit: false,
        }
    }

    /// Base colour × (1 − metallic).
    #[must_use]
    pub fn diffuse_albedo(self) -> [f32; 3] {
        let keep = 1.0 - self.metallic;
        [
            self.base[0] * keep,
            self.base[1] * keep,
            self.base[2] * keep,
        ]
    }

    /// Outgoing radiance for the Lambert picture.
    ///
    /// `brdf` is the engine weight (paint albedo / π on analytic hits). Direct
    /// light is not scaled by occlusion. Indirect light is. Emission is added
    /// after that product and is not clamped.
    #[must_use]
    pub fn outgoing(self, brdf: f32, direct: [f32; 3], indirect: [f32; 3]) -> [f32; 3] {
        let albedo = self.diffuse_albedo();
        std::array::from_fn(|channel| {
            let lit = direct[channel] + indirect[channel] * self.occlusion;
            albedo[channel] * brdf * lit + self.emissive[channel]
        })
    }
}

/// Shade `material` with every texture read at `uv`.
///
/// This is the explicit-UV entry the materials gate uses: the caller has
/// already chosen the coordinate, and a `texCoord` above 0 falls back to it.
/// A hit that carries more than one TEXCOORD set uses [`shade_sets`].
#[must_use]
pub fn shade_uv(material: &PbrMaterial, images: &[Image], uv: [f32; 2]) -> ShadeSample {
    shade_sets(material, images, &[Some(uv)])
}

/// Shade `material`, reading each slot from `uvs[slot.texcoord]`.
///
/// `uvs` is indexed by TEXCOORD number. `None` is a set the primitive did not
/// have. A missing set falls back to set 0 when that entry is `Some`, and to
/// (0, 0) otherwise.
#[must_use]
pub fn shade_sets(
    material: &PbrMaterial,
    images: &[Image],
    uvs: &[Option<[f32; 2]>],
) -> ShadeSample {
    let base_tex = sample_slot(material.base_color_texture, images, uvs, true);
    let mr = sample_slot(material.metallic_roughness_texture, images, uvs, false);
    let occlusion_tex = sample_slot(material.occlusion_texture, images, uvs, false);
    let emissive_tex = sample_slot(material.emissive_texture, images, uvs, true);
    let normal_tex = sample_slot(material.normal_texture, images, uvs, false);
    let metallic = (material.metallic * mr[2]).clamp(0.0, 1.0);
    let roughness = (material.roughness * mr[1]).clamp(0.0, 1.0);
    let occlusion = occlusion_of(material.occlusion_texture, occlusion_tex);
    ShadeSample {
        base: std::array::from_fn(|channel| material.base_color[channel] * base_tex[channel]),
        metallic,
        roughness,
        occlusion,
        alpha: material.base_color[3] * base_tex[3],
        texel_alpha: base_tex[3],
        normal_ts: normal_of(material.normal_texture, normal_tex),
        emissive: std::array::from_fn(|channel| {
            material.emissive[channel] * emissive_tex[channel] * material.emissive_strength
        }),
        unlit: false,
    }
}

fn occlusion_of(slot: Option<TextureSlot>, texel: [f32; 4]) -> f32 {
    let Some(slot) = slot else {
        return 1.0;
    };
    // glTF: 1 + strength * (R − 1). Direct light does not use this.
    (1.0 + slot.scale * (texel[0] - 1.0)).clamp(0.0, 1.0)
}

fn normal_of(slot: Option<TextureSlot>, texel: [f32; 4]) -> [f32; 3] {
    let Some(slot) = slot else {
        return [0.0, 0.0, 1.0];
    };
    let decoded = [
        texel[0] * 2.0 - 1.0,
        texel[1] * 2.0 - 1.0,
        texel[2] * 2.0 - 1.0,
    ];
    let scaled = [decoded[0] * slot.scale, decoded[1] * slot.scale, decoded[2]];
    normalize(scaled)
}

fn sample_slot(
    slot: Option<TextureSlot>,
    images: &[Image],
    uvs: &[Option<[f32; 2]>],
    srgb: bool,
) -> [f32; 4] {
    let Some(slot) = slot else {
        return [1.0; 4];
    };
    let Some(image) = images.get(slot.image) else {
        return miss_colour(&[], 0, [1.0; 4]);
    };
    if image.width == 0 || image.height == 0 {
        return miss_colour(&[], 0, [1.0; 4]);
    }
    let uv = uv_for(slot.texcoord, uvs);
    let u = wrap_coord(uv[0], slot.sampler.wrap_s);
    let v = wrap_coord(uv[1], slot.sampler.wrap_t);
    if linear_filter(slot.sampler.mag_filter) {
        sample_linear(image, u, v, srgb)
    } else {
        let x = nearest_coord(u, image.width);
        let y = nearest_coord(v, image.height);
        decode_texel(image.pixel(x, y).unwrap_or([255; 4]), srgb)
    }
}

fn uv_for(set: u32, uvs: &[Option<[f32; 2]>]) -> [f32; 2] {
    let index = usize::try_from(set).unwrap_or(0);
    if let Some(Some(uv)) = uvs.get(index) {
        return *uv;
    }
    if let Some(Some(uv)) = uvs.first() {
        return *uv;
    }
    [0.0, 0.0]
}

fn linear_filter(mag: u32) -> bool {
    mag == LINEAR
}

fn wrap_coord(t: f32, mode: u32) -> f32 {
    match mode {
        MIRRORED_REPEAT => {
            let wrapped = t.rem_euclid(2.0);
            if wrapped > 1.0 {
                2.0 - wrapped
            } else {
                wrapped
            }
        }
        CLAMP_TO_EDGE => t.clamp(0.0, 1.0),
        _ => t - t.floor(),
    }
}

fn nearest_coord(t: f32, texels: u32) -> u32 {
    let scaled = t * u32_as_f32(texels);
    let last = texels.saturating_sub(1);
    if scaled >= u32_as_f32(last) {
        last
    } else {
        f32_as_u32(scaled)
    }
}

fn sample_linear(image: &Image, u: f32, v: f32, srgb: bool) -> [f32; 4] {
    let width = u32_as_f32(image.width);
    let height = u32_as_f32(image.height);
    let texel_x = u * width - 0.5;
    let texel_y = v * height - 0.5;
    let x0 = texel_x.floor();
    let y0 = texel_y.floor();
    let frac_x = texel_x - x0;
    let frac_y = texel_y - y0;
    let sample = |ix: f32, iy: f32| {
        let px = wrap_index(ix, image.width);
        let py = wrap_index(iy, image.height);
        decode_texel(image.pixel(px, py).unwrap_or([255; 4]), srgb)
    };
    let left_top = sample(x0, y0);
    let right_top = sample(x0 + 1.0, y0);
    let left_bottom = sample(x0, y0 + 1.0);
    let right_bottom = sample(x0 + 1.0, y0 + 1.0);
    std::array::from_fn(|channel| {
        let top = left_top[channel] * (1.0 - frac_x) + right_top[channel] * frac_x;
        let bottom = left_bottom[channel] * (1.0 - frac_x) + right_bottom[channel] * frac_x;
        top * (1.0 - frac_y) + bottom * frac_y
    })
}

fn wrap_index(texel: f32, texels: u32) -> u32 {
    let last = i64::from(texels.saturating_sub(1));
    let rounded = f32_as_i64(texel).clamp(0, last);
    u32::try_from(rounded).unwrap_or(0)
}

fn decode_texel(pixel: [u8; 4], srgb: bool) -> [f32; 4] {
    std::array::from_fn(|channel| {
        let value = f32::from(pixel[channel]) / 255.0;
        if srgb && channel < 3 {
            srgb_to_linear(value)
        } else {
            value
        }
    })
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length <= 1.0e-8 {
        return [0.0, 0.0, 1.0];
    }
    [v[0] / length, v[1] / length, v[2] / length]
}

#[expect(
    clippy::as_conversions,
    reason = "texture edges fit the f32 mantissa for the sizes we sample"
)]
#[expect(
    clippy::cast_precision_loss,
    reason = "texture edges fit the f32 mantissa for the sizes we sample"
)]
fn u32_as_f32(value: u32) -> f32 {
    value as f32
}

#[expect(
    clippy::as_conversions,
    reason = "nearest_coord clamps to the image edge first"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "nearest_coord clamps to the image edge first"
)]
#[expect(
    clippy::cast_sign_loss,
    reason = "nearest_coord only converts a non-negative texel"
)]
fn f32_as_u32(value: f32) -> u32 {
    value as u32
}

#[expect(
    clippy::as_conversions,
    reason = "the linear sample is clamped to the image edge"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the linear sample is clamped to the image edge"
)]
fn f32_as_i64(value: f32) -> i64 {
    value as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffuse_albedo_darkens_metal_and_leaves_a_dielectric() {
        let metal = PbrMaterial {
            base_color: [0.8, 0.1, 0.1, 1.0],
            metallic: 0.25,
            ..PbrMaterial::default()
        };
        let sample = shade_uv(&metal, &[], [0.0, 0.0]);
        let albedo = sample.diffuse_albedo();
        assert!((albedo[0] - 0.6).abs() < 1.0e-6);
        assert!((albedo[1] - 0.075).abs() < 1.0e-6);
        let dielectric = PbrMaterial {
            base_color: [0.8, 0.1, 0.1, 1.0],
            metallic: 0.0,
            ..PbrMaterial::default()
        };
        let plain = shade_uv(&dielectric, &[], [0.0, 0.0]).diffuse_albedo();
        assert!((plain[0] - 0.8).abs() < 1.0e-6);
    }

    #[test]
    fn occlusion_scales_indirect_only() {
        let mut material = PbrMaterial {
            base_color: [1.0, 1.0, 1.0, 1.0],
            metallic: 0.0,
            ..PbrMaterial::default()
        };
        material.occlusion_texture = Some(TextureSlot {
            image: 0,
            texcoord: 0,
            sampler: crate::Sampler::default(),
            scale: 1.0,
        });
        let image = Image {
            width: 1,
            height: 1,
            pixels: vec![128, 0, 0, 255],
        };
        let sample = shade_uv(&material, &[image], [0.25, 0.25]);
        let direct = [1.0; 3];
        let indirect = [1.0; 3];
        let lit = sample.outgoing(1.0, direct, indirect);
        let full = sample.outgoing(1.0, direct, [0.0; 3]);
        assert!((sample.occlusion - 128.0 / 255.0).abs() < 1.0e-6);
        assert!((full[0] - 1.0).abs() < 1.0e-5);
        assert!((lit[0] - (1.0 + sample.occlusion)).abs() < 1.0e-5);
    }

    #[test]
    fn a_slot_reads_its_texcoord_and_a_missing_set_falls_back() {
        let mut material = PbrMaterial {
            base_color: [1.0, 1.0, 1.0, 1.0],
            metallic: 0.0,
            emissive: [1.0, 1.0, 1.0],
            ..PbrMaterial::default()
        };
        material.emissive_texture = Some(TextureSlot {
            image: 0,
            texcoord: 1,
            sampler: crate::Sampler::default(),
            scale: 1.0,
        });
        let images = [Image {
            width: 2,
            height: 2,
            pixels: vec![
                0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255,
            ],
        }];
        let hit = shade_sets(
            &material,
            &images,
            &[Some([0.25, 0.25]), Some([0.75, 0.75])],
        );
        assert!((hit.emissive[0] - 1.0).abs() < 1.0e-5);
        let fallback = shade_uv(&material, &images, [0.25, 0.25]);
        assert!(fallback.emissive[0] < 1.0e-5);
    }
}
