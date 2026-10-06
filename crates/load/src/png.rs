//! PNG images. Pixels are RGBA8, row-major, top to bottom.
//!
//! 8-bit greyscale, truecolor, indexed, greyscale-alpha, and truecolor-alpha
//! images are accepted. The image is not interlaced. Each edge is at most
//! 8192 pixels.

use crate::bin::{u16_be, u32_be};
use crate::error::LoadError;
use crate::inflate::zlib_decompress;

const MAX_EDGE: u32 = 8192;
const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// An RGBA8 image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8 pixels. Length is `width * height * 4`.
    pub pixels: Vec<u8>,
}

impl Image {
    /// Pixel at `x`, `y`, or `None` when the point is outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = ((y * self.width + x) * 4) as usize;
        Some([
            self.pixels[index],
            self.pixels[index + 1],
            self.pixels[index + 2],
            self.pixels[index + 3],
        ])
    }
}

pub(crate) fn decode_png(data: &[u8]) -> Result<Image, LoadError> {
    if data.len() < SIGNATURE.len() || &data[..SIGNATURE.len()] != SIGNATURE {
        return Err(if data.starts_with(b"\x89PNG") {
            LoadError::Truncated
        } else {
            LoadError::Unrecognized
        });
    }
    let mut pos = SIGNATURE.len();
    let mut header = None;
    let mut palette = None;
    let mut transparency = None;
    let mut idat = Vec::new();
    let mut idat_started = false;
    let mut idat_closed = false;
    let mut saw_iend = false;
    let mut first = true;
    while !saw_iend {
        if pos + 8 > data.len() {
            return Err(LoadError::Truncated);
        }
        let length = u32_be(data, pos)? as usize;
        let kind = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
        let body_at = pos + 8;
        let crc_at = body_at.checked_add(length).ok_or(LoadError::Unrecognized)?;
        if crc_at + 4 > data.len() {
            return Err(LoadError::Truncated);
        }
        let body = &data[body_at..crc_at];
        if png_crc(&kind, body) != u32_be(data, crc_at)? {
            return Err(LoadError::Unrecognized);
        }
        pos = crc_at + 4;
        if first && &kind != b"IHDR" {
            return Err(LoadError::Unrecognized);
        }
        first = false;
        match &kind {
            b"IHDR" => {
                if header.is_some() || length != 13 {
                    return Err(LoadError::Unrecognized);
                }
                header = Some(parse_ihdr(body)?);
            }
            b"PLTE" => {
                if palette.is_some()
                    || idat_started
                    || length == 0
                    || length % 3 != 0
                    || length / 3 > 256
                {
                    return Err(LoadError::Unrecognized);
                }
                let mut colors = Vec::with_capacity(length / 3);
                for entry in body.chunks(3) {
                    colors.push([entry[0], entry[1], entry[2]]);
                }
                palette = Some(colors);
            }
            b"tRNS" => {
                if transparency.is_some() || idat_started {
                    return Err(LoadError::Unrecognized);
                }
                transparency = Some(body.to_vec());
            }
            b"IDAT" => {
                if idat_closed {
                    return Err(LoadError::Unrecognized);
                }
                idat_started = true;
                idat.extend_from_slice(body);
            }
            b"IEND" => {
                if length != 0 {
                    return Err(LoadError::Unrecognized);
                }
                saw_iend = true;
            }
            _ => {
                if idat_started {
                    idat_closed = true;
                }
                if kind[0].is_ascii_uppercase() {
                    return Err(LoadError::Unrecognized);
                }
            }
        }
    }
    let (width, height, color_type) = header.ok_or(LoadError::Unrecognized)?;
    if idat.is_empty() {
        return Err(LoadError::Unrecognized);
    }
    let channels = match color_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return Err(LoadError::Unrecognized),
    };
    if color_type == 3 && palette.is_none() {
        return Err(LoadError::Unrecognized);
    }
    if matches!(color_type, 0 | 4) && palette.is_some() {
        return Err(LoadError::Unrecognized);
    }
    if matches!(color_type, 4 | 6) && transparency.is_some() {
        return Err(LoadError::Unrecognized);
    }
    let row_bytes = (width as usize)
        .checked_mul(channels)
        .ok_or(LoadError::Unrecognized)?;
    let scan_bytes = row_bytes.checked_add(1).ok_or(LoadError::Unrecognized)?;
    let expected = scan_bytes
        .checked_mul(height as usize)
        .ok_or(LoadError::Unrecognized)?;
    let raw = zlib_decompress(&idat, expected)?;
    if raw.len() != expected {
        return Err(if raw.len() < expected {
            LoadError::Truncated
        } else {
            LoadError::Unrecognized
        });
    }
    let samples = unfilter(&raw, height as usize, row_bytes, channels)?;
    let pixels = expand(
        &samples,
        color_type,
        palette.as_deref(),
        transparency.as_deref(),
    )?;
    Ok(Image {
        width,
        height,
        pixels,
    })
}

