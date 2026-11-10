//! `mix_into` does not allocate. The output buffer is the caller's.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, Ordering};

use genos_audio::{mix_into, Barrier, Listener, Source, Vec3};
use genos_load::Pcm;

static ARMED: AtomicBool = AtomicBool::new(false);
static PANICKED: AtomicBool = AtomicBool::new(false);

// SAFETY: methods forward to the system allocator and only record that a call happened.
unsafe impl GlobalAlloc for Guard {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            let first = !PANICKED.swap(true, Ordering::Relaxed);
            assert!(!first, "mix_into allocated");
        }
        // SAFETY: `layout` is the size and alignment the caller requested.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from this allocator and `layout` matches that allocation.
        unsafe { System.dealloc(ptr, layout) }
    }
}

struct Guard;

#[global_allocator]
static GUARD: Guard = Guard;

#[test]
fn mix_into_does_not_allocate() {
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..256)
            .map(|index| i16::try_from(index).unwrap_or(0))
            .collect(),
    };
    let source = Source {
        pcm: &pcm,
        position: Vec3::new(-2.0, 1.0, -3.0),
        velocity: Vec3::new(1.0, 0.0, 6.0),
        looping: true,
        positional: true,
        transmission: None,
    };
    let listener = Listener {
        position: Vec3::new(0.0, 1.7, 0.0),
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::ZERO,
    };
    let barrier = Barrier {
        min: Vec3::new(-0.5, 0.0, -1.5),
        max: Vec3::new(0.5, 3.0, -1.4),
        absorption: 2.0,
    };
    let mut out = vec![0.0f32; 480 * 2];
    PANICKED.store(false, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    mix_into(
        &mut out,
        std::slice::from_ref(&source),
        &listener,
        std::slice::from_ref(&barrier),
        48_000,
    );
    ARMED.store(false, Ordering::SeqCst);
    assert!(out.iter().any(|sample| *sample != 0.0));
}
