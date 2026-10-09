//! JPEG images (JFIF and EXIF): baseline and progressive Huffman, 8-bit samples.
//!
//! Greyscale and three-component YCbCr images are accepted, with any chroma
//! subsampling up to 4x in each direction and with restart markers. Pixels come
//! back as RGBA8 with alpha 255, row-major, top to bottom, like a PNG.
//! Arithmetic coding, lossless and hierarchical JPEG, 12-bit samples and
//! CMYK are not decoded. Each edge is at most 8192 pixels.

use crate::error::LoadError;
use crate::png::Image;

const MAX_EDGE: usize = 8192;

/// Natural (row-major) index of each zig-zag position.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27,
    20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58,
    59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[derive(Clone, Default)]
struct Huffman {
    /// (code length, code) -> symbol, via per-length tables.
    max_code: [i32; 18],
    val_offset: [i32; 18],
    values: Vec<u8>,
    /// Fast lookup of the first 9 bits: (length << 8) | symbol, 0 when longer.
    fast: Vec<u16>,
}

const FAST_BITS: u32 = 9;

impl Huffman {
    fn new(counts: &[u8; 16], values: Vec<u8>) -> Result<Self, LoadError> {
        let mut h = Huffman {
            max_code: [-1; 18],
            val_offset: [0; 18],
            values,
            fast: vec![0; 1 << FAST_BITS],
        };
        let mut code: i32 = 0;
        let mut k: i32 = 0;
        for len in 1..=16usize {
            let n = counts[len - 1] as i32;
            h.val_offset[len] = k - code;
            for _ in 0..n {
                if (k as usize) >= h.values.len() {
                    return Err(LoadError::Unrecognized);
                }
                if len as u32 <= FAST_BITS {
                    let shift = FAST_BITS - len as u32;
                    let base = (code as usize) << shift;
                    for fill in 0..(1usize << shift) {
                        h.fast[base + fill] = ((len as u16) << 8) | h.values[k as usize] as u16;
                    }
                }
                code += 1;
                k += 1;
            }
            h.max_code[len] = if n > 0 { code - 1 } else { -1 };
            code <<= 1;
        }
        h.max_code[17] = i32::MAX;
        Ok(h)
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    count: u32,
    /// A marker was met; further reads return zero bits.
    marker: bool,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Bits { data, pos, acc: 0, count: 0, marker: false }
    }

    fn fill(&mut self) {
        while self.count <= 56 {
            let mut byte = 0u8;
            if !self.marker && self.pos < self.data.len() {
                byte = self.data[self.pos];
                if byte == 0xFF {
                    let next = self.data.get(self.pos + 1).copied().unwrap_or(0xD9);
                    if next == 0x00 {
                        self.pos += 2;
                    } else {
                        self.marker = true;
                        byte = 0;
                    }
                } else {
                    self.pos += 1;
                }
            }
            self.acc |= (byte as u64) << (56 - self.count);
            self.count += 8;
        }
    }

    fn peek(&mut self, n: u32) -> u32 {
        if self.count < n {
            self.fill();
        }
        (self.acc >> (64 - n)) as u32
    }

    fn skip(&mut self, n: u32) {
        self.acc <<= n;
        self.count -= n;
    }

    fn bit(&mut self) -> u32 {
        let b = self.peek(1);
        self.skip(1);
        b
    }

    fn bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek(n);
        self.skip(n);
        v
    }

    /// Value of `n` bits as a signed JPEG magnitude category.
    fn receive_extend(&mut self, n: u32) -> i32 {
        if n == 0 {
            return 0;
        }
        let v = self.bits(n) as i32;
        if v < (1 << (n - 1)) {
            v - (1 << n) + 1
        } else {
            v
        }
    }

    fn decode(&mut self, h: &Huffman) -> Result<u8, LoadError> {
        if self.count < 16 {
            self.fill();
        }
        let fast = h.fast[self.peek(FAST_BITS) as usize];
        if fast != 0 {
            self.skip((fast >> 8) as u32);
            return Ok(fast as u8);
        }
        let mut code: i32 = 0;
        for len in 1..=16usize {
            code = (code << 1) | self.bit() as i32;
            if code <= h.max_code[len] {
                let index = (code + h.val_offset[len]) as usize;
                return h.values.get(index).copied().ok_or(LoadError::Unrecognized);
            }
        }
        Err(LoadError::Unrecognized)
    }

    /// Drop to the next byte boundary and step over an RSTn marker.
    fn restart(&mut self) {
        self.acc = 0;
        self.count = 0;
        self.marker = false;
        while self.pos + 1 < self.data.len() {
            if self.data[self.pos] == 0xFF && (0xD0..=0xD7).contains(&self.data[self.pos + 1]) {
                self.pos += 2;
                return;
            }
            self.pos += 1;
        }
    }

    /// Byte position after this scan's entropy data: the next marker.
    fn end(&self) -> usize {
        let mut p = self.pos;
        while p + 1 < self.data.len() {
            if self.data[p] == 0xFF && self.data[p + 1] != 0x00 && !(0xD0..=0xD7).contains(&self.data[p + 1]) {
                return p;
            }
            p += 1;
        }
        self.data.len()
    }
}