fn parse_ihdr(body: &[u8]) -> Result<(u32, u32, u8), LoadError> {
    let width = u32_be(body, 0)?;
    let height = u32_be(body, 4)?;
    let depth = body[8];
    let color = body[9];
    let compression = body[10];
    let filter = body[11];
    let interlace = body[12];
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
        return Err(LoadError::Unrecognized);
    }
    if depth != 8 || compression != 0 || filter != 0 || interlace != 0 {
        return Err(LoadError::Unrecognized);
    }
    if !matches!(color, 0 | 2 | 3 | 4 | 6) {
        return Err(LoadError::Unrecognized);
    }
    Ok((width, height, color))
}

fn unfilter(
    raw: &[u8],
    height: usize,
    row_bytes: usize,
    channels: usize,
) -> Result<Vec<u8>, LoadError> {
    let mut previous = vec![0u8; row_bytes];
    let mut samples = Vec::with_capacity(height * row_bytes);
    let mut at = 0usize;
    for _ in 0..height {
        let filter = *raw.get(at).ok_or(LoadError::Truncated)?;
        at += 1;
        let row = raw.get(at..at + row_bytes).ok_or(LoadError::Truncated)?;
        at += row_bytes;
        let mut reconstructed = row.to_vec();
        for x in 0..row_bytes {
            let left = if x >= channels {
                reconstructed[x - channels]
            } else {
                0
            };
            let up = previous[x];
            let up_left = if x >= channels {
                previous[x - channels]
            } else {
                0
            };
            let predicted = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, up_left),
                _ => return Err(LoadError::Unrecognized),
            };
            reconstructed[x] = reconstructed[x].wrapping_add(predicted);
        }
        previous.copy_from_slice(&reconstructed);
        samples.extend_from_slice(&reconstructed);
    }
    Ok(samples)
}

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let a = i16::from(left);
    let b = i16::from(up);
    let c = i16::from(up_left);
    let estimate = a + b - c;
    let distance_left = (estimate - a).abs();
    let distance_up = (estimate - b).abs();
    let distance_up_left = (estimate - c).abs();
    if distance_left <= distance_up && distance_left <= distance_up_left {
        left
    } else if distance_up <= distance_up_left {
        up
    } else {
        up_left
    }
}

fn expand(
    samples: &[u8],
    color_type: u8,
    palette: Option<&[[u8; 3]]>,
    transparency: Option<&[u8]>,
) -> Result<Vec<u8>, LoadError> {
    let mut pixels = Vec::with_capacity(samples.len().saturating_mul(4));
    match color_type {
        0 => {
            let transparent = match transparency {
                Some(bytes) if bytes.len() >= 2 => Some((u16_be(bytes, 0)? & 0xff) as u8),
                Some(_) => return Err(LoadError::Unrecognized),
                None => None,
            };
            for &gray in samples {
                let alpha = if transparent == Some(gray) { 0 } else { 255 };
                pixels.extend_from_slice(&[gray, gray, gray, alpha]);
            }
        }
        2 => {
            let key = match transparency {
                Some(bytes) if bytes.len() >= 6 => Some((
                    (u16_be(bytes, 0)? & 0xff) as u8,
                    (u16_be(bytes, 2)? & 0xff) as u8,
                    (u16_be(bytes, 4)? & 0xff) as u8,
                )),
                Some(_) => return Err(LoadError::Unrecognized),
                None => None,
            };
            if samples.len() % 3 != 0 {
                return Err(LoadError::Unrecognized);
            }
            for pixel in samples.chunks(3) {
                let alpha = if key == Some((pixel[0], pixel[1], pixel[2])) {
                    0
                } else {
                    255
                };
                pixels.extend_from_slice(&[pixel[0], pixel[1], pixel[2], alpha]);
            }
        }
        3 => {
            let colors = palette.ok_or(LoadError::Unrecognized)?;
            let alphas = transparency.unwrap_or(&[]);
            for &index in samples {
                let color = colors
                    .get(usize::from(index))
                    .ok_or(LoadError::Unrecognized)?;
                let alpha = alphas.get(usize::from(index)).copied().unwrap_or(255);
                pixels.extend_from_slice(&[color[0], color[1], color[2], alpha]);
            }
        }
        4 => {
            if samples.len() % 2 != 0 {
                return Err(LoadError::Unrecognized);
            }
            for pixel in samples.chunks(2) {
                pixels.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        6 => pixels.extend_from_slice(samples),
        _ => return Err(LoadError::Unrecognized),
    }
    let channels = channels_of(color_type);
    if channels == 0 || samples.len() % channels != 0 {
        return Err(LoadError::Unrecognized);
    }
    if pixels.len() != samples.len() / channels * 4 {
        return Err(LoadError::Unrecognized);
    }
    Ok(pixels)
}

fn channels_of(color_type: u8) -> usize {
    match color_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => 1,
    }
}

fn png_crc(kind: &[u8; 4], body: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in kind.iter().chain(body.iter()) {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            if crc & 1 == 1 {
                crc = (crc >> 1) ^ 0xedb8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}
