#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::bool_to_int_with_if, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::chunks_exact_to_as_chunks, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::missing_errors_doc, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::missing_panics_doc, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::return_self_not_must_use, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::unwrap_used, reason = "TODO(lint-2026-10-14): pre-existing. docs/LINT_RULES.md")]

//! Linear RGB pictures and the tone curve the renderer uses.
//!
//! The curve matches `genos_render::mesh::tone` and
//! `genos_render::linear_from_display`. Values at or below 0.64 are unchanged.
//! Above that, a shoulder compresses the highlight.

use std::path::Path;

/// Linear RGB, row-major, length `width * height * 3`.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub linear: Vec<f32>,
}

impl Picture {
    pub fn new(width: u32, height: u32, linear: Vec<f32>) -> Result<Self, String> {
        let n = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(3))
            .ok_or_else(|| "picture size overflow".to_string())?;
        if linear.len() != n {
            return Err(format!(
                "picture {}x{} needs {n} floats, got {}",
                width,
                height,
                linear.len()
            ));
        }
        Ok(Self {
            width,
            height,
            linear,
        })
    }

    pub fn solid(width: u32, height: u32, rgb: [f32; 3]) -> Self {
        let n = (width * height) as usize;
        let mut linear = Vec::with_capacity(n * 3);
        for _ in 0..n {
            linear.extend_from_slice(&rgb);
        }
        Self {
            width,
            height,
            linear,
        }
    }

    pub fn pixels(&self) -> usize {
        (self.width * self.height) as usize
    }

    pub fn pixel(&self, x: u32, y: u32) -> [f32; 3] {
        let i = ((y * self.width + x) * 3) as usize;
        [self.linear[i], self.linear[i + 1], self.linear[i + 2]]
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, rgb: [f32; 3]) {
        let i = ((y * self.width + x) * 3) as usize;
        self.linear[i..i + 3].copy_from_slice(&rgb);
    }

    pub fn luma_at(&self, index: usize) -> f32 {
        let i = index * 3;
        luma([self.linear[i], self.linear[i + 1], self.linear[i + 2]])
    }

    /// Scale every channel. Used by the brightness-bias defect.
    pub fn scaled(&self, gain: f32) -> Self {
        Self {
            width: self.width,
            height: self.height,
            linear: self.linear.iter().map(|c| c * gain).collect(),
        }
    }

    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, rgb: [f32; 3]) {
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        for yy in y..y1 {
            for xx in x..x1 {
                self.set_pixel(xx, yy, rgb);
            }
        }
    }

    /// Mean of the frames. Every frame must match this picture's size.
    pub fn average(frames: &[Self]) -> Result<Self, String> {
        let first = frames
            .first()
            .ok_or_else(|| "no frames to average".to_string())?;
        let mut acc = vec![0.0f64; first.linear.len()];
        for frame in frames {
            if frame.width != first.width || frame.height != first.height {
                return Err("frames differ in size".to_string());
            }
            for (a, s) in acc.iter_mut().zip(&frame.linear) {
                *a += f64::from(*s);
            }
        }
        let n = frames.len() as f64;
        let linear = acc.into_iter().map(|v| (v / n) as f32).collect();
        Self::new(first.width, first.height, linear)
    }

    /// Display BGRA8 from a renderer readback, tone undone per channel.
    pub fn from_bgra(width: u32, height: u32, bgra: &[u8]) -> Result<Self, String> {
        let n = (width * height) as usize;
        if bgra.len() < n * 4 {
            return Err(format!(
                "bgra {}x{} needs {} bytes, got {}",
                width,
                height,
                n * 4,
                bgra.len()
            ));
        }
        let mut linear = Vec::with_capacity(n * 3);
        for px in bgra.chunks_exact(4).take(n) {
            linear.push(linear_from_display(px[2] as f32 / 255.0));
            linear.push(linear_from_display(px[1] as f32 / 255.0));
            linear.push(linear_from_display(px[0] as f32 / 255.0));
        }
        Self::new(width, height, linear)
    }

    pub fn toned_rgb8(&self) -> Vec<u8> {
        self.linear
            .iter()
            .map(|c| (tone(*c) * 255.0).round().clamp(0.0, 255.0) as u8)
            .collect()
    }

    pub fn save_png(&self, path: &Path) -> Result<(), String> {
        write_bytes(
            path,
            &encode_png(self.width, self.height, &self.toned_rgb8()),
        )
    }

    /// Exact linear samples. `LREF` header, then little-endian f32 RGB.
    pub fn save_linear(&self, path: &Path) -> Result<(), String> {
        let mut bytes = Vec::with_capacity(16 + self.linear.len() * 4);
        bytes.extend_from_slice(b"LREF");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&self.width.to_le_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        for c in &self.linear {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        write_bytes(path, &bytes)
    }

    pub fn load_linear(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
        if bytes.len() < 16 || &bytes[0..4] != b"LREF" {
            return Err(format!("{}: not a linear reference", path.display()));
        }
        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        if version != 1 {
            return Err(format!("{}: linear version {version}", path.display()));
        }
        let width = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let height = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let rest = &bytes[16..];
        if rest.len() % 4 != 0 {
            return Err(format!("{}: truncated linear file", path.display()));
        }
        let linear = rest
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        Self::new(width, height, linear)
    }

    /// Stored-deflate RGB8 PNG, the encoding `genos-debug` writes for captures.
    ///
    /// Display bytes are undone with [`linear_from_display`].
    pub fn load_png(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
        decode_png(&bytes).map_err(|err| format!("{}: {err}", path.display()))
    }
}

