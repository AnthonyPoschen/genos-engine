//! RGBA8 mip chain. BC7 and ASTC are not encoded here.
//!
//! Open question 1 in the material design is unchosen, so the cook stays RGBA8.
//! sRGB images are boxed in linear light and encoded back. Linear images are
//! boxed in the stored values. Normals are averaged as vectors and renormalised.

use crate::Image;

/// How a mip is boxed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipSpace {
    /// Base colour and emissive.
    Srgb,
    /// Metallic-roughness and occlusion.
    Linear,
    /// Tangent-space normal. Stored RGBA8.
    Normal,
}

/// One level. Index 0 is the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mip {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Full chain, finest first, down to 1×1.
#[must_use]
pub fn mip_chain(image: &Image, space: MipSpace) -> Vec<Mip> {
    let mut chain = vec![Mip {
        width: image.width,
        height: image.height,
        pixels: image.pixels.clone(),
    }];
    while let Some(last) = chain.last() {
        if last.width <= 1 && last.height <= 1 {
            break;
        }
        let next = downsample(last, space);
        chain.push(next);
    }
    chain
}

fn downsample(source: &Mip, space: MipSpace) -> Mip {
    let width = source.width.max(1).div_ceil(2);
    let height = source.height.max(1).div_ceil(2);
    let texels = usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0);
    let mut pixels = vec![0_u8; texels * 4];
    for y in 0..height {
        for x in 0..width {
            let block = block_average(source, x, y, space);
            let at = texel_index(x, y, width);
            pixels[at..at + 4].copy_from_slice(&block);
        }
    }
    Mip {
        width,
        height,
        pixels,
    }
}

fn block_average(source: &Mip, x: u32, y: u32, space: MipSpace) -> [u8; 4] {
    let x0 = x * 2;
    let y0 = y * 2;
    let x1 = (x0 + 1).min(source.width.saturating_sub(1));
    let y1 = (y0 + 1).min(source.height.saturating_sub(1));
    let samples = [
        texel(source, x0, y0),
        texel(source, x1, y0),
        texel(source, x0, y1),
        texel(source, x1, y1),
    ];
    match space {
        MipSpace::Linear => average_bytes(&samples),
        MipSpace::Srgb => average_srgb(&samples),
        MipSpace::Normal => average_normal(&samples),
    }
}

fn texel(source: &Mip, x: u32, y: u32) -> [u8; 4] {
    let at = texel_index(x, y, source.width);
    [
        source.pixels[at],
        source.pixels[at + 1],
        source.pixels[at + 2],
        source.pixels[at + 3],
    ]
}

fn texel_index(x: u32, y: u32, width: u32) -> usize {
    let x = usize::try_from(x).unwrap_or(0);
    let y = usize::try_from(y).unwrap_or(0);
    let width = usize::try_from(width).unwrap_or(0);
    (y * width + x) * 4
}

fn average_bytes(samples: &[[u8; 4]; 4]) -> [u8; 4] {
    let mut sum = [0.0_f32; 4];
    for sample in samples {
        for channel in 0..4 {
            sum[channel] += unit(sample[channel]);
        }
    }
    sum.map(|value| unit_to_byte(value / 4.0))
}

fn average_srgb(samples: &[[u8; 4]; 4]) -> [u8; 4] {
    let mut sum = [0.0_f32; 4];
    for sample in samples {
        for channel in 0..3 {
            sum[channel] += srgb_to_linear(unit(sample[channel]));
        }
        sum[3] += unit(sample[3]);
    }
    let mut out = [0_u8; 4];
    for channel in 0..3 {
        out[channel] = unit_to_byte(linear_to_srgb(sum[channel] / 4.0));
    }
    out[3] = unit_to_byte(sum[3] / 4.0);
    out
}

fn average_normal(samples: &[[u8; 4]; 4]) -> [u8; 4] {
    let mut sum = [0.0_f32; 3];
    for sample in samples {
        for channel in 0..3 {
            sum[channel] += unit(sample[channel]) * 2.0 - 1.0;
        }
    }
    let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
    let normal = if length <= 1.0e-8 {
        [0.0, 0.0, 1.0]
    } else {
        [sum[0] / length, sum[1] / length, sum[2] / length]
    };
    [
        unit_to_byte(normal[0] * 0.5 + 0.5),
        unit_to_byte(normal[1] * 0.5 + 0.5),
        unit_to_byte(normal[2] * 0.5 + 0.5),
        255,
    ]
}

fn unit(byte: u8) -> f32 {
    f32::from(byte) / 255.0
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[expect(clippy::as_conversions, reason = "the rounded texel is clamped to 0..=255")]
#[expect(clippy::cast_possible_truncation, reason = "the rounded texel is clamped to 0..=255")]
#[expect(clippy::cast_sign_loss, reason = "the rounded texel is clamped to 0..=255")]
fn unit_to_byte(value: f32) -> u8 {
    value.clamp(0.0, 1.0).mul_add(255.0, 0.5).floor() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_srgb_colour_survives_the_mip() {
        let image = Image {
            width: 2,
            height: 2,
            pixels: vec![188, 0, 0, 255, 188, 0, 0, 255, 188, 0, 0, 255, 188, 0, 0, 255],
        };
        let chain = mip_chain(&image, MipSpace::Srgb);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[1].pixels, vec![188, 0, 0, 255]);
    }

    #[test]
    fn opposite_x_normals_cancel() {
        let image = Image {
            width: 2,
            height: 2,
            pixels: vec![
                255, 128, 128, 255, 0, 128, 128, 255, 255, 128, 128, 255, 0, 128, 128, 255,
            ],
        };
        let chain = mip_chain(&image, MipSpace::Normal);
        let pixel = &chain[1].pixels;
        // Byte 128 is 1/255 above zero, so +X and -X leave equal +Y and +Z.
        assert_eq!(pixel[0], 128);
        assert_eq!(pixel[1], 218);
        assert_eq!(pixel[2], 218);
    }
}
