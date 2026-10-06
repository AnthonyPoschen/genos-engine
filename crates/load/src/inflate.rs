//! Zlib-wrapped deflate (RFC 1950 and RFC 1951).
//!
//! Huffman codes are packed most-significant bit first. Every other field is
//! packed least-significant bit first. Bytes themselves start at the
//! least-significant bit.

use crate::error::LoadError;

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u8,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bit: 0,
        }
    }

    fn one(&mut self) -> Result<i32, LoadError> {
        if self.pos >= self.data.len() {
            return Err(LoadError::Truncated);
        }
        let bit = ((self.data[self.pos] >> self.bit) & 1) as i32;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.pos += 1;
        }
        Ok(bit)
    }

    fn bits(&mut self, n: u32) -> Result<u32, LoadError> {
        if n > 16 {
            return Err(LoadError::Unrecognized);
        }
        let mut value = 0u32;
        for i in 0..n {
            value |= (self.one()? as u32) << i;
        }
        Ok(value)
    }

    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }

    fn read(&mut self, n: usize) -> Result<&'a [u8], LoadError> {
        self.align();
        let end = self.pos.checked_add(n).ok_or(LoadError::Unrecognized)?;
        if end > self.data.len() {
            return Err(LoadError::Truncated);
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn finished(&self) -> bool {
        self.pos == self.data.len() && self.bit == 0
    }
}

struct Huffman {
    counts: [i32; 16],
    symbols: Vec<u16>,
}

fn construct(lengths: &[u8]) -> Result<Huffman, LoadError> {
    let mut counts = [0i32; 16];
    for &len in lengths {
        if len > 15 {
            return Err(LoadError::Unrecognized);
        }
        counts[len as usize] += 1;
    }
    if counts[0] as usize == lengths.len() {
        return Ok(Huffman {
            counts,
            symbols: Vec::new(),
        });
    }
    let mut left = 1i32;
    for len in 1..=15 {
        left <<= 1;
        left -= counts[len];
        if left < 0 {
            return Err(LoadError::Unrecognized);
        }
    }
    let mut offsets = [0usize; 16];
    for len in 1..15 {
        offsets[len + 1] = offsets[len] + counts[len] as usize;
    }
    let symbol_count = lengths.len() - counts[0] as usize;
    let mut symbols = vec![0u16; symbol_count];
    for (symbol, &len) in lengths.iter().enumerate() {
        if len != 0 {
            let slot = offsets[len as usize];
            if slot >= symbols.len() {
                return Err(LoadError::Unrecognized);
            }
            symbols[slot] = symbol as u16;
            offsets[len as usize] = slot + 1;
        }
    }
    Ok(Huffman { counts, symbols })
}

fn decode(bits: &mut Bits<'_>, huff: &Huffman) -> Result<u16, LoadError> {
    if huff.symbols.is_empty() {
        return Err(LoadError::Unrecognized);
    }
    let mut code = 0i32;
    let mut first = 0i32;
    let mut index = 0usize;
    for len in 1..=15 {
        code |= bits.one()?;
        let count = huff.counts[len];
        if code - count < first {
            let symbol_index = index + (code - first) as usize;
            return huff
                .symbols
                .get(symbol_index)
                .copied()
                .ok_or(LoadError::Unrecognized);
        }
        index += count as usize;
        first += count;
        first <<= 1;
        code <<= 1;
    }
    Err(LoadError::Unrecognized)
}

fn fixed_pair() -> Result<(Huffman, Huffman), LoadError> {
    let mut lit = vec![8u8; 288];
    for len in lit.iter_mut().take(144) {
        *len = 8;
    }
    for len in lit.iter_mut().take(256).skip(144) {
        *len = 9;
    }
    for len in lit.iter_mut().take(280).skip(256) {
        *len = 7;
    }
    for len in lit.iter_mut().skip(280) {
        *len = 8;
    }
    Ok((construct(&lit)?, construct(&[5u8; 32])?))
}

fn dynamic_pair(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), LoadError> {
    let nlit = bits.bits(5)? as usize + 257;
    let ndist = bits.bits(5)? as usize + 1;
    let nlen = bits.bits(4)? as usize + 4;
    if nlit > 286 || ndist > 32 || nlen > 19 {
        return Err(LoadError::Unrecognized);
    }
    let mut cl_lengths = [0u8; 19];
    for slot in cl_lengths.iter_mut().take(nlen) {
        *slot = 0;
    }
    for i in 0..nlen {
        cl_lengths[CL_ORDER[i]] = bits.bits(3)? as u8;
    }
    let cl_huff = construct(&cl_lengths)?;
    let mut lengths = vec![0u8; nlit + ndist];
    let mut filled = 0usize;
    while filled < lengths.len() {
        let symbol = decode(bits, &cl_huff)?;
        match symbol {
            0..=15 => {
                lengths[filled] = symbol as u8;
                filled += 1;
            }
            16 => {
                if filled == 0 {
                    return Err(LoadError::Unrecognized);
                }
                let repeat = 3 + bits.bits(2)? as usize;
                let previous = lengths[filled - 1];
                if filled + repeat > lengths.len() {
                    return Err(LoadError::Unrecognized);
                }
                for _ in 0..repeat {
                    lengths[filled] = previous;
                    filled += 1;
                }
            }
            17 => {
                let repeat = 3 + bits.bits(3)? as usize;
                if filled + repeat > lengths.len() {
                    return Err(LoadError::Unrecognized);
                }
                filled += repeat;
            }
            18 => {
                let repeat = 11 + bits.bits(7)? as usize;
                if filled + repeat > lengths.len() {
                    return Err(LoadError::Unrecognized);
                }
                filled += repeat;
            }
            _ => return Err(LoadError::Unrecognized),
        }
    }
    Ok((construct(&lengths[..nlit])?, construct(&lengths[nlit..])?))
}

