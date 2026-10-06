//! Big-endian and little-endian reads that fail when the slice ends.

use crate::error::LoadError;

pub(crate) fn u16_be(data: &[u8], at: usize) -> Result<u16, LoadError> {
    let bytes = data.get(at..at + 2).ok_or(LoadError::Truncated)?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

pub(crate) fn i16_be(data: &[u8], at: usize) -> Result<i16, LoadError> {
    Ok(u16_be(data, at)? as i16)
}

pub(crate) fn u32_be(data: &[u8], at: usize) -> Result<u32, LoadError> {
    let bytes = data.get(at..at + 4).ok_or(LoadError::Truncated)?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub(crate) fn u16_le(data: &[u8], at: usize) -> Result<u16, LoadError> {
    let bytes = data.get(at..at + 2).ok_or(LoadError::Truncated)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

pub(crate) fn u32_le(data: &[u8], at: usize) -> Result<u32, LoadError> {
    let bytes = data.get(at..at + 4).ok_or(LoadError::Truncated)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
