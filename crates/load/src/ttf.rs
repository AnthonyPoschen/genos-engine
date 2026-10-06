//! TrueType glyphs.
//!
//! `glyph` returns the horizontal advance and the simple-glyph outline in font
//! units. A composite glyph returns an error. Coordinates are the font's
//! design grid. The em square is `units_per_em` by `units_per_em`.

use crate::bin::{i16_be, u16_be, u32_be};
use crate::error::LoadError;

const ON_CURVE: u8 = 0x01;
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT: u8 = 0x08;
const X_SAME: u8 = 0x10;
const Y_SAME: u8 = 0x20;

/// One point on a glyph outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutlinePoint {
    pub x: i32,
    pub y: i32,
    /// `true` when the point sits on the curve. `false` is an off-curve control.
    pub on_curve: bool,
}

/// A character's advance and outline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// Horizontal advance in font units.
    pub advance: u16,
    /// Closed contours. Empty when the glyph has no outline.
    pub contours: Vec<Vec<OutlinePoint>>,
}

#[derive(Clone, Debug)]
enum Outline {
    Empty,
    Simple(Vec<Vec<OutlinePoint>>),
    Composite,
}

/// A TrueType font.
#[derive(Clone, Debug)]
pub struct Font {
    units_per_em: u16,
    advances: Vec<u16>,
    glyphs: Vec<Outline>,
    /// BMP code point to glyph id. Zero means the character is not in the font.
    bmp: Vec<u16>,
    extra: Vec<(u32, u32, u32)>,
}

impl Font {
    /// Design units on each side of the em square.
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// Outline and advance for `ch`.
    pub fn glyph(&self, ch: char) -> Result<Glyph, LoadError> {
        let id = self.glyph_id(ch as u32)?;
        let outline = self
            .glyphs
            .get(usize::from(id))
            .ok_or(LoadError::Unrecognized)?;
        let contours = match outline {
            Outline::Empty => Vec::new(),
            Outline::Simple(contours) => contours.clone(),
            Outline::Composite => return Err(LoadError::Unrecognized),
        };
        let advance = self
            .advances
            .get(usize::from(id))
            .copied()
            .ok_or(LoadError::Unrecognized)?;
        Ok(Glyph { advance, contours })
    }

    fn glyph_id(&self, code: u32) -> Result<u16, LoadError> {
        if code <= 0xffff {
            let id = self.bmp[code as usize];
            if id == 0 {
                return Err(LoadError::Unrecognized);
            }
            return Ok(id);
        }
        for &(start, end, glyph) in &self.extra {
            if code >= start && code <= end {
                let id = glyph + (code - start);
                if id == 0 || id > u32::from(u16::MAX) {
                    return Err(LoadError::Unrecognized);
                }
                return Ok(id as u16);
            }
        }
        Err(LoadError::Unrecognized)
    }
}

pub(crate) fn decode_font(data: &[u8]) -> Result<Font, LoadError> {
    if data.len() < 12 || u32_be(data, 0)? != 0x0001_0000 {
        return Err(LoadError::Unrecognized);
    }
    let num_tables = usize::from(u16_be(data, 4)?);
    if num_tables == 0 {
        return Err(LoadError::Unrecognized);
    }
    let directory_end = 12usize
        .checked_add(num_tables.checked_mul(16).ok_or(LoadError::Unrecognized)?)
        .ok_or(LoadError::Unrecognized)?;
    if directory_end > data.len() {
        return Err(LoadError::Truncated);
    }
    let mut found = Vec::with_capacity(num_tables);
    for index in 0..num_tables {
        let at = 12 + index * 16;
        let tag = [data[at], data[at + 1], data[at + 2], data[at + 3]];
        let offset = u32_be(data, at + 8)? as usize;
        let length = u32_be(data, at + 12)? as usize;
        let end = offset.checked_add(length).ok_or(LoadError::Unrecognized)?;
        if end > data.len() {
            return Err(LoadError::Truncated);
        }
        found.push((tag, offset, length));
    }
    let head = slice(data, &found, *b"head")?;
    let hhea = slice(data, &found, *b"hhea")?;
    let maxp = slice(data, &found, *b"maxp")?;
    let loca = slice(data, &found, *b"loca")?;
    let glyf = slice(data, &found, *b"glyf")?;
    let hmtx = slice(data, &found, *b"hmtx")?;
    let cmap = slice(data, &found, *b"cmap")?;
    if head.len() < 54 || u32_be(head, 12)? != 0x5f0f_3cf5 {
        return Err(LoadError::Unrecognized);
    }
    let units_per_em = u16_be(head, 18)?;
    let loca_format = i16_be(head, 50)?;
    if units_per_em == 0 || (loca_format != 0 && loca_format != 1) {
        return Err(LoadError::Unrecognized);
    }
    if maxp.len() < 6 {
        return Err(LoadError::Truncated);
    }
    let num_glyphs = usize::from(u16_be(maxp, 4)?);
    if num_glyphs == 0 {
        return Err(LoadError::Unrecognized);
    }
    if hhea.len() < 36 {
        return Err(LoadError::Truncated);
    }
    let hmetrics = usize::from(u16_be(hhea, 34)?);
    if hmetrics == 0 || hmetrics > num_glyphs {
        return Err(LoadError::Unrecognized);
    }
    let (bmp, extra) = read_cmap(cmap)?;
    Ok(Font {
        units_per_em,
        advances: read_advances(hmtx, num_glyphs, hmetrics)?,
        glyphs: read_glyphs(glyf, loca, num_glyphs, loca_format)?,
        bmp,
        extra,
    })
}

