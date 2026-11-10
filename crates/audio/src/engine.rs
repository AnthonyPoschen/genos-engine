//! The `genos-audio` thread. It mixes one quantum and writes the device.
//!
//! The callback does not allocate and does not lock. Commands arrive on
//! [`CommandRing`]. The listener is a latest-wins slot.

use std::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{Builder, JoinHandle};

use genos_load::Pcm;
use genos_math::Vec3;

use crate::mix::{quantize_into, render_source, Barrier, Listener, Source};
use crate::path::DevicePath;
use crate::ring::{Command, CommandRing};

const VOICES: usize = 32;

#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    clip: u16,
    positional: bool,
    looping: bool,
    gain: f32,
    position: Vec3,
    velocity: Vec3,
    transmission: Option<f32>,
    cursor: f64,
}

impl Voice {
    const fn silent() -> Self {
        Self {
            active: false,
            clip: 0,
            positional: false,
            looping: false,
            gain: 1.0,
            position: Vec3::new(0.0, 0.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, 0.0),
            transmission: None,
            cursor: 0.0,
        }
    }
}

struct ListenerSlot {
    seq: AtomicU64,
    bits: [AtomicU32; 9],
}

impl ListenerSlot {
    fn new() -> Self {
        let slot = Self {
            seq: AtomicU64::new(0),
            bits: std::array::from_fn(|_| AtomicU32::new(0)),
        };
        slot.store(&Listener {
            position: Vec3::ZERO,
            facing: Vec3::new(0.0, 0.0, -1.0),
            velocity: Vec3::ZERO,
        });
        slot
    }