fn push(out: &mut Vec<u8>, limit: usize, byte: u8) -> Result<(), LoadError> {
    if out.len() >= limit {
        return Err(LoadError::Unrecognized);
    }
    out.push(byte);
    Ok(())
}

fn copy_match(out: &mut Vec<u8>, limit: usize, dist: usize, len: usize) -> Result<(), LoadError> {
    if dist == 0 || dist > out.len() {
        return Err(LoadError::Unrecognized);
    }
    for _ in 0..len {
        let byte = out[out.len() - dist];
        push(out, limit, byte)?;
    }
    Ok(())
}

fn length_from(bits: &mut Bits<'_>, symbol: u16) -> Result<usize, LoadError> {
    if !(257..=285).contains(&symbol) {
        return Err(LoadError::Unrecognized);
    }
    let index = (symbol - 257) as usize;
    let extra = bits.bits(u32::from(LEN_EXTRA[index]))? as usize;
    Ok(usize::from(LEN_BASE[index]) + extra)
}

fn distance_from(bits: &mut Bits<'_>, symbol: u16) -> Result<usize, LoadError> {
    let index = symbol as usize;
    if index >= DIST_BASE.len() {
        return Err(LoadError::Unrecognized);
    }
    let extra = bits.bits(u32::from(DIST_EXTRA[index]))? as usize;
    Ok(usize::from(DIST_BASE[index]) + extra)
}

fn inflate_block(
    bits: &mut Bits<'_>,
    out: &mut Vec<u8>,
    limit: usize,
    lit: &Huffman,
    dist: &Huffman,
) -> Result<(), LoadError> {
    loop {
        let symbol = decode(bits, lit)?;
        match symbol {
            0..=255 => push(out, limit, symbol as u8)?,
            256 => return Ok(()),
            257..=285 => {
                let len = length_from(bits, symbol)?;
                let distance_symbol = decode(bits, dist)?;
                let distance = distance_from(bits, distance_symbol)?;
                copy_match(out, limit, distance, len)?;
            }
            _ => return Err(LoadError::Unrecognized),
        }
    }
}

fn stored_block(bits: &mut Bits<'_>, out: &mut Vec<u8>, limit: usize) -> Result<(), LoadError> {
    let len_bytes = bits.read(4)?;
    let len = u16::from_le_bytes([len_bytes[0], len_bytes[1]]);
    let nlen = u16::from_le_bytes([len_bytes[2], len_bytes[3]]);
    if nlen != !len {
        return Err(LoadError::Unrecognized);
    }
    let copy = bits.read(usize::from(len))?;
    for &byte in copy {
        push(out, limit, byte)?;
    }
    Ok(())
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Inflate a zlib stream. `limit` is the maximum uncompressed size.
pub(crate) fn zlib_decompress(data: &[u8], limit: usize) -> Result<Vec<u8>, LoadError> {
    if data.len() < 2 {
        return Err(LoadError::Truncated);
    }
    let cmf = u32::from(data[0]);
    let flg = u32::from(data[1]);
    if (cmf * 256 + flg) % 31 != 0 || cmf & 0x0f != 8 || cmf >> 4 > 7 || flg & 0x20 != 0 {
        return Err(LoadError::Unrecognized);
    }
    let mut bits = Bits::new(&data[2..]);
    let mut out = Vec::new();
    loop {
        let finished = bits.bits(1)?;
        let kind = bits.bits(2)?;
        match kind {
            0 => stored_block(&mut bits, &mut out, limit)?,
            1 => {
                let (lit, dist) = fixed_pair()?;
                inflate_block(&mut bits, &mut out, limit, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = dynamic_pair(&mut bits)?;
                inflate_block(&mut bits, &mut out, limit, &lit, &dist)?;
            }
            _ => return Err(LoadError::Unrecognized),
        }
        if finished == 1 {
            break;
        }
    }
    let checksum = bits.read(4)?;
    let stored = u32::from_be_bytes([checksum[0], checksum[1], checksum[2], checksum[3]]);
    if stored != adler32(&out) {
        return Err(LoadError::Unrecognized);
    }
    if !bits.finished() {
        return Err(LoadError::Unrecognized);
    }
    Ok(out)
}