fn slice<'a>(
    data: &'a [u8],
    tables: &[([u8; 4], usize, usize)],
    tag: [u8; 4],
) -> Result<&'a [u8], LoadError> {
    let (offset, length) = tables
        .iter()
        .find(|(found, _, _)| *found == tag)
        .map(|(_, offset, length)| (*offset, *length))
        .ok_or(LoadError::Unrecognized)?;
    data.get(offset..offset + length)
        .ok_or(LoadError::Truncated)
}

fn read_advances(hmtx: &[u8], num_glyphs: usize, hmetrics: usize) -> Result<Vec<u16>, LoadError> {
    let needed = hmetrics * 4 + (num_glyphs - hmetrics) * 2;
    if hmtx.len() < needed {
        return Err(LoadError::Truncated);
    }
    let mut advances = Vec::with_capacity(num_glyphs);
    for index in 0..hmetrics {
        advances.push(u16_be(hmtx, index * 4)?);
    }
    let last = *advances.last().ok_or(LoadError::Unrecognized)?;
    while advances.len() < num_glyphs {
        advances.push(last);
    }
    Ok(advances)
}

fn read_glyphs(
    glyf: &[u8],
    loca: &[u8],
    num_glyphs: usize,
    loca_format: i16,
) -> Result<Vec<Outline>, LoadError> {
    let mut glyphs = Vec::with_capacity(num_glyphs);
    for index in 0..num_glyphs {
        let start = loca_offset(loca, loca_format, index)?;
        let end = loca_offset(loca, loca_format, index + 1)?;
        if end < start || end > glyf.len() {
            return Err(LoadError::Unrecognized);
        }
        glyphs.push(read_glyph(&glyf[start..end])?);
    }
    Ok(glyphs)
}

fn loca_offset(loca: &[u8], format: i16, index: usize) -> Result<usize, LoadError> {
    if format == 0 {
        let at = index.checked_mul(2).ok_or(LoadError::Unrecognized)?;
        Ok(usize::from(u16_be(loca, at)?) * 2)
    } else {
        let at = index.checked_mul(4).ok_or(LoadError::Unrecognized)?;
        Ok(u32_be(loca, at)? as usize)
    }
}

