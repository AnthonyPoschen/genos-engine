//! One preallocated single-producer command ring.
//!
//! The frame thread pushes. The device thread pops. A full ring drops the newest
//! command. Head and tail are atomics. The quantum does not lock and does not allocate.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use genos_math::Vec3;

/// Commands the frame thread may enqueue. Every variant is `Copy`.
#[derive(Clone, Copy, Debug)]
pub enum Command {
    /// Start `voice` on `clip`. `voice` is an index into the 32-voice pool.
    Play {
        voice: u8,
        clip: u16,
        positional: bool,
        looping: bool,
        gain: f32,
        position: Vec3,
        velocity: Vec3,
    },
    Stop {
        voice: u8,
    },
    SetPose {
        voice: u8,
        position: Vec3,
        velocity: Vec3,
    },
    SetTransmission {
        voice: u8,
        gain: f32,
    },
}

const CAPACITY: usize = 256;

/// 256 slots, allocated once.
pub struct CommandRing {
    slots: Box<[UnsafeCell<Command>]>,
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped: AtomicU64,
}

// SAFETY: one producer writes a slot before releasing `tail`. One consumer
// reads that slot only after acquiring `tail`, and does not reuse it until
// `head` has moved past it.
unsafe impl Send for CommandRing {}
// SAFETY: same single-producer single-consumer rule as `Send`.
unsafe impl Sync for CommandRing {}

impl CommandRing {
    pub(crate) fn new() -> Self {
        let mut slots = Vec::with_capacity(CAPACITY);
        for _ in 0..CAPACITY {
            slots.push(UnsafeCell::new(Command::Stop { voice: 0 }));
        }
        Self {
            slots: slots.into_boxed_slice(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// Push `command`. Returns false when the ring is full; the command is dropped.
    pub fn push(&self, command: Command) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail.wrapping_sub(head) >= CAPACITY {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let index = tail % CAPACITY;
        // SAFETY: this slot is free. The producer is the only writer, and the
        // consumer does not read it until the `tail` store below.
        unsafe {
            *self.slots[index].get() = command;
        }
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        true
    }

    pub(crate) fn pop(&self) -> Option<Command> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head == tail {
            return None;
        }
        let index = head % CAPACITY;
        // SAFETY: the producer released `tail` after writing this slot, and
        // will not write it again until `head` moves past it.
        let command = unsafe { *self.slots[index].get() };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Some(command)
    }

    pub(crate) fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}