pub fn luma(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// Same shoulder as the renderer. At or below 0.64 the channel is unchanged.
pub fn tone(x: f32) -> f32 {
    if x <= 0.64 {
        x.max(0.0)
    } else {
        let extra = x - 0.64;
        0.64 + 0.14 * (extra / (extra + 1.1))
    }
}

/// Inverse of [`tone`]. Matches `genos_render::linear_from_display`.
pub fn linear_from_display(displayed: f32) -> f32 {
    if displayed <= 0.64 {
        return displayed.max(0.0);
    }
    let k = ((displayed - 0.64) / 0.14).clamp(0.0, 0.999);
    0.64 + 1.1 * k / (1.0 - k)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        }
    }
    std::fs::write(path, bytes).map_err(|err| format!("{}: {err}", path.display()))
}

/// PNG, stored deflate blocks, no compression. RGB8.
pub fn encode_png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for y in 0..height {
        raw.push(0);
        let row = (y * width * 3) as usize;
        let end = row + (width * 3) as usize;
        if end <= rgb.len() {
            raw.extend_from_slice(&rgb[row..end]);
        } else {
            raw.resize(raw.len() + (width * 3) as usize, 0);
        }
    }
    let mut zlib = Vec::new();
    zlib.extend_from_slice(&[0x78, 0x01]);
    let mut adler_s1: u32 = 1;
    let mut adler_s2: u32 = 0;
    let mut rest = raw.as_slice();
    while !rest.is_empty() {
        let take = rest.len().min(65535);
        let last = take == rest.len();
        zlib.push(if last { 1 } else { 0 });
        zlib.extend_from_slice(&(take as u16).to_le_bytes());
        zlib.extend_from_slice(&(!take as u16).to_le_bytes());
        zlib.extend_from_slice(&rest[..take]);
        for byte in &rest[..take] {
            adler_s1 = (adler_s1 + u32::from(*byte)) % 65521;
            adler_s2 = (adler_s2 + adler_s1) % 65521;
        }
        rest = &rest[take..];
    }
    zlib.extend_from_slice(&((adler_s2 << 16) | adler_s1).to_be_bytes());

    let mut png = Vec::new();
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut hdr = Vec::new();
    hdr.extend_from_slice(&width.to_be_bytes());
    hdr.extend_from_slice(&height.to_be_bytes());
    hdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_chunk(&mut png, b"IHDR", &hdr);
    write_chunk(&mut png, b"IDAT", &zlib);
    write_chunk(&mut png, b"IEND", &[]);
    png
}

