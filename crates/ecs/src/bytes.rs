//! Little-endian component bytes. A delta is one component, not the object.

use crate::DecodeError;

/// Append a `u32` in little-endian order.
pub fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Append an `f32` in little-endian order.
pub fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Append raw bytes, such as a 32-byte content hash.
pub fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(bytes);
}

/// Read a little-endian `u32` and advance `input`.
///
/// # Errors
///
/// [`DecodeError::Truncated`] when fewer than 4 bytes remain.
pub fn read_u32(input: &mut &[u8]) -> Result<u32, DecodeError> {
    let bytes = take(input, 4)?;
    let Some(array) = copy4(bytes) else {
        return Err(DecodeError::Truncated);
    };
    Ok(u32::from_le_bytes(array))
}

/// Read a little-endian `f32` and advance `input`.
///
/// # Errors
///
/// [`DecodeError::Truncated`] when fewer than 4 bytes remain.
pub fn read_f32(input: &mut &[u8]) -> Result<f32, DecodeError> {
    let bytes = take(input, 4)?;
    let Some(array) = copy4(bytes) else {
        return Err(DecodeError::Truncated);
    };
    Ok(f32::from_le_bytes(array))
}

/// Copy `n` bytes out of `input`.
///
/// # Errors
///
/// [`DecodeError::Truncated`] when `input` is shorter than `n`.
pub fn take<'a>(input: &mut &'a [u8], n: usize) -> Result<&'a [u8], DecodeError> {
    if input.len() < n {
        return Err(DecodeError::Truncated);
    }
    let (head, tail) = input.split_at(n);
    *input = tail;
    Ok(head)
}

fn copy4(bytes: &[u8]) -> Option<[u8; 4]> {
    let b0 = *bytes.first()?;
    let b1 = *bytes.get(1)?;
    let b2 = *bytes.get(2)?;
    let b3 = *bytes.get(3)?;
    Some([b0, b1, b2, b3])
}