struct Component {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    /// Blocks per row and rows of blocks, padded to whole MCUs.
    bw: usize,
    bh: usize,
    /// Blocks that hold image data (a non-interleaved scan covers only these).
    bw_used: usize,
    bh_used: usize,
    coeffs: Vec<[i32; 64]>,
    dc_pred: i32,
    dc_table: usize,
    ac_table: usize,
}

fn u16_at(data: &[u8], at: usize) -> Result<usize, LoadError> {
    let b = data.get(at..at + 2).ok_or(LoadError::Truncated)?;
    Ok(((b[0] as usize) << 8) | b[1] as usize)
}

pub(crate) fn decode_jpeg(data: &[u8]) -> Result<Image, LoadError> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(LoadError::Unrecognized);
    }
    let mut quant = [[1u16; 64]; 4];
    let mut dc_tables: [Option<Huffman>; 4] = Default::default();
    let mut ac_tables: [Option<Huffman>; 4] = Default::default();
    let mut comps: Vec<Component> = Vec::new();
    let (mut width, mut height) = (0usize, 0usize);
    let (mut hmax, mut vmax) = (1usize, 1usize);
    let (mut mcux, mut mcuy) = (0usize, 0usize);
    let mut progressive = false;
    let mut restart_interval = 0usize;
    let mut adobe_transform: Option<u8> = None;
    let mut pos = 2;
    loop {
        // Find the next marker.
        while pos < data.len() && data[pos] != 0xFF {
            pos += 1;
        }
        while pos < data.len() && data[pos] == 0xFF {
            pos += 1;
        }
        let marker = *data.get(pos).ok_or(LoadError::Truncated)?;
        pos += 1;
        match marker {
            0xD9 => break,
            0xD0..=0xD7 | 0x01 => continue,
            _ => {}
        }
        let len = u16_at(data, pos)?;
        if len < 2 {
            return Err(LoadError::Unrecognized);
        }
        let seg = data.get(pos + 2..pos + len).ok_or(LoadError::Truncated)?;
        match marker {
            0xDB => {
                let mut i = 0;
                while i < seg.len() {
                    let pq = seg[i] >> 4;
                    let tq = (seg[i] & 15) as usize;
                    if tq > 3 {
                        return Err(LoadError::Unrecognized);
                    }
                    i += 1;
                    for (k, &zz) in ZIGZAG.iter().enumerate() {
                        let v = if pq == 0 {
                            *seg.get(i + k).ok_or(LoadError::Truncated)? as u16
                        } else {
                            u16_at(seg, i + 2 * k)? as u16
                        };
                        quant[tq][zz] = v;
                    }
                    i += if pq == 0 { 64 } else { 128 };
                }
            }
            0xC4 => {
                let mut i = 0;
                while i < seg.len() {
                    let tc = seg[i] >> 4;
                    let th = (seg[i] & 15) as usize;
                    if th > 3 || tc > 1 {
                        return Err(LoadError::Unrecognized);
                    }
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(seg.get(i + 1..i + 17).ok_or(LoadError::Truncated)?);
                    let total: usize = counts.iter().map(|&c| c as usize).sum();
                    let values = seg.get(i + 17..i + 17 + total).ok_or(LoadError::Truncated)?.to_vec();
                    let table = Huffman::new(&counts, values)?;
                    if tc == 0 {
                        dc_tables[th] = Some(table);
                    } else {
                        ac_tables[th] = Some(table);
                    }
                    i += 17 + total;
                }
            }
            0xC0..=0xC2 => {
                progressive = marker == 0xC2;
                if *seg.first().ok_or(LoadError::Truncated)? != 8 {
                    return Err(LoadError::Unrecognized);
                }
                height = u16_at(seg, 1)?;
                width = u16_at(seg, 3)?;
                let n = *seg.get(5).ok_or(LoadError::Truncated)? as usize;
                if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE || !(n == 1 || n == 3) {
                    return Err(LoadError::Unrecognized);
                }
                for c in 0..n {
                    let b = seg.get(6 + c * 3..9 + c * 3).ok_or(LoadError::Truncated)?;
                    let (h, v) = ((b[1] >> 4) as usize, (b[1] & 15) as usize);
                    if !(1..=4).contains(&h) || !(1..=4).contains(&v) || b[2] > 3 {
                        return Err(LoadError::Unrecognized);
                    }
                    comps.push(Component {
                        id: b[0],
                        h,
                        v,
                        tq: b[2] as usize,
                        bw: 0,
                        bh: 0,
                        bw_used: 0,
                        bh_used: 0,
                        coeffs: Vec::new(),
                        dc_pred: 0,
                        dc_table: 0,
                        ac_table: 0,
                    });
                }
                hmax = comps.iter().map(|c| c.h).max().unwrap_or(1);
                vmax = comps.iter().map(|c| c.v).max().unwrap_or(1);
                mcux = width.div_ceil(8 * hmax);
                mcuy = height.div_ceil(8 * vmax);
                for c in comps.iter_mut() {
                    c.bw = mcux * c.h;
                    c.bh = mcuy * c.v;
                    c.bw_used = (width * c.h).div_ceil(hmax).div_ceil(8);
                    c.bh_used = (height * c.v).div_ceil(vmax).div_ceil(8);
                    c.coeffs = vec![[0; 64]; c.bw * c.bh];
                }
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return Err(LoadError::Unrecognized),
            0xDD => restart_interval = u16_at(seg, 0)?,
            0xEE => {
                if seg.len() >= 12 && &seg[..5] == b"Adobe" {
                    adobe_transform = Some(seg[11]);
                }
            }
            0xDA => {
                if comps.is_empty() {
                    return Err(LoadError::Unrecognized);
                }
                let ns = *seg.first().ok_or(LoadError::Truncated)? as usize;
                let mut scan = Vec::with_capacity(ns);
                for i in 0..ns {
                    let b = seg.get(1 + i * 2..3 + i * 2).ok_or(LoadError::Truncated)?;
                    let ci = comps.iter().position(|c| c.id == b[0]).ok_or(LoadError::Unrecognized)?;
                    comps[ci].dc_table = (b[1] >> 4) as usize & 3;
                    comps[ci].ac_table = (b[1] & 15) as usize & 3;
                    scan.push(ci);
                }
                let tail = seg.get(1 + ns * 2..4 + ns * 2).ok_or(LoadError::Truncated)?;
                let (ss, se, ah, al) = (tail[0] as usize, tail[1] as usize, (tail[2] >> 4) as u32, (tail[2] & 15) as u32);
                if ss > 63 || se > 63 || ss > se && progressive {
                    return Err(LoadError::Unrecognized);
                }
                let start = pos + len;
                let mut bits = Bits::new(data, start);
                let p = ScanParams { ss, se: if progressive { se } else { 63 }, ah, al, progressive };
                decode_scan(&mut bits, &mut comps, &scan, &dc_tables, &ac_tables, p, restart_interval, mcux, mcuy)?;
                pos = bits.end();
                continue;
            }
            _ => {}
        }
        pos += len;
    }
    if comps.is_empty() {
        return Err(LoadError::Unrecognized);
    }
    Ok(to_image(&comps, &quant, width, height, hmax, vmax, adobe_transform))
}

