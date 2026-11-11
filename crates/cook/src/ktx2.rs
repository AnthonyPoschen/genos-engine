//! KTX 2.0 container for block-compressed mips (plan step 11).
//!
//! <https://registry.khronos.org/KTX/specs/2.0/ktxspec.v2.html>. One layer, one
//! face, no supercompression, no key/value data. The data format descriptor is
//! the basic block for the format. Level 0 is the finest; in the file the
//! smallest level comes first and every level starts on a 16-byte boundary.

use crate::CookError;

const IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
const HEADER: usize = 12 + 9 * 4 + 4 * 4 + 2 * 8;
const LEVEL_ENTRY: usize = 3 * 8;
const ALIGN: usize = 16;

/// The block formats the cook writes. Values are `VkFormat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockFormat {
    Bc5Unorm,
    Bc7Unorm,
    Bc7Srgb,
    Astc4x4Unorm,
    Astc4x4Srgb,
}

impl BlockFormat {
    #[must_use]
    pub fn vk_format(self) -> u32 {
        match self {
            Self::Bc5Unorm => 141,
            Self::Bc7Unorm => 145,
            Self::Bc7Srgb => 146,
            Self::Astc4x4Unorm => 157,
            Self::Astc4x4Srgb => 158,
        }
    }

    fn from_vk(v: u32) -> Option<Self> {
        [
            Self::Bc5Unorm,
            Self::Bc7Unorm,
            Self::Bc7Srgb,
            Self::Astc4x4Unorm,
            Self::Astc4x4Srgb,
        ]
        .into_iter()
        .find(|f| f.vk_format() == v)
    }

    fn srgb(self) -> bool {
        matches!(self, Self::Bc7Srgb | Self::Astc4x4Srgb)
    }

    /// `KHR_DF_MODEL_*`.
    fn colour_model(self) -> u32 {
        match self {
            Self::Bc5Unorm => 132,
            Self::Bc7Unorm | Self::Bc7Srgb => 134,
            Self::Astc4x4Unorm | Self::Astc4x4Srgb => 162,
        }
    }
}

/// A decoded KTX2: the format, the base size and each level's blocks, finest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ktx2 {
    pub format: BlockFormat,
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
}

fn dfd(format: BlockFormat) -> Vec<u8> {
    // (bit offset, bit length - 1, channel id) per sample.
    let samples: &[(u32, u32, u32)] = if format == BlockFormat::Bc5Unorm {
        &[(0, 63, 0), (64, 63, 1)]
    } else {
        &[(0, 127, 0)]
    };
    let block = 24 + 16 * samples.len();
    let transfer = if format.srgb() { 2 } else { 1 };
    let mut words = vec![
        0,
        2 | (u32::try_from(block).unwrap_or(0) << 16),
        format.colour_model() | (1 << 8) | (transfer << 16),
        3 | (3 << 8),
        16,
        0,
    ];
    for &(offset, length, channel) in samples {
        words.extend([offset | (length << 16) | (channel << 24), 0, 0, u32::MAX]);
    }
    let total = u32::try_from(4 + block).unwrap_or(0);
    std::iter::once(total)
        .chain(words)
        .flat_map(u32::to_le_bytes)
        .collect()
}

fn pad(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(ALIGN) {
        out.push(0);
    }
}

fn u32_of(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn u64_of(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Write a KTX2 file.
#[must_use]
pub fn write(k: &Ktx2) -> Vec<u8> {
    let dfd = dfd(k.format);
    let dfd_offset = HEADER + LEVEL_ENTRY * k.levels.len();
    let mut out = Vec::new();
    out.extend_from_slice(&IDENTIFIER);
    for v in [
        k.format.vk_format(),
        1,
        k.width,
        k.height,
        0,
        0,
        1,
        u32_of(k.levels.len()),
        0,
        u32_of(dfd_offset),
        u32_of(dfd.len()),
        0,
        0,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[0; 16]);
    out.resize(dfd_offset, 0);
    out.extend_from_slice(&dfd);
    let mut offsets = vec![0; k.levels.len()];
    for (i, level) in k.levels.iter().enumerate().rev() {
        pad(&mut out);
        offsets[i] = out.len();
        out.extend_from_slice(level);
    }
    for (i, level) in k.levels.iter().enumerate() {
        let at = HEADER + LEVEL_ENTRY * i;
        let len = u64_of(level.len()).to_le_bytes();
        out[at..at + 8].copy_from_slice(&u64_of(offsets[i]).to_le_bytes());
        out[at + 8..at + 16].copy_from_slice(&len);
        out[at + 16..at + 24].copy_from_slice(&len);
    }
    out
}

fn word(b: &[u8], at: usize) -> Result<u32, CookError> {
    b.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(CookError::Corrupt("ktx2 truncated"))
}

fn dword(b: &[u8], at: usize) -> Result<usize, CookError> {
    b.get(at..at + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .and_then(|v| usize::try_from(v).ok())
        .ok_or(CookError::Corrupt("ktx2 truncated"))
}

/// Read a KTX2 file this module wrote.
///
/// # Errors
///
/// A bad identifier, an unknown format, supercompression, or a level outside the file.
pub fn read(b: &[u8]) -> Result<Ktx2, CookError> {
    if b.get(..12) != Some(&IDENTIFIER[..]) {
        return Err(CookError::Corrupt("ktx2 identifier"));
    }
    let format = BlockFormat::from_vk(word(b, 12)?).ok_or(CookError::Corrupt("ktx2 format"))?;
    if word(b, 44)? != 0 {
        return Err(CookError::Corrupt("ktx2 supercompression"));
    }
    let count = usize::try_from(word(b, 40)?.max(1)).unwrap_or(0);
    let mut levels = Vec::with_capacity(count.min(32));
    for i in 0..count {
        let at = HEADER + LEVEL_ENTRY * i;
        let (offset, len) = (dword(b, at)?, dword(b, at + 8)?);
        let end = offset
            .checked_add(len)
            .ok_or(CookError::Corrupt("ktx2 level"))?;
        levels.push(
            b.get(offset..end)
                .ok_or(CookError::Corrupt("ktx2 level"))?
                .to_vec(),
        );
    }
    Ok(Ktx2 {
        format,
        width: word(b, 20)?,
        height: word(b, 24)?,
        levels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_block_is_one_16_byte_level() {
        let k = Ktx2 {
            format: BlockFormat::Bc7Srgb,
            width: 4,
            height: 4,
            levels: vec![vec![7; 16]],
        };
        let bytes = write(&k);
        assert_eq!(&bytes[..12], &IDENTIFIER);
        assert_eq!(word(&bytes, 12).ok(), Some(146));
        assert_eq!(dword(&bytes, HEADER + 8).ok(), Some(16));
        assert_eq!(dword(&bytes, HEADER).ok().map(|o| o % 16), Some(0));
        assert_eq!(read(&bytes).ok(), Some(k));
    }

    #[test]
    fn levels_are_stored_smallest_first_and_read_finest_first() {
        let k = Ktx2 {
            format: BlockFormat::Bc5Unorm,
            width: 8,
            height: 8,
            levels: vec![vec![1; 64], vec![2; 16]],
        };
        let bytes = write(&k);
        let (fine, coarse) = (dword(&bytes, HEADER), dword(&bytes, HEADER + 24));
        assert!(matches!((fine, coarse), (Ok(f), Ok(c)) if c < f));
        assert_eq!(read(&bytes).ok(), Some(k));
        let mut bad = bytes;
        bad[0] = 0;
        assert!(read(&bad).is_err());
    }
}
