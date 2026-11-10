//! IMA ADPCM, 4-bit, with the IMA reference step table.
//!
//! The cooked record is version byte `1`, codec id `1`, then little-endian
//! rate (`u32`), channels (`u16`), frame count (`u32`), loop start (`u32`),
//! loop end (`u32`), and the nibble payload. Predictor and step index start
//! at 0. A nibble's low half is the earlier sample.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::bin::{u16_le, u32_le};
use crate::error::LoadError;
use crate::wav::{decode_wav, Pcm};

const VERSION: u8 = 1;
const CODEC_IMA: u8 = 1;
const HEADER: usize = 20;
const STEP: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX_DELTA: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

static COOK_FALLBACK: AtomicU64 = AtomicU64::new(0);

/// How many times a shipping load fell back from a missing cook to WAV.
#[must_use]
pub fn audio_cook_fallbacks() -> u64 {
    COOK_FALLBACK.load(Ordering::Relaxed)
}

/// True when the decoded clip is at most 2 seconds or at most 256 KiB.
#[must_use]
pub fn resident_clip(rate: u32, channels: u16, frames: u32) -> bool {
    let under_two = frames <= rate.saturating_mul(2);
    let bytes = u64::from(frames)
        .saturating_mul(u64::from(channels))
        .saturating_mul(2);
    under_two || bytes <= 256 * 1024
}

/// A cooked clip. The payload stays packed until a reader asks for samples.
#[derive(Clone, Debug)]
pub struct CookedAudio {
    rate: u32,
    channels: u16,
    frames: u32,
    loop_start: u32,
    loop_end: u32,
    payload: Vec<u8>,
}

/// Decoder state. It starts at predictor 0 and step index 0.
#[derive(Clone, Copy, Debug)]
pub struct ImaCursor {
    predictor: [i32; 2],
    index: [i32; 2],
    frame: u32,
}

impl ImaCursor {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            predictor: [0, 0],
            index: [0, 0],
            frame: 0,
        }
    }
}

impl Default for ImaCursor {
    fn default() -> Self {
        Self::new()
    }
}

impl CookedAudio {
    #[must_use]
    pub fn rate(&self) -> u32 {
        self.rate
    }

    #[must_use]
    pub fn channels(&self) -> u16 {
        self.channels
    }

    #[must_use]
    pub fn frames(&self) -> u32 {
        self.frames
    }

    #[must_use]
    pub fn loop_start(&self) -> u32 {
        self.loop_start
    }

    #[must_use]
    pub fn loop_end(&self) -> u32 {
        self.loop_end
    }

    /// Decode the whole clip. A disk-fed voice uses [`Self::decode_frames`] instead.
    #[must_use]
    pub fn decode_all(&self) -> Pcm {
        let channels = usize::from(self.channels);
        let frames = usize::try_from(self.frames).unwrap_or(0);
        let mut samples = vec![0; frames.saturating_mul(channels)];
        let mut cursor = ImaCursor::new();
        let _ = self.decode_frames(&mut cursor, &mut samples);
        Pcm {
            sample_rate: self.rate,
            channels: self.channels,
            samples,
        }
    }

    /// Decode the next frames into `out` (interleaved). Returns samples written.
    pub fn decode_frames(&self, cursor: &mut ImaCursor, out: &mut [i16]) -> usize {
        let channels = usize::from(self.channels);
        if channels == 0 || out.len() < channels {
            return 0;
        }
        let mut written = 0usize;
        while written + channels <= out.len() && cursor.frame < self.frames {
            for channel in 0..channels {
                let nibble = self.nibble_at(cursor.frame, channel);
                out[written + channel] = decode_nibble(
                    &mut cursor.predictor[channel],
                    &mut cursor.index[channel],
                    nibble,
                );
            }
            written += channels;
            cursor.frame = cursor.frame.saturating_add(1);
        }
        written
    }

    fn nibble_at(&self, frame: u32, channel: usize) -> u8 {
        let channels = usize::from(self.channels);
        let ordinal = usize::try_from(frame).unwrap_or(0).saturating_mul(channels) + channel;
        let byte = self.payload.get(ordinal / 2).copied().unwrap_or(0);
        if ordinal.is_multiple_of(2) {
            byte & 0x0f
        } else {
            byte >> 4
        }
    }
}