#[derive(Clone, Copy)]
struct ScanParams {
    ss: usize,
    se: usize,
    ah: u32,
    al: u32,
    progressive: bool,
}

#[allow(clippy::too_many_arguments)]
fn decode_scan(
    bits: &mut Bits,
    comps: &mut [Component],
    scan: &[usize],
    dc_tables: &[Option<Huffman>; 4],
    ac_tables: &[Option<Huffman>; 4],
    p: ScanParams,
    restart_interval: usize,
    mcux: usize,
    mcuy: usize,
) -> Result<(), LoadError> {
    for &ci in scan {
        comps[ci].dc_pred = 0;
    }
    let mut eobrun = 0u32;
    let single = scan.len() == 1;
    let (units_x, units_y) = if single {
        (comps[scan[0]].bw_used, comps[scan[0]].bh_used)
    } else {
        (mcux, mcuy)
    };
    let total = units_x * units_y;
    for unit in 0..total {
        if restart_interval > 0 && unit > 0 && unit % restart_interval == 0 {
            bits.restart();
            for &ci in scan {
                comps[ci].dc_pred = 0;
            }
            eobrun = 0;
        }
        let (ux, uy) = (unit % units_x, unit / units_x);
        for &ci in scan {
            let (h, v) = if single { (1, 1) } else { (comps[ci].h, comps[ci].v) };
            for by in 0..v {
                for bx in 0..h {
                    let (x, y) = if single { (ux, uy) } else { (ux * comps[ci].h + bx, uy * comps[ci].v + by) };
                    let c = &mut comps[ci];
                    let index = y * c.bw + x;
                    let dc = dc_tables[c.dc_table].as_ref();
                    let ac = ac_tables[c.ac_table].as_ref();
                    let mut block = c.coeffs[index];
                    let mut pred = c.dc_pred;
                    decode_block(bits, &mut block, &mut pred, dc, ac, p, &mut eobrun)?;
                    c.dc_pred = pred;
                    c.coeffs[index] = block;
                }
            }
        }
    }
    Ok(())
}