    fn store(&self, listener: &Listener) {
        let seq = self.seq.load(Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        store_vec(&self.bits, 0, listener.position);
        store_vec(&self.bits, 3, listener.facing);
        store_vec(&self.bits, 6, listener.velocity);
        fence(Ordering::Release);
        self.seq.store(seq.wrapping_add(2), Ordering::Release);
    }

    fn load(&self) -> Listener {
        loop {
            let start = self.seq.load(Ordering::Acquire);
            if start & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            fence(Ordering::Acquire);
            let position = load_vec(&self.bits, 0);
            let facing = load_vec(&self.bits, 3);
            let velocity = load_vec(&self.bits, 6);
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == start {
                return Listener {
                    position,
                    facing,
                    velocity,
                };
            }
        }
    }
}

fn store_vec(bits: &[AtomicU32; 9], at: usize, value: Vec3) {
    bits[at].store(value.x.to_bits(), Ordering::Relaxed);
    bits[at + 1].store(value.y.to_bits(), Ordering::Relaxed);
    bits[at + 2].store(value.z.to_bits(), Ordering::Relaxed);
}

fn load_vec(bits: &[AtomicU32; 9], at: usize) -> Vec3 {
    Vec3::new(
        f32::from_bits(bits[at].load(Ordering::Relaxed)),
        f32::from_bits(bits[at + 1].load(Ordering::Relaxed)),
        f32::from_bits(bits[at + 2].load(Ordering::Relaxed)),
    )
}

struct Counters {
    period_frames: AtomicU32,
    xrun: AtomicU64,
    periods: AtomicU64,
    rate_rejected: AtomicU64,
    voice_drop: AtomicU64,
}

/// Probes the device thread publishes.
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioStats {
    pub device_period_frames: u32,
    pub device_xrun: u64,
    pub audio_command_drop: u64,
    pub periods_written: u64,
    pub device_rate_rejected: u64,
    pub audio_voice_drop: u64,
}

/// A running mix thread and its command ring.
pub struct Stream {
    commands: Arc<CommandRing>,
    listener: Arc<ListenerSlot>,
    counters: Arc<Counters>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Stream {
    /// Latest listener. This is not a ring entry.
    pub fn set_listener(&self, listener: Listener) {
        self.listener.store(&listener);
    }

    /// Enqueue one command. A full ring drops it and counts `audio_command_drop`.
    #[must_use]
    pub fn push(&self, command: Command) -> bool {
        self.commands.push(command)
    }

    #[must_use]
    pub fn stats(&self) -> AudioStats {
        AudioStats {
            device_period_frames: self.counters.period_frames.load(Ordering::Relaxed),
            device_xrun: self.counters.xrun.load(Ordering::Relaxed),
            audio_command_drop: self.commands.dropped(),
            periods_written: self.counters.periods.load(Ordering::Relaxed),
            device_rate_rejected: self.counters.rate_rejected.load(Ordering::Relaxed),
            audio_voice_drop: self.counters.voice_drop.load(Ordering::Relaxed),
        }
    }

    /// Ask the thread to finish the current quantum and join it.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Start `genos-audio` on `device`.
///
/// `clips` and `barriers` are fixed for the life of the stream. The thread
/// calls `device.open` once and keeps that device until [`Stream::stop`].
///
/// # Errors
///
/// Returns the device's open error, or a thread-start error when the
/// operating system refuses the thread.
pub fn start_with<D: DevicePath + 'static>(
    mut device: D,
    clips: Vec<Pcm>,
    barriers: &[Barrier],
) -> Result<Stream, String> {
    let commands = Arc::new(CommandRing::new());
    let listener = Arc::new(ListenerSlot::new());
    let counters = Arc::new(Counters {
        period_frames: AtomicU32::new(0),
        xrun: AtomicU64::new(0),
        periods: AtomicU64::new(0),
        rate_rejected: AtomicU64::new(0),
        voice_drop: AtomicU64::new(0),
    });
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::sync_channel(1);
    let commands_thread = Arc::clone(&commands);
    let listener_thread = Arc::clone(&listener);
    let counters_thread = Arc::clone(&counters);
    let stop_thread = Arc::clone(&stop);
    let barriers = barriers.to_vec();
    let spawned = Builder::new()
        .name(String::from("genos-audio"))
        .spawn(move || {
            run_device(
                &mut device,
                &clips,
                &barriers,
                &ThreadIn {
                    commands: &commands_thread,
                    listener: &listener_thread,
                    counters: &counters_thread,
                    stop: &stop_thread,
                    opened: &tx,
                },
            );
        });
    let handle = match spawned {
        Ok(handle) => handle,
        Err(err) => return Err(err.to_string()),
    };
    match rx.recv() {
        Ok(Ok(())) => Ok(Stream {
            commands,
            listener,
            counters,
            stop,
            thread: Some(handle),
        }),
        Ok(Err(err)) => {
            let _ = handle.join();
            Err(err)
        }
        Err(err) => {
            let _ = handle.join();
            Err(err.to_string())
        }
    }
}

#[derive(Clone, Copy)]
struct ThreadIn<'a> {
    commands: &'a CommandRing,
    listener: &'a ListenerSlot,
    counters: &'a Counters,
    stop: &'a AtomicBool,
    opened: &'a mpsc::SyncSender<Result<(), String>>,
}

fn run_device<D: DevicePath>(
    device: &mut D,
    clips: &[Pcm],
    barriers: &[Barrier],
    thread: &ThreadIn<'_>,
) {
    let ThreadIn {
        commands,
        listener,
        counters,
        stop,
        opened,
    } = thread;
    let format = match device.open() {
        Ok(format) => format,
        Err(err) => {
            let _ = opened.send(Err(err));
            return;
        }
    };
    counters
        .period_frames
        .store(format.period_frames, Ordering::Relaxed);
    if format.rate != 48_000 {
        counters.rate_rejected.store(1, Ordering::Relaxed);
    }
    if opened.send(Ok(())).is_err() {
        device.close();
        return;
    }
    let frames = usize::try_from(format.period_frames).unwrap_or(0);
    let mut acc = vec![0.0f32; frames.saturating_mul(2)];
    let mut pcm = vec![0i16; frames.saturating_mul(2)];
    let mut voices = [Voice::silent(); VOICES];
    while !stop.load(Ordering::Acquire) {
        if !device.ready() {
            std::hint::spin_loop();
            continue;
        }
        while let Some(command) = commands.pop() {
            apply(command, &mut voices, clips.len(), counters);
        }
        let ear = listener.load();
        acc.fill(0.0);
        for voice in &mut voices {
            render_voice(voice, clips, &ear, barriers, format.rate, &mut acc);
        }
        quantize_into(&acc, &mut pcm);
        if device.write(&pcm).is_err() {
            counters.xrun.fetch_add(1, Ordering::Relaxed);
        }
        counters.periods.fetch_add(1, Ordering::Relaxed);
    }
    device.close();
}

fn render_voice(
    voice: &mut Voice,
    clips: &[Pcm],
    listener: &Listener,
    barriers: &[Barrier],
    rate: u32,
    acc: &mut [f32],
) {
    if !voice.active {
        return;
    }
    let Some(clip) = clips.get(usize::from(voice.clip)) else {
        voice.active = false;
        return;
    };
    let source = Source {
        pcm: clip,
        position: voice.position,
        velocity: voice.velocity,
        looping: voice.looping,
        positional: voice.positional,
        transmission: voice.transmission,
    };
    voice.cursor = render_source(
        acc,
        &source,
        listener,
        barriers,
        rate,
        voice.gain,
        voice.cursor,
    );
}

fn apply(command: Command, voices: &mut [Voice; VOICES], clips: usize, counters: &Counters) {
    match command {
        Command::Play {
            voice,
            clip,
            positional,
            looping,
            gain,
            position,
            velocity,
        } => {
            let Some(slot) = voices.get_mut(usize::from(voice)) else {
                counters.voice_drop.fetch_add(1, Ordering::Relaxed);
                return;
            };
            if usize::from(clip) >= clips {
                counters.voice_drop.fetch_add(1, Ordering::Relaxed);
                return;
            }
            *slot = Voice {
                active: true,
                clip,
                positional,
                looping,
                gain,
                position,
                velocity,
                transmission: None,
                cursor: 0.0,
            };
        }
        Command::Stop { voice } => {
            if let Some(slot) = voices.get_mut(usize::from(voice)) {
                slot.active = false;
            }
        }
        Command::SetPose {
            voice,
            position,
            velocity,
        } => {
            if let Some(slot) = voices.get_mut(usize::from(voice)) {
                slot.position = position;
                slot.velocity = velocity;
            }
        }
        Command::SetTransmission { voice, gain } => {
            if let Some(slot) = voices.get_mut(usize::from(voice)) {
                slot.transmission = Some(gain.max(0.0));
            }
        }
    }
}
