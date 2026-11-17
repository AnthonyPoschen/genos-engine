//! Disk-fed clips. `genos-audio-io` decodes IMA ADPCM into a fixed ring.
//!
//! The ring is 200 ms of stereo-or-mono `f32`, allocated before the thread
//! starts. The audio thread only pops. An empty pop is an underrun.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{Builder, JoinHandle};

use genos_load::{CookedAudio, ImaCursor};

/// Samples the IO thread has published and the audio thread has not consumed.
pub struct SampleRing {
    slots: Box<[AtomicU32]>,
    write: AtomicUsize,
    read: AtomicUsize,
}

impl SampleRing {
    /// `rate` and `channels` size a 200 ms ring.
    #[must_use]
    pub fn new(rate: u32, channels: u16) -> Self {
        let frames = u64::from(rate) / 5;
        let count = frames.saturating_mul(u64::from(channels)).max(1);
        let count = usize::try_from(count).unwrap_or(1);
        let slots = (0..count).map(|_| AtomicU32::new(0)).collect();
        Self {
            slots,
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
        }
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn filled(&self) -> usize {
        self.write
            .load(Ordering::Acquire)
            .wrapping_sub(self.read.load(Ordering::Acquire))
    }

    pub fn push(&self, sample: f32) -> bool {
        let write = self.write.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        let cap = self.slots.len();
        if cap == 0 || write.wrapping_sub(read) >= cap {
            return false;
        }
        let Some(slot) = self.slots.get(write % cap) else {
            return false;
        };
        slot.store(sample.to_bits(), Ordering::Relaxed);
        self.write.store(write.wrapping_add(1), Ordering::Release);
        true
    }

    pub fn pop(&self) -> Option<f32> {
        let read = self.read.load(Ordering::Relaxed);
        let write = self.write.load(Ordering::Acquire);
        if read == write {
            return None;
        }
        let cap = self.slots.len();
        let bits = self.slots.get(read % cap)?.load(Ordering::Relaxed);
        self.read.store(read.wrapping_add(1), Ordering::Release);
        Some(f32::from_bits(bits))
    }
}

/// Decode `cooked` into `ring` until `stop`. `fill` false leaves the ring empty.
///
/// # Errors
///
/// Returns the operating-system error when the thread cannot start.
pub fn spawn_reader(
    cooked: CookedAudio,
    ring: Arc<SampleRing>,
    stop: Arc<AtomicBool>,
    fill: bool,
) -> Result<JoinHandle<()>, String> {
    Builder::new()
        .name(String::from("genos-audio-io"))
        .spawn(move || run_reader(&cooked, &ring, &stop, fill))
        .map_err(|err| err.to_string())
}

fn run_reader(cooked: &CookedAudio, ring: &SampleRing, stop: &AtomicBool, fill: bool) {
    if !fill {
        while !stop.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
        return;
    }
    let channels = usize::from(cooked.channels()).max(1);
    let mut cursor = ImaCursor::new();
    let mut block = vec![0i16; 256usize.saturating_mul(channels)];
    while !stop.load(Ordering::Acquire) {
        let free = ring.capacity().saturating_sub(ring.filled());
        let room = free.min(block.len());
        let room = room - room % channels;
        if room < channels {
            std::hint::spin_loop();
            continue;
        }
        let wrote = cooked.decode_frames(&mut cursor, &mut block[..room]);
        if wrote == 0 {
            if cooked.loop_end() > cooked.loop_start() {
                cursor = ImaCursor::new();
                continue;
            }
            while !stop.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
            return;
        }
        for sample in &block[..wrote] {
            let _ = ring.push(f32::from(*sample));
        }
    }
}
