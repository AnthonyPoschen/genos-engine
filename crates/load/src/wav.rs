//! PCM WAV. 16-bit mono or stereo, little-endian samples.

use crate::bin::{u16_le, u32_le};
use crate::error::LoadError;

/// Interleaved 16-bit PCM samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    /// Samples per second.
    pub sample_rate: u32,
    /// 1 for mono, 2 for stereo.
    pub channels: u16,
    /// Interleaved samples. Stereo pairs are left then right.
    pub samples: Vec<i16>,
}

pub(crate) fn decode_wav(data: &[u8]) -> Result<Pcm, LoadError> {
    if data.len() < 12 {
        return Err(if data.starts_with(b"RIFF") {
            LoadError::Truncated
        } else {
            LoadError::Unrecognized
        });
    }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(LoadError::Unrecognized);
    }
    let mut pos = 12usize;
    let mut format = None;
    let mut samples = None;
    while pos < data.len() {
        if pos + 8 > data.len() {
            return Err(LoadError::Truncated);
        }
        let id = [data[pos], data[pos + 1], data[pos + 2], data[pos + 3]];
        let size = u32_le(data, pos + 4)? as usize;
        let body = pos + 8;
        let body_end = body.checked_add(size).ok_or(LoadError::Unrecognized)?;
        if body_end > data.len() {
            return Err(LoadError::Truncated);
        }
        let chunk = &data[body..body_end];
        pos = body_end;
        if size % 2 == 1 && pos < data.len() {
            pos += 1;
        }
        match &id {
            b"fmt " => format = Some(parse_fmt(chunk)?),
            b"data" => {
                if samples.is_some() {
                    return Err(LoadError::Unrecognized);
                }
                samples = Some(chunk.to_vec());
            }
            _ => {}
        }
    }
    let (rate, channels, align) = format.ok_or(LoadError::Unrecognized)?;
    let data_bytes = samples.ok_or(LoadError::Unrecognized)?;
    if data_bytes.len() % usize::from(align) != 0 {
        return Err(LoadError::Truncated);
    }
    let mut pcm = Vec::with_capacity(data_bytes.len() / 2);
    let mut at = 0usize;
    while at + 1 < data_bytes.len() {
        pcm.push(u16_le(&data_bytes, at)? as i16);
        at += 2;
    }
    Ok(Pcm {
        sample_rate: rate,
        channels,
        samples: pcm,
    })
}

fn parse_fmt(chunk: &[u8]) -> Result<(u32, u16, u16), LoadError> {
    if chunk.len() < 16 {
        return Err(LoadError::Unrecognized);
    }
    let audio_format = u16_le(chunk, 0)?;
    let channels = u16_le(chunk, 2)?;
    let rate = u32_le(chunk, 4)?;
    let byte_rate = u32_le(chunk, 8)?;
    let align = u16_le(chunk, 12)?;
    let bits = u16_le(chunk, 14)?;
    if audio_format != 1 || bits != 16 || (channels != 1 && channels != 2) || rate == 0 {
        return Err(LoadError::Unrecognized);
    }
    if align != channels * 2 || byte_rate != rate * u32::from(align) {
        return Err(LoadError::Unrecognized);
    }
    Ok((rate, channels, align))
}