fn decode_png(bytes: &[u8]) -> Result<Picture, String> {
    const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if bytes.len() < 8 || bytes[0..8] != SIG {
        return Err("not a png".to_string());
    }
    let mut cursor = 8_usize;
    let mut width = None;
    let mut height = None;
    let mut idat = Vec::new();
    while cursor + 8 <= bytes.len() {
        let len = be_u32(bytes, cursor)?;
        let len_us = usize::try_from(len).map_err(|_| "png chunk is too large".to_string())?;
        let kind = bytes
            .get(cursor + 4..cursor + 8)
            .ok_or_else(|| "truncated png".to_string())?;
        let data_at = cursor + 8;
        let data_end = data_at
            .checked_add(len_us)
            .ok_or_else(|| "png chunk overflows".to_string())?;
        let next = data_end
            .checked_add(4)
            .ok_or_else(|| "png chunk overflows".to_string())?;
        if next > bytes.len() {
            return Err("truncated png".to_string());
        }
        let data = &bytes[data_at..data_end];
        match kind {
            b"IHDR" => {
                let (w, h) = ihdr(data)?;
                width = Some(w);
                height = Some(h);
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        cursor = next;
    }
    let width = width.ok_or_else(|| "png has no IHDR".to_string())?;
    let height = height.ok_or_else(|| "png has no IHDR".to_string())?;
    let raw = inflate_stored(&idat)?;
    rgb_from_rows(width, height, &raw)
}

fn ihdr(data: &[u8]) -> Result<(u32, u32), String> {
    if data.len() < 13 {
        return Err("short IHDR".to_string());
    }
    let width = u32::from_be_bytes(copy4(data, 0)?);
    let height = u32::from_be_bytes(copy4(data, 4)?);
    let bit_depth = data[8];
    let color = data[9];
    let interlace = data[12];
    if bit_depth != 8 || color != 2 || interlace != 0 {
        return Err("png must be 8-bit RGB, not interlaced".to_string());
    }
    if width == 0 || height == 0 {
        return Err("png size is zero".to_string());
    }
    Ok((width, height))
}

fn rgb_from_rows(width: u32, height: u32, raw: &[u8]) -> Result<Picture, String> {
    let w = usize::try_from(width).map_err(|_| "png size overflow".to_string())?;
    let h = usize::try_from(height).map_err(|_| "png size overflow".to_string())?;
    let rgb_len = w
        .checked_mul(3)
        .ok_or_else(|| "png size overflow".to_string())?;
    let stride = rgb_len
        .checked_add(1)
        .ok_or_else(|| "png size overflow".to_string())?;
    let need = stride
        .checked_mul(h)
        .ok_or_else(|| "png size overflow".to_string())?;
    if raw.len() < need {
        return Err("png image data is short".to_string());
    }
    let mut linear = Vec::with_capacity(rgb_len.saturating_mul(h));
    for y in 0..h {
        let at = y * stride;
        if raw[at] != 0 {
            return Err("png row filter is not none".to_string());
        }
        for byte in &raw[at + 1..at + 1 + rgb_len] {
            linear.push(linear_from_display(f32::from(*byte) / 255.0));
        }
    }
    Picture::new(width, height, linear)
}

fn inflate_stored(zlib: &[u8]) -> Result<Vec<u8>, String> {
    if zlib.len() < 6 {
        return Err("short zlib stream".to_string());
    }
    let cmf = u32::from(zlib[0]);
    let flg = u32::from(zlib[1]);
    if (cmf * 256 + flg) % 31 != 0 || cmf & 0x0f != 8 {
        return Err("bad zlib header".to_string());
    }
    if flg & 0x20 != 0 {
        return Err("zlib preset dictionary".to_string());
    }
    let mut cursor = 2_usize;
    let mut out = Vec::new();
    loop {
        let hdr = *zlib
            .get(cursor)
            .ok_or_else(|| "truncated deflate".to_string())?;
        cursor += 1;
        let bfinal = hdr & 1;
        let btype = (hdr >> 1) & 3;
        if btype != 0 {
            return Err("png is compressed; captures are stored deflate".to_string());
        }
        if cursor + 4 > zlib.len() {
            return Err("truncated stored block".to_string());
        }
        let len = u16::from_le_bytes([zlib[cursor], zlib[cursor + 1]]);
        let nlen = u16::from_le_bytes([zlib[cursor + 2], zlib[cursor + 3]]);
        if nlen != !len {
            return Err("stored block length check failed".to_string());
        }
        cursor += 4;
        let n = usize::from(len);
        let end = cursor
            .checked_add(n)
            .ok_or_else(|| "stored block overflows".to_string())?;
        if end > zlib.len() {
            return Err("truncated stored block".to_string());
        }
        out.extend_from_slice(&zlib[cursor..end]);
        cursor = end;
        if bfinal == 1 {
            break;
        }
    }
    if cursor + 4 > zlib.len() {
        return Err("missing adler32".to_string());
    }
    let adler = u32::from_be_bytes(copy4(zlib, cursor)?);
    if adler32(&out) != adler {
        return Err("adler32 mismatch".to_string());
    }
    Ok(out)
}

fn adler32(data: &[u8]) -> u32 {
    let mut s1 = 1_u32;
    let mut s2 = 0_u32;
    for byte in data {
        s1 = (s1 + u32::from(*byte)) % 65_521;
        s2 = (s2 + s1) % 65_521;
    }
    (s2 << 16) | s1
}

fn be_u32(bytes: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_be_bytes(copy4(bytes, at)?))
}

fn copy4(bytes: &[u8], at: usize) -> Result<[u8; 4], String> {
    let slice = bytes
        .get(at..at + 4)
        .ok_or_else(|| "truncated png".to_string())?;
    slice.try_into().map_err(|_| "truncated png".to_string())
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = 0xffff_ffffu32;
    for byte in kind.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg() & 0xedb8_8320;
            crc = (crc >> 1) ^ mask;
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_round_trips_under_and_over_the_shoulder() {
        for x in [0.0f32, 0.2, 0.64, 0.8, 1.5, 4.0] {
            let back = linear_from_display(tone(x));
            assert!((back - x).abs() < 1.0e-4, "{x} -> {back}");
        }
    }

    #[test]
    fn linear_file_round_trips() {
        let picture = Picture::solid(2, 2, [0.1, 0.2, 0.3]);
        let path = std::env::temp_dir().join(format!("lref-{}-pic.f32", std::process::id()));
        picture.save_linear(&path).unwrap();
        let loaded = Picture::load_linear(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(loaded, picture);
    }

    #[test]
    fn png_round_trips_under_the_shoulder() {
        let picture = Picture::solid(2, 2, [0.2, 0.4, 0.6]);
        let path = std::env::temp_dir().join(format!("lref-{}-png.png", std::process::id()));
        picture.save_png(&path).unwrap();
        let loaded = Picture::load_png(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(loaded.width, 2);
        assert_eq!(loaded.height, 2);
        let px = loaded.pixel(1, 1);
        for (got, want) in px.into_iter().zip([0.2_f32, 0.4, 0.6]) {
            assert!((got - want).abs() < 1.0 / 255.0, "{got} vs {want}");
        }
    }

    #[test]
    fn png_starts_with_the_signature() {
        let picture = Picture::solid(1, 1, [0.5, 0.25, 0.0]);
        let png = encode_png(1, 1, &picture.toned_rgb8());
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn average_is_the_mean() {
        let a = Picture::solid(1, 1, [0.0, 0.0, 0.0]);
        let b = Picture::solid(1, 1, [1.0, 0.5, 0.0]);
        let mean = Picture::average(&[a, b]).unwrap();
        let px = mean.pixel(0, 0);
        assert!((px[0] - 0.5).abs() < 1.0e-6);
        assert!((px[1] - 0.25).abs() < 1.0e-6);
    }
}
