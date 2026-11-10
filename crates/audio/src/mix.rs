#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_midpoint, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::map_unwrap_or, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]

//! Mix loaded PCM into one device-rate stereo buffer.

use genos_load::Pcm;
use genos_math::Vec3;

use crate::space::{distance_gain, doppler_ratio, pan_gains, transmission_gain};

/// Where the listener is, which way they face, and how they move.
#[derive(Clone, Copy, Debug)]
pub struct Listener {
    pub position: Vec3,
    pub facing: Vec3,
    pub velocity: Vec3,
}

/// One playing clip.
///
/// A positional clip takes Doppler, stereo balance, distance, and transmission.
/// A non-positional clip is music or UI: the mix copies it with no pitch shift
/// and no world loss. `transmission` replaces the barrier walk when the GPU
/// pass already computed that gain.
#[derive(Clone, Copy, Debug)]
pub struct Source<'a> {
    pub pcm: &'a Pcm,
    pub position: Vec3,
    pub velocity: Vec3,
    pub looping: bool,
    pub positional: bool,
    pub transmission: Option<f32>,
}

/// One solid on the straight path. Absorption is nepers per meter.
#[derive(Clone, Copy, Debug)]
pub struct Barrier {
    pub min: Vec3,
    pub max: Vec3,
    pub absorption: f32,
}