fn read_glyph(data: &[u8]) -> Result<Outline, LoadError> {
    if data.is_empty() {
        return Ok(Outline::Empty);
    }
    if data.len() < 10 {
        return Err(LoadError::Truncated);
    }
    let contours = i16_be(data, 0)?;
    if contours < 0 {
        return Ok(Outline::Composite);
    }
    if contours == 0 {
        return Ok(Outline::Empty);
    }
    let contour_count = contours as usize;
    let mut pos = 10usize;
    let ends_end = pos + contour_count * 2;
    if ends_end + 2 > data.len() {
        return Err(LoadError::Truncated);
    }
    let mut ends = Vec::with_capacity(contour_count);
    let mut previous = -1i32;
    for _ in 0..contour_count {
        let end = i32::from(u16_be(data, pos)?);
        pos += 2;
        if end <= previous {
            return Err(LoadError::Unrecognized);
        }
        previous = end;
        ends.push(end as usize);
    }
    let point_count = previous as usize + 1;
    let instruction_len = usize::from(u16_be(data, pos)?);
    pos += 2;
    pos = pos
        .checked_add(instruction_len)
        .ok_or(LoadError::Unrecognized)?;
    if pos > data.len() {
        return Err(LoadError::Truncated);
    }
    let mut flags = Vec::with_capacity(point_count);
    while flags.len() < point_count {
        let flag = *data.get(pos).ok_or(LoadError::Truncated)?;
        pos += 1;
        let repeat = if flag & REPEAT != 0 {
            let count = *data.get(pos).ok_or(LoadError::Truncated)?;
            pos += 1;
            usize::from(count)
        } else {
            0
        };
        if flags.len() + 1 + repeat > point_count {
            return Err(LoadError::Unrecognized);
        }
        flags.push(flag);
        for _ in 0..repeat {
            flags.push(flag);
        }
    }
    let x = deltas(data, &mut pos, &flags, X_SHORT, X_SAME)?;
    let y = deltas(data, &mut pos, &flags, Y_SHORT, Y_SAME)?;
    let mut points = Vec::with_capacity(point_count);
    let mut absolute_x = 0i32;
    let mut absolute_y = 0i32;
    for index in 0..point_count {
        absolute_x += x[index];
        absolute_y += y[index];
        points.push(OutlinePoint {
            x: absolute_x,
            y: absolute_y,
            on_curve: flags[index] & ON_CURVE != 0,
        });
    }
    let mut contours_out = Vec::with_capacity(ends.len());
    let mut start = 0usize;
    for end in ends {
        contours_out.push(points[start..=end].to_vec());
        start = end + 1;
    }
    Ok(Outline::Simple(contours_out))
}

fn deltas(
    data: &[u8],
    pos: &mut usize,
    flags: &[u8],
    short_bit: u8,
    same_bit: u8,
) -> Result<Vec<i32>, LoadError> {
    let mut values = Vec::with_capacity(flags.len());
    for &flag in flags {
        let delta = if flag & short_bit != 0 {
            let byte = *data.get(*pos).ok_or(LoadError::Truncated)?;
            *pos += 1;
            if flag & same_bit != 0 {
                i32::from(byte)
            } else {
                -i32::from(byte)
            }
        } else if flag & same_bit != 0 {
            0
        } else {
            let value = i32::from(i16_be(data, *pos)?);
            *pos += 2;
            value
        };
        values.push(delta);
    }
    Ok(values)
}

fn read_cmap(cmap: &[u8]) -> Result<(Vec<u16>, Vec<(u32, u32, u32)>), LoadError> {
    if cmap.len() < 4 {
        return Err(LoadError::Truncated);
    }
    if u16_be(cmap, 0)? != 0 {
        return Err(LoadError::Unrecognized);
    }
    let records = usize::from(u16_be(cmap, 2)?);
    let directory = 4 + records * 8;
    if directory > cmap.len() {
        return Err(LoadError::Truncated);
    }
    let mut chosen: Option<(u16, u16, usize)> = None;
    for index in 0..records {
        let at = 4 + index * 8;
        let platform = u16_be(cmap, at)?;
        let encoding = u16_be(cmap, at + 2)?;
        let offset = u32_be(cmap, at + 4)? as usize;
        if offset >= cmap.len() {
            return Err(LoadError::Truncated);
        }
        let format = u16_be(cmap, offset)?;
        let rank = cmap_rank(platform, encoding, format);
        if rank == 0 {
            continue;
        }
        let replace = match chosen {
            Some((best, _, _)) => rank < best,
            None => true,
        };
        if replace {
            chosen = Some((rank, format, offset));
        }
    }
    let (_, format, offset) = chosen.ok_or(LoadError::Unrecognized)?;
    let sub = &cmap[offset..];
    if format == 4 {
        Ok((read_format4(sub)?, Vec::new()))
    } else if format == 12 {
        read_format12(sub)
    } else {
        Err(LoadError::Unrecognized)
    }
}

