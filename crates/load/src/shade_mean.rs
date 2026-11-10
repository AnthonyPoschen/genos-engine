//! Area-weighted mean of one primitive, stored in the `.gmat` record.
//!
//! This is the cook-side copy of `surface_mean` in `crates/render/src/gltf_draw.rs`.
//! The draw function keeps its existing lines so the lossy-cast ceiling stays put.
//! `primitive_shades_match_surface_mean_across_meshes` locks the two together.

use crate::{shade_sets, shade_uv, GltfScene, Image, PbrMaterial, Primitive, PrimitiveShade};

/// Barycentric samples. The same four points `surface_mean` uses.
const SAMPLE_POINTS: [[f32; 3]; 4] = [
    [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
    [2.0 / 3.0, 1.0 / 6.0, 1.0 / 6.0],
    [1.0 / 6.0, 2.0 / 3.0, 1.0 / 6.0],
    [1.0 / 6.0, 1.0 / 6.0, 2.0 / 3.0],
];

/// One mean and emission per primitive, in mesh order.
///
/// A primitive with no material stores white and no emission, which is what the
/// draw uses for that primitive.
#[must_use]
pub fn primitive_shades(scene: &GltfScene) -> Vec<PrimitiveShade> {
    let mut shades = Vec::new();
    for mesh in &scene.meshes {
        for primitive in &mesh.primitives {
            let (mean, emission) = match primitive
                .material
                .and_then(|index| scene.materials.get(index))
            {
                Some(material) => surface_mean(material, &scene.images, primitive),
                None => ([1.0; 3], [0.0; 3]),
            };
            shades.push(PrimitiveShade { mean, emission });
        }
    }
    shades
}

fn surface_mean(m: &PbrMaterial, images: &[Image], prim: &Primitive) -> ([f32; 3], [f32; 3]) {
    let textured = m.base_color_texture.is_some()
        || m.metallic_roughness_texture.is_some()
        || m.emissive_texture.is_some();
    if !textured || prim.texcoords.len() < prim.positions.len() {
        return (diffuse(m, images), emission(m, images));
    }
    let (index_chunks, _) = prim.indices.as_chunks::<3>();
    let tris: Vec<[usize; 3]> = index_chunks
        .iter()
        .filter_map(|corners| {
            let tri = [
                usize::try_from(corners[0]).ok()?,
                usize::try_from(corners[1]).ok()?,
                usize::try_from(corners[2]).ok()?,
            ];
            tri.iter()
                .all(|corner| *corner < prim.positions.len())
                .then_some(tri)
        })
        .collect();
    let areas: Vec<f64> = tris.iter().map(|tri| triangle_area(prim, tri)).collect();
    let total: f64 = areas.iter().sum();
    if tris.is_empty() || total <= 0.0 {
        return (diffuse(m, images), emission(m, images));
    }
    let (mut diffuse_sum, mut emission_sum) = ([0.0_f64; 3], [0.0_f64; 3]);
    let weight_scale = f64::from(u32::try_from(SAMPLE_POINTS.len()).unwrap_or(1));
    for (tri, area) in tris.iter().zip(&areas) {
        let weight = area / total / weight_scale;
        for bary in SAMPLE_POINTS {
            let uvs = texcoord_sample(prim, tri, bary);
            let sample = shade_sets(m, images, &uvs);
            let diffuse_albedo = sample.diffuse_albedo();
            let emissive = sample.emissive;
            for channel in 0..3 {
                diffuse_sum[channel] += weight * f64::from(diffuse_albedo[channel]);
                emission_sum[channel] += weight * f64::from(emissive[channel]);
            }
        }
    }
    (diffuse_sum.map(narrow_f32), emission_sum.map(narrow_f32))
}

fn triangle_area(prim: &Primitive, corners: &[usize; 3]) -> f64 {
    let Some(origin) = prim.positions.get(corners[0]) else {
        return 0.0;
    };
    let Some(second) = prim.positions.get(corners[1]) else {
        return 0.0;
    };
    let Some(third) = prim.positions.get(corners[2]) else {
        return 0.0;
    };
    let edge_u: [f32; 3] = std::array::from_fn(|axis| second[axis] - origin[axis]);
    let edge_v: [f32; 3] = std::array::from_fn(|axis| third[axis] - origin[axis]);
    let cross = [
        edge_u[1] * edge_v[2] - edge_u[2] * edge_v[1],
        edge_u[2] * edge_v[0] - edge_u[0] * edge_v[2],
        edge_u[0] * edge_v[1] - edge_u[1] * edge_v[0],
    ];
    let sum = cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2];
    0.5 * f64::from(sum).sqrt()
}

fn texcoord_sample(prim: &Primitive, corners: &[usize; 3], b: [f32; 3]) -> Vec<Option<[f32; 2]>> {
    let interp = |set: &[[f32; 2]]| -> Option<[f32; 2]> {
        if set.len() != prim.positions.len() {
            return None;
        }
        let samples = [
            set.get(corners[0])?,
            set.get(corners[1])?,
            set.get(corners[2])?,
        ];
        Some(std::array::from_fn(|axis| {
            b[0] * samples[0][axis] + b[1] * samples[1][axis] + b[2] * samples[2][axis]
        }))
    };
    if prim.texcoord_sets.is_empty() {
        return vec![interp(&prim.texcoords)];
    }
    prim.texcoord_sets.iter().map(|set| interp(set)).collect()
}

