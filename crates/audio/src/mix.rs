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
    let mut acc = vec![0.0f32; frames * 2];
    for source in sources {
        render_source(&mut acc, source, listener, barriers, device_rate);
    }
    acc.into_iter()
        .map(|sample| sample.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16)
        .collect()
}

fn render_source(
    acc: &mut [f32],
    source: &Source<'_>,
    listener: &Listener,
    barriers: &[Barrier],
    device_rate: u32,
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
        let cursor = frame as f64 * step;
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
    let position = if source.looping {
        cursor.rem_euclid(source_frames as f64)
    } else if cursor < 0.0 || cursor >= source_frames as f64 {
        return 0.0;
    } else {
        cursor
    };
    let index = position.floor() as usize;
    let frac = (position - index as f64) as f32;
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

fn pcm_sample(source: &Source<'_>, frame: usize, channel: usize) -> f32 {
    let channels = source.pcm.channels as usize;
    source.pcm.samples[frame * channels + channel] as f32
}