/// Cook `pcm` into the canonical record.
///
/// # Errors
///
/// Returns [`LoadError::Unrecognized`] when the channel count is not 1 or 2,
/// or the sample count is not a whole number of frames.
pub fn cook_ima(pcm: &Pcm, loop_start: u32, loop_end: u32) -> Result<Vec<u8>, LoadError> {
    if pcm.channels != 1 && pcm.channels != 2 {
        return Err(LoadError::Unrecognized);
    }
    let channels = usize::from(pcm.channels);
    if channels == 0 || !pcm.samples.len().is_multiple_of(channels) {
        return Err(LoadError::Unrecognized);
    }
    let frames = u32::try_from(pcm.samples.len() / channels).unwrap_or(u32::MAX);
    let mut payload = Vec::new();
    let mut predictor = [0i32; 2];
    let mut index = [0i32; 2];
    let mut pending: Option<u8> = None;
    for frame in 0..usize::try_from(frames).unwrap_or(0) {
        for channel in 0..channels {
            let sample = i32::from(pcm.samples[frame * channels + channel]);
            let nibble = encode_nibble(&mut predictor[channel], &mut index[channel], sample);
            match pending {
                None => pending = Some(nibble),
                Some(low) => {
                    payload.push(low | (nibble << 4));
                    pending = None;
                }
            }
        }
    }
    if let Some(low) = pending {
        payload.push(low);
    }
    let mut record = Vec::with_capacity(HEADER + payload.len());
    record.push(VERSION);
    record.push(CODEC_IMA);
    record.extend(pcm.sample_rate.to_le_bytes());
    record.extend(pcm.channels.to_le_bytes());
    record.extend(frames.to_le_bytes());
    record.extend(loop_start.to_le_bytes());
    record.extend(loop_end.to_le_bytes());
    record.extend(payload);
    Ok(record)
}

/// Read a cooked record.
///
/// # Errors
///
/// Returns [`LoadError::Unrecognized`] when the version or codec is not the
/// IMA record, and [`LoadError::Truncated`] when the payload is short.
pub fn parse_cooked(bytes: &[u8]) -> Result<CookedAudio, LoadError> {
    if bytes.first().copied() != Some(VERSION) || bytes.get(1).copied() != Some(CODEC_IMA) {
        return Err(LoadError::Unrecognized);
    }
    if bytes.len() < HEADER {
        return Err(LoadError::Truncated);
    }
    let rate = u32_le(bytes, 2)?;
    let channels = u16_le(bytes, 6)?;
    let frames = u32_le(bytes, 8)?;
    let loop_start = u32_le(bytes, 12)?;
    let loop_end = u32_le(bytes, 16)?;
    if channels != 1 && channels != 2 {
        return Err(LoadError::Unrecognized);
    }
    let nibbles = u64::from(frames).saturating_mul(u64::from(channels));
    let payload_len = usize::try_from(nibbles.div_ceil(2)).unwrap_or(usize::MAX);
    let end = HEADER
        .checked_add(payload_len)
        .ok_or(LoadError::Truncated)?;
    let payload = bytes.get(HEADER..end).ok_or(LoadError::Truncated)?;
    Ok(CookedAudio {
        rate,
        channels,
        frames,
        loop_start,
        loop_end,
        payload: payload.to_vec(),
    })
}

/// Load a cooked clip, or a WAV when the cook is missing or mismatched.
///
/// A cooked record does not parse RIFF. The fallback logs one line.
///
/// # Errors
///
/// Returns the WAV error when the fallback cannot decode, or
/// [`LoadError::Truncated`] when a cook header is cut short.
pub fn load_audio(bytes: &[u8]) -> Result<Pcm, LoadError> {
    match parse_cooked(bytes) {
        Ok(cooked) => Ok(cooked.decode_all()),
        Err(LoadError::Unrecognized) => {
            let pcm = decode_wav(bytes)?;
            COOK_FALLBACK.fetch_add(1, Ordering::Relaxed);
            eprintln!("genos-load: audio cook missing or mismatched, decoded WAV");
            Ok(pcm)
        }
        Err(err) => Err(err),
    }
}

fn step_at(index: i32) -> i32 {
    let at = usize::try_from(index).unwrap_or(0);
    STEP.get(at).copied().unwrap_or(32_767)
}

fn decode_nibble(predictor: &mut i32, index: &mut i32, nibble: u8) -> i16 {
    let step = step_at(*index);
    let code = i32::from(nibble & 0x0f);
    let mut diff = step >> 3;
    if code & 4 != 0 {
        diff += step;
    }
    if code & 2 != 0 {
        diff += step >> 1;
    }
    if code & 1 != 0 {
        diff += step >> 2;
    }
    if code & 8 != 0 {
        *predictor -= diff;
    } else {
        *predictor += diff;
    }
    *predictor = (*predictor).clamp(i32::from(i16::MIN), i32::from(i16::MAX));
    let delta = INDEX_DELTA
        .get(usize::try_from(code & 7).unwrap_or(0))
        .copied()
        .unwrap_or(0);
    *index = (*index + delta).clamp(0, 88);
    i16::try_from(*predictor).unwrap_or(0)
}

fn encode_nibble(predictor: &mut i32, index: &mut i32, sample: i32) -> u8 {
    let step = step_at(*index);
    let mut diff = sample - *predictor;
    let mut nibble = 0i32;
    if diff < 0 {
        nibble = 8;
        diff = -diff;
    }
    if diff >= step {
        nibble += 4;
        diff -= step;
    }
    if diff >= (step >> 1) {
        nibble += 2;
        diff -= step >> 1;
    }
    if diff >= (step >> 2) {
        nibble += 1;
    }
    let nibble = u8::try_from(nibble).unwrap_or(0);
    let _ = decode_nibble(predictor, index, nibble);
    nibble
}