fn texel_light(m: &PbrMaterial, images: &[Image], u: f32, v: f32) -> ([f32; 3], [f32; 3]) {
    let sample = shade_uv(m, images, [u, v]);
    (sample.diffuse_albedo(), sample.emissive)
}

fn diffuse(m: &PbrMaterial, images: &[Image]) -> [f32; 3] {
    const GRID: u32 = 256;
    let step = f32::from(u16::try_from(GRID).unwrap_or(1));
    let mut sum = [0.0_f64; 3];
    for row in 0..GRID {
        for column in 0..GRID {
            let u = (f32::from(u16::try_from(column).unwrap_or(0)) + 0.5) / step;
            let v = (f32::from(u16::try_from(row).unwrap_or(0)) + 0.5) / step;
            let (albedo, _) = texel_light(m, images, u, v);
            for channel in 0..3 {
                sum[channel] += f64::from(albedo[channel]);
            }
        }
    }
    let n = f64::from(GRID) * f64::from(GRID);
    sum.map(|v| narrow_f32(v / n))
}

fn emission(m: &PbrMaterial, images: &[Image]) -> [f32; 3] {
    let tex = m
        .emissive_texture
        .and_then(|t| images.get(t.image))
        .map_or([1.0; 3], mean_linear);
    std::array::from_fn(|c| m.emissive[c] * tex[c] * m.emissive_strength)
}

fn mean_linear(img: &Image) -> [f32; 3] {
    let lut = srgb_lut();
    let mut sum = [0.0_f64; 3];
    let (pixels, _) = img.pixels.as_chunks::<4>();
    for pixel in pixels {
        for (channel_index, channel) in pixel.iter().take(3).enumerate() {
            let index = usize::from(*channel);
            sum[channel_index] += f64::from(lut.get(index).copied().unwrap_or(0.0));
        }
    }
    let count = u32::try_from(img.pixels.len() / 4)
        .unwrap_or(u32::MAX)
        .max(1);
    let n = f64::from(count);
    sum.map(|s| narrow_f32(s / n))
}

fn srgb_lut() -> Vec<f32> {
    (0..256_u16)
        .map(|v| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
        .collect()
}

/// IEEE-754 round-ties-to-even narrowing. Same bits as a Rust `f64 as f32`.
fn narrow_f32(value: f64) -> f32 {
    let bits = value.to_bits();
    let sign = u32::try_from(bits >> 63).unwrap_or(0);
    let exp_u = u32::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    let sig = bits & 0x000f_ffff_ffff_ffff;

    if exp_u == 0x7ff {
        if sig == 0 {
            return f32::from_bits((sign << 31) | 0x7f80_0000);
        }
        let payload = (u32::try_from(sig >> (52 - 23)).unwrap_or(0) & 0x007f_ffff) | 0x0040_0000;
        return f32::from_bits((sign << 31) | 0x7f80_0000 | payload);
    }
    if exp_u == 0 {
        return f32::from_bits(sign << 31);
    }

    let exp = i32::try_from(exp_u).unwrap_or(0) - 1023;
    let fexp = exp + 127;
    let sig_full = sig | (1_u64 << 52);

    if fexp >= 0xff {
        return f32::from_bits((sign << 31) | 0x7f80_0000);
    }
    if fexp > 0 {
        return pack_normal(sign, fexp, sig_full);
    }

    let shift = u32::try_from(-exp - 97).unwrap_or(64);
    if shift >= 64 {
        return f32::from_bits(sign << 31);
    }
    let mant = round_shift(sig_full, shift);
    if mant == 0 {
        return f32::from_bits(sign << 31);
    }
    if mant >= (1_u32 << 23) {
        return f32::from_bits((sign << 31) | (1 << 23));
    }
    f32::from_bits((sign << 31) | mant)
}

fn pack_normal(sign: u32, fexp: i32, sig_full: u64) -> f32 {
    let mant_wide = round_shift(sig_full, 29);
    let mut exp = u32::try_from(fexp).unwrap_or(0);
    let mut mant = mant_wide;
    if mant >= (1 << 24) {
        mant >>= 1;
        exp += 1;
    }
    if exp >= 0xff {
        return f32::from_bits((sign << 31) | 0x7f80_0000);
    }
    let frac = mant & 0x007f_ffff;
    f32::from_bits((sign << 31) | (exp << 23) | frac)
}

fn round_shift(value: u64, shift: u32) -> u32 {
    if shift == 0 {
        return u32::try_from(value).unwrap_or(u32::MAX);
    }
    if shift >= 64 {
        return 0;
    }
    let kept = value >> shift;
    let remainder = value & ((1_u64 << shift) - 1);
    let half = 1_u64 << (shift - 1);
    let round_up = remainder > half || (remainder == half && (kept & 1) == 1);
    let rounded = if round_up { kept + 1 } else { kept };
    u32::try_from(rounded).unwrap_or(u32::MAX)
}
