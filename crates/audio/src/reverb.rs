//! One Schroeder tail for the listener's reverb zone.
//!
//! Four combs in parallel, then two allpasses. Delays are samples at 48 kHz.
//! Gains are constants. Low, and a listener outside every zone, leave the dry
//! buffer alone and do not step the delay lines.

use genos_math::Vec3;

const POLE: f32 = 0.08;
const COMB_A: usize = 1_429;
const COMB_B: usize = 1_553;
const COMB_C: usize = 1_699;
const COMB_D: usize = 1_861;
const ALLPASS_A: usize = 241;
const ALLPASS_B: usize = 383;

/// Preset. Low forces the send to 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Low,
    Medium,
    High,
    Ultra,
}

/// An axis-aligned room. `send` is in `[0, 1]`. `tail` selects a fixed gain set.
#[derive(Clone, Copy, Debug)]
pub struct Zone {
    pub min: Vec3,
    pub max: Vec3,
    pub send: f32,
    pub tail: u8,
}

/// Delay lines for one tail. Unallocated on Low: pass `None` to [`apply_reverb`].
pub struct Reverb {
    comb_a: Comb<COMB_A>,
    comb_b: Comb<COMB_B>,
    comb_c: Comb<COMB_C>,
    comb_d: Comb<COMB_D>,
    allpass_a: Allpass<ALLPASS_A>,
    allpass_b: Allpass<ALLPASS_B>,
    send: f32,
}

impl Reverb {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            comb_a: Comb::new(),
            comb_b: Comb::new(),
            comb_c: Comb::new(),
            comb_d: Comb::new(),
            allpass_a: Allpass::new(),
            allpass_b: Allpass::new(),
            send: 0.0,
        }
    }
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

/// The zone that contains `point`, if any.
///
/// The smallest box wins. Equal volume uses the lower tail index, then the
/// lower send, then the earlier zone.
#[must_use]
pub fn zone_index(zones: &[Zone], point: Vec3) -> Option<usize> {
    let mut best: Option<(usize, f32, u8, u32)> = None;
    for (index, zone) in zones.iter().enumerate() {
        if !contains(zone, point) {
            continue;
        }
        let volume = box_volume(zone);
        let send_bits = zone.send.clamp(0.0, 1.0).to_bits();
        let replace = match best {
            None => true,
            Some((_, best_volume, best_tail, best_send)) => {
                volume < best_volume
                    || (volume.to_bits() == best_volume.to_bits()
                        && (zone.tail < best_tail
                            || (zone.tail == best_tail && send_bits < best_send)))
            }
        };
        if replace {
            best = Some((index, volume, zone.tail, send_bits));
        }
    }
    best.map(|(index, _, _, _)| index)
}

/// Add the tail into `dry` (interleaved stereo).
///
/// Low returns without reading `state`. A zero send does not step the lines.
pub fn apply_reverb(
    dry: &mut [f32],
    listener: Vec3,
    zones: &[Zone],
    quality: Quality,
    state: Option<&mut Reverb>,
) {
    if quality == Quality::Low {
        return;
    }
    let Some(state) = state else {
        return;
    };
    let (target, tail) = match zone_index(zones, listener) {
        Some(index) => {
            let zone = zones.get(index).copied().unwrap_or(Zone {
                min: Vec3::ZERO,
                max: Vec3::ZERO,
                send: 0.0,
                tail: 0,
            });
            (zone.send.clamp(0.0, 1.0), zone.tail)
        }
        None => (0.0, 0),
    };
    if target.to_bits() == 0 && state.send.to_bits() == 0 {
        return;
    }
    let comb = comb_gain(tail);
    let allpass = allpass_gain(tail);
    let frames = dry.len() / 2;
    for frame in 0..frames {
        state.send += (target - state.send) * POLE;
        let send = state.send.clamp(0.0, 1.0);
        let left = dry.get(frame * 2).copied().unwrap_or(0.0);
        let right = dry.get(frame * 2 + 1).copied().unwrap_or(0.0);
        let wet = state.step(f32::midpoint(left, right) * send, comb, allpass);
        if let Some(slot) = dry.get_mut(frame * 2) {
            *slot = left * (1.0 - send) + wet;
        }
        if let Some(slot) = dry.get_mut(frame * 2 + 1) {
            *slot = right * (1.0 - send) + wet;
        }
    }
}

impl Reverb {
    fn step(&mut self, input: f32, comb: f32, allpass: f32) -> f32 {
        let mixed = 0.25
            * (self.comb_a.step(input, comb)
                + self.comb_b.step(input, comb)
                + self.comb_c.step(input, comb)
                + self.comb_d.step(input, comb));
        self.allpass_b
            .step(self.allpass_a.step(mixed, allpass), allpass)
    }
}

fn comb_gain(tail: u8) -> f32 {
    match tail {
        0 => 0.84,
        1 => 0.74,
        2 => 0.64,
        _ => 0.54,
    }
}

fn allpass_gain(tail: u8) -> f32 {
    match tail {
        0 | 1 => 0.5,
        2 => 0.4,
        _ => 0.3,
    }
}

fn contains(zone: &Zone, point: Vec3) -> bool {
    point.x >= zone.min.x
        && point.x <= zone.max.x
        && point.y >= zone.min.y
        && point.y <= zone.max.y
        && point.z >= zone.min.z
        && point.z <= zone.max.z
}

fn box_volume(zone: &Zone) -> f32 {
    (zone.max.x - zone.min.x).abs()
        * (zone.max.y - zone.min.y).abs()
        * (zone.max.z - zone.min.z).abs()
}

struct Comb<const N: usize> {
    buf: [f32; N],
    index: usize,
}

struct Allpass<const N: usize> {
    buf: [f32; N],
    index: usize,
}

impl<const N: usize> Comb<N> {
    const fn new() -> Self {
        Self {
            buf: [0.0; N],
            index: 0,
        }
    }

    fn step(&mut self, input: f32, gain: f32) -> f32 {
        let delayed = self.buf.get(self.index).copied().unwrap_or(0.0);
        if let Some(slot) = self.buf.get_mut(self.index) {
            *slot = input + delayed * gain;
        }
        self.advance();
        delayed
    }

    fn advance(&mut self) {
        self.index = self.index.saturating_add(1);
        if self.index >= N {
            self.index = 0;
        }
    }
}

impl<const N: usize> Allpass<N> {
    const fn new() -> Self {
        Self {
            buf: [0.0; N],
            index: 0,
        }
    }

    fn step(&mut self, input: f32, gain: f32) -> f32 {
        let delayed = self.buf.get(self.index).copied().unwrap_or(0.0);
        let stored = input + delayed * gain;
        if let Some(slot) = self.buf.get_mut(self.index) {
            *slot = stored;
        }
        self.index = self.index.saturating_add(1);
        if self.index >= N {
            self.index = 0;
        }
        delayed - gain * stored
    }
}