/// Coefficients stay in zig-zag order until dequantization.
fn decode_block(
    bits: &mut Bits,
    block: &mut [i32; 64],
    pred: &mut i32,
    dc: Option<&Huffman>,
    ac: Option<&Huffman>,
    p: ScanParams,
    eobrun: &mut u32,
) -> Result<(), LoadError> {
    if !p.progressive {
        let dc = dc.ok_or(LoadError::Unrecognized)?;
        let ac = ac.ok_or(LoadError::Unrecognized)?;
        let t = bits.decode(dc)? as u32;
        *pred += bits.receive_extend(t);
        block[0] = *pred;
        let mut k = 1;
        while k < 64 {
            let rs = bits.decode(ac)?;
            let (r, s) = ((rs >> 4) as usize, (rs & 15) as u32);
            if s == 0 {
                if r == 15 {
                    k += 16;
                    continue;
                }
                break;
            }
            k += r;
            if k > 63 {
                return Err(LoadError::Unrecognized);
            }
            block[k] = bits.receive_extend(s);
            k += 1;
        }
        return Ok(());
    }
    if p.ss == 0 {
        // DC scans (spectral selection is DC only).
        if p.ah == 0 {
            let dc = dc.ok_or(LoadError::Unrecognized)?;
            let t = bits.decode(dc)? as u32;
            *pred += bits.receive_extend(t);
            block[0] = *pred << p.al;
        } else if bits.bit() != 0 {
            block[0] |= 1 << p.al;
        }
        return Ok(());
    }
    let ac = ac.ok_or(LoadError::Unrecognized)?;
    if p.ah == 0 {
        // AC first pass.
        if *eobrun > 0 {
            *eobrun -= 1;
            return Ok(());
        }
        let mut k = p.ss;
        while k <= p.se {
            let rs = bits.decode(ac)?;
            let (r, s) = ((rs >> 4) as u32, (rs & 15) as u32);
            if s == 0 {
                if r < 15 {
                    *eobrun = (1 << r) - 1 + bits.bits(r);
                    break;
                }
                k += 16;
                continue;
            }
            k += r as usize;
            if k > 63 {
                return Err(LoadError::Unrecognized);
            }
            block[k] = bits.receive_extend(s) * (1 << p.al);
            k += 1;
        }
        return Ok(());
    }
    // AC refinement.
    let p1 = 1i32 << p.al;
    let m1 = -1i32 << p.al;
    let mut k = p.ss;
    if *eobrun == 0 {
        while k <= p.se {
            let rs = bits.decode(ac)?;
            let (mut r, s) = ((rs >> 4) as i32, (rs & 15) as u32);
            let mut value = 0;
            if s == 0 {
                if r < 15 {
                    *eobrun = (1 << r) + bits.bits(r as u32);
                    break;
                }
            } else {
                value = if bits.bit() != 0 { p1 } else { m1 };
            }
            while k <= p.se {
                if block[k] != 0 {
                    if bits.bit() != 0 && (block[k] & p1) == 0 {
                        block[k] += if block[k] >= 0 { p1 } else { m1 };
                    }
                } else {
                    if r == 0 {
                        if value != 0 {
                            block[k] = value;
                        }
                        k += 1;
                        break;
                    }
                    r -= 1;
                }
                k += 1;
            }
        }
    }
    if *eobrun > 0 {
        while k <= p.se {
            if block[k] != 0 && bits.bit() != 0 && (block[k] & p1) == 0 {
                block[k] += if block[k] >= 0 { p1 } else { m1 };
            }
            k += 1;
        }
        *eobrun -= 1;
    }
    Ok(())
}