/// Mix `sources` into `frames` of interleaved stereo at `device_rate`.
///
/// The returned buffer is the only allocation. The sum uses a stack scratch
/// and [`mix_into`] does not allocate.
pub fn mix(
    sources: &[Source<'_>],
    listener: &Listener,
    barriers: &[Barrier],
    device_rate: u32,
    frames: usize,
) -> Vec<i16> {
    if device_rate == 0 || frames == 0 {
        return Vec::new();
    }
    let mut out = vec![0i16; frames.saturating_mul(2)];
    write_mix(&mut out, sources, listener, barriers, device_rate);
    out
}

const SCRATCH_FRAMES: usize = 128;

fn write_mix(
    out: &mut [i16],
    sources: &[Source<'_>],
    listener: &Listener,
    barriers: &[Barrier],
    device_rate: u32,
) {
    let mut scratch = [0.0f32; SCRATCH_FRAMES * 2];
    let total = out.len() / 2;
    let mut base = 0usize;
    while base < total {
        let count = (total - base).min(SCRATCH_FRAMES);
        let window = &mut scratch[..count.saturating_mul(2)];
        window.fill(0.0);
        if device_rate != 0 {
            for source in sources {
                render_source(window, source, listener, barriers, device_rate, base);
            }
        }
        let start = base.saturating_mul(2);
        quantize_into(window, &mut out[start..start + window.len()]);
        base += count;
    }
}

fn quantize_into(acc: &[f32], out: &mut [i16]) {
    let count = acc.len().min(out.len());
    for index in 0..count {
        out[index] = acc[index].round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    }
}

/// Add every voice into `out`.
///
/// `out` is interleaved stereo and is cleared first. The caller owns the
/// buffer. This function does not allocate.
pub fn mix_into(
    out: &mut [f32],
    sources: &[Source<'_>],
    listener: &Listener,
    barriers: &[Barrier],
    device_rate: u32,
) {
    out.fill(0.0);
    if device_rate == 0 || out.len() < 2 {
        return;
    }
    for source in sources {
        render_source(out, source, listener, barriers, device_rate, 0);
    }
}

fn render_source(
    acc: &mut [f32],
    source: &Source<'_>,
    listener: &Listener,
    barriers: &[Barrier],
    device_rate: u32,
    frame_base: usize,
) {
    let channels = source.pcm.channels as usize;
    if source.pcm.sample_rate == 0 || (channels != 1 && channels != 2) {
        return;
    }
    let source_frames = source.pcm.samples.len() / channels;
    if source_frames == 0 {
        return;
    }
    let frames = acc.len() / 2;
    let (level_l, level_r, step) = levels(source, listener, barriers, device_rate);
    for frame in 0..frames {
        let cursor = f64_from_usize(frame_base + frame) * step;
        if !source.looping && cursor >= source_frames as f64 {
            break;
        }
        let (left, right) = read_pair(source, cursor, source_frames);
        acc[frame * 2] += left * level_l;
        acc[frame * 2 + 1] += right * level_r;
    }
}

fn levels(
    source: &Source<'_>,
    listener: &Listener,
    barriers: &[Barrier],
    device_rate: u32,
) -> (f32, f32, f64) {
    if !source.positional {
        let step = source.pcm.sample_rate as f64 / device_rate as f64;
        return (1.0, 1.0, step);
    }
    let ratio = doppler_ratio(
        source.position,
        source.velocity,
        listener.position,
        listener.velocity,
    );
    let (pan_l, pan_r) = pan_gains(source.position, listener.position, listener.facing);
    let distance = distance_gain(source.position, listener.position);
    let transmit = source
        .transmission
        .map(|gain| gain.max(0.0))
        .unwrap_or_else(|| transmission_gain(source.position, listener.position, barriers));
    let step = source.pcm.sample_rate as f64 / device_rate as f64 * f64::from(ratio);
    (
        pan_l * distance * transmit,
        pan_r * distance * transmit,
        step,
    )
}

fn read_pair(source: &Source<'_>, cursor: f64, source_frames: usize) -> (f32, f32) {
    let channels = source.pcm.channels as usize;
    if source.positional {
        let mono = if channels == 1 {
            interp(source, cursor, 0, source_frames)
        } else {
            0.5 * (interp(source, cursor, 0, source_frames)
                + interp(source, cursor, 1, source_frames))
        };
        return (mono, mono);
    }
    if channels == 1 {
        let sample = interp(source, cursor, 0, source_frames);
        return (sample, sample);
    }
    (
        interp(source, cursor, 0, source_frames),
        interp(source, cursor, 1, source_frames),
    )
}

fn interp(source: &Source<'_>, cursor: f64, channel: usize, source_frames: usize) -> f32 {
    if source_frames == 0 {
        return 0.0;
    }
    let period = source_frames as f64;
    let position = if source.looping {
        wrap_period(cursor, period)
    } else if cursor < 0.0 || cursor >= period {
        return 0.0;
    } else {
        cursor
    };
    let whole = floor_f64(position);
    let index = usize_from_f64(whole);
    if index >= source_frames {
        return 0.0;
    }
    let frac = (position - whole) as f32;
    let next = if source.looping {
        (index + 1) % source_frames
    } else {
        index + 1
    };
    let first = pcm_sample(source, index, channel);
    let second = if next >= source_frames {
        0.0
    } else {
        pcm_sample(source, next, channel)
    };
    first + (second - first) * frac
}

fn f64_from_usize(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}

fn wrap_period(cursor: f64, period: f64) -> f64 {
    if period <= 0.0 {
        return 0.0;
    }
    let mut wrapped = cursor - floor_f64(cursor / period) * period;
    if wrapped < 0.0 {
        wrapped += period;
    } else if wrapped >= period {
        wrapped -= period;
    }
    wrapped
}

fn floor_f64(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let bits = value.to_bits();
    let negative = bits & 0x8000_0000_0000_0000 != 0;
    let biased = (bits >> 52) & 0x7ff;
    if biased >= 0x7ff {
        return value;
    }
    if biased < 1023 {
        if value == 0.0 {
            return value;
        }
        return if negative { -1.0 } else { 0.0 };
    }
    let exponent = biased - 1023;
    if exponent >= 52 {
        return value;
    }
    let frac_bits = u32::try_from(52 - exponent).unwrap_or(0);
    let mask = (1u64 << frac_bits) - 1;
    if bits & mask == 0 {
        return value;
    }
    let truncated = f64::from_bits(bits & !mask);
    if negative {
        truncated - 1.0
    } else {
        truncated
    }
}

fn usize_from_f64(value: f64) -> usize {
    if value <= 0.0 || !value.is_finite() {
        return 0;
    }
    let bits = value.to_bits();
    let biased = (bits >> 52) & 0x7ff;
    if biased < 1023 {
        return 0;
    }
    let exponent = biased - 1023;
    if exponent >= 63 {
        return usize::MAX;
    }
    let significand = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
    let shift = u32::try_from(exponent).unwrap_or(0);
    let magnitude = if shift >= 52 {
        significand << (shift - 52)
    } else {
        significand >> (52 - shift)
    };
    usize::try_from(magnitude).unwrap_or(usize::MAX)
}

fn pcm_sample(source: &Source<'_>, frame: usize, channel: usize) -> f32 {
    let channels = source.pcm.channels as usize;
    source.pcm.samples[frame * channels + channel] as f32
}