fn cmap_rank(platform: u16, encoding: u16, format: u16) -> u16 {
    match (platform, encoding, format) {
        (3, 10, 12) | (0, 4, 12) => 1,
        (3, 1, 4) | (0, 3, 4) => 2,
        (_, _, 12) => 3,
        (_, _, 4) => 4,
        _ => 0,
    }
}

fn read_format4(sub: &[u8]) -> Result<Vec<u16>, LoadError> {
    if sub.len() < 14 || u16_be(sub, 0)? != 4 {
        return Err(LoadError::Unrecognized);
    }
    let length = usize::from(u16_be(sub, 2)?);
    if length > sub.len() || length < 14 {
        return Err(LoadError::Truncated);
    }
    let sub = &sub[..length];
    let seg_count_x2 = usize::from(u16_be(sub, 6)?);
    if seg_count_x2 % 2 != 0 || seg_count_x2 == 0 {
        return Err(LoadError::Unrecognized);
    }
    let seg_count = seg_count_x2 / 2;
    let end_at = 14usize;
    let reserved_at = end_at + seg_count * 2;
    let start_at = reserved_at + 2;
    let delta_at = start_at + seg_count * 2;
    let offset_at = delta_at + seg_count * 2;
    if offset_at + seg_count * 2 > sub.len() {
        return Err(LoadError::Truncated);
    }
    if u16_be(sub, reserved_at)? != 0 {
        return Err(LoadError::Unrecognized);
    }
    let mut map = vec![0u16; 65536];
    for index in 0..seg_count {
        let end = u16_be(sub, end_at + index * 2)?;
        let start = u16_be(sub, start_at + index * 2)?;
        let delta = i16_be(sub, delta_at + index * 2)?;
        let range_offset = u16_be(sub, offset_at + index * 2)?;
        if start > end {
            return Err(LoadError::Unrecognized);
        }
        let mut code = u32::from(start);
        let last = u32::from(end);
        loop {
            let glyph = if range_offset == 0 {
                (code as i32).wrapping_add(i32::from(delta)) as u16
            } else {
                let entry = offset_at + index * 2;
                let at =
                    entry + usize::from(range_offset) + ((code - u32::from(start)) as usize) * 2;
                if at + 2 > sub.len() {
                    return Err(LoadError::Truncated);
                }
                let glyph_id = u16_be(sub, at)?;
                if glyph_id == 0 {
                    0
                } else {
                    (i32::from(glyph_id).wrapping_add(i32::from(delta))) as u16
                }
            };
            map[code as usize] = glyph;
            if code == last {
                break;
            }
            code += 1;
        }
    }
    Ok(map)
}

fn read_format12(sub: &[u8]) -> Result<(Vec<u16>, Vec<(u32, u32, u32)>), LoadError> {
    if sub.len() < 16 || u16_be(sub, 0)? != 12 {
        return Err(LoadError::Unrecognized);
    }
    let length = u32_be(sub, 4)? as usize;
    if length > sub.len() || length < 16 {
        return Err(LoadError::Truncated);
    }
    let groups = u32_be(sub, 12)? as usize;
    let needed = 16usize
        .checked_add(groups.checked_mul(12).ok_or(LoadError::Unrecognized)?)
        .ok_or(LoadError::Unrecognized)?;
    if needed > length {
        return Err(LoadError::Truncated);
    }
    let mut map = vec![0u16; 65536];
    let mut extra = Vec::new();
    for index in 0..groups {
        let at = 16 + index * 12;
        let start = u32_be(sub, at)?;
        let end = u32_be(sub, at + 4)?;
        let glyph = u32_be(sub, at + 8)?;
        if start > end {
            return Err(LoadError::Unrecognized);
        }
        if start <= 0xffff {
            let mut code = start;
            let bmp_end = end.min(0xffff);
            while code <= bmp_end {
                let id = glyph + (code - start);
                if id <= u32::from(u16::MAX) {
                    map[code as usize] = id as u16;
                }
                code += 1;
            }
        }
        if end > 0xffff {
            let extra_start = start.max(0x1_0000);
            extra.push((extra_start, end, glyph + (extra_start - start)));
        }
    }
    Ok((map, extra))
}