fn idct_block(coeffs: &[i32; 64], q: &[u16; 64], out: &mut [u8; 64]) {
    // Dequantize into natural order, then a separable float IDCT.
    let mut f = [0f32; 64];
    for k in 0..64 {
        f[ZIGZAG[k]] = coeffs[k] as f32 * q[ZIGZAG[k]] as f32;
    }
    let mut tmp = [0f32; 64];
    let c = |u: usize| if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
    let mut cos = [[0f32; 8]; 8];
    for (x, row) in cos.iter_mut().enumerate() {
        for (u, v) in row.iter_mut().enumerate() {
            *v = c(u) * (((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI) / 16.0).cos();
        }
    }
    for y in 0..8 {
        for x in 0..8 {
            let mut s = 0.0;
            for u in 0..8 {
                s += cos[x][u] * f[y * 8 + u];
            }
            tmp[y * 8 + x] = s / 2.0;
        }
    }
    for x in 0..8 {
        for y in 0..8 {
            let mut s = 0.0;
            for v in 0..8 {
                s += cos[y][v] * tmp[v * 8 + x];
            }
            out[y * 8 + x] = (s / 2.0 + 128.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

fn to_image(
    comps: &[Component],
    quant: &[[u16; 64]; 4],
    width: usize,
    height: usize,
    hmax: usize,
    vmax: usize,
    adobe_transform: Option<u8>,
) -> Image {
    let planes: Vec<(Vec<u8>, usize)> = comps
        .iter()
        .map(|c| {
            let pw = c.bw * 8;
            let mut plane = vec![0u8; pw * c.bh * 8];
            let mut out = [0u8; 64];
            for by in 0..c.bh {
                for bx in 0..c.bw {
                    idct_block(&c.coeffs[by * c.bw + bx], &quant[c.tq], &mut out);
                    for y in 0..8 {
                        let row = (by * 8 + y) * pw + bx * 8;
                        plane[row..row + 8].copy_from_slice(&out[y * 8..y * 8 + 8]);
                    }
                }
            }
            (plane, pw)
        })
        .collect();
    let mut pixels = vec![255u8; width * height * 4];
    let rgb_direct = adobe_transform == Some(0);
    for y in 0..height {
        for x in 0..width {
            // Subsampled components are interpolated between sample centres (as
            // libjpeg's "fancy" upsampling does), clamped at the image edge.
            let sample = |i: usize| {
                let c = &comps[i];
                let (plane, pw) = (&planes[i].0, planes[i].1);
                if c.h == hmax && c.v == vmax {
                    return plane[y * pw + x] as f32;
                }
                let cw = (width * c.h).div_ceil(hmax);
                let ch = (height * c.v).div_ceil(vmax);
                let fx = ((x as f32 + 0.5) * c.h as f32 / hmax as f32 - 0.5).clamp(0.0, (cw - 1) as f32);
                let fy = ((y as f32 + 0.5) * c.v as f32 / vmax as f32 - 0.5).clamp(0.0, (ch - 1) as f32);
                let (x0, y0) = (fx as usize, fy as usize);
                let (x1, y1) = ((x0 + 1).min(cw - 1), (y0 + 1).min(ch - 1));
                let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                let at = |xx: usize, yy: usize| plane[yy * pw + xx] as f32;
                let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
                let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
                top * (1.0 - ty) + bottom * ty
            };
            let o = (y * width + x) * 4;
            if comps.len() == 1 {
                let l = sample(0) as u8;
                pixels[o..o + 3].copy_from_slice(&[l, l, l]);
            } else if rgb_direct {
                pixels[o] = sample(0) as u8;
                pixels[o + 1] = sample(1) as u8;
                pixels[o + 2] = sample(2) as u8;
            } else {
                let (yy, cb, cr) = (sample(0), sample(1) - 128.0, sample(2) - 128.0);
                let r = yy + 1.402 * cr;
                let g = yy - 0.344136 * cb - 0.714136 * cr;
                let b = yy + 1.772 * cb;
                pixels[o] = r.round().clamp(0.0, 255.0) as u8;
                pixels[o + 1] = g.round().clamp(0.0, 255.0) as u8;
                pixels[o + 2] = b.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Image {
        width: width as u32,
        height: height as u32,
        pixels,
    }
}
