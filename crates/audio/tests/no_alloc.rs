#![cfg_attr(
    test,
    expect(clippy::panic, reason = "a test aborts when the stream fails to start")
)]

//! `mix_into` does not allocate. The output buffer is the caller's.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, Ordering};

use genos_audio::{
    calling_thread, mix_into, start_with, Barrier, Command, FakeDevice, Listener, Source, Vec3,
};
use genos_load::Pcm;

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCATED: AtomicBool = AtomicBool::new(false);
static WATCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_test() -> std::sync::MutexGuard<'static, ()> {
    match TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// SAFETY: methods forward to the system allocator and only record that a call happened.
unsafe impl GlobalAlloc for Guard {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) && calling_thread() == WATCH.load(Ordering::Relaxed) {
            ALLOCATED.store(true, Ordering::Relaxed);
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
    let _guard = lock_test();
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
    ALLOCATED.store(false, Ordering::SeqCst);
    WATCH.store(calling_thread(), Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    mix_into(
        &mut out,
        std::slice::from_ref(&source),
        &listener,
        std::slice::from_ref(&barrier),
        48_000,
    );
    ARMED.store(false, Ordering::SeqCst);
    WATCH.store(0, Ordering::SeqCst);
    assert!(!ALLOCATED.load(Ordering::SeqCst), "mix_into allocated");
    assert!(out.iter().any(|sample| *sample != 0.0));
}

#[test]
fn the_quantum_callback_does_not_allocate() {
    let _guard = lock_test();
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..256)
            .map(|index| i16::try_from(index).unwrap_or(0))
            .collect(),
    };
    let (device, capture) = FakeDevice::new(64, 256);
    capture.hold();
    let stream = match start_with(device, vec![pcm], &[]) {
        Ok(stream) => stream,
        Err(err) => panic!("stream did not start: {err}"),
    };
    let began = std::time::Instant::now();
    while capture.parked() == 0 && began.elapsed() < std::time::Duration::from_secs(2) {
        std::thread::yield_now();
    }
    stream.set_listener(Listener {
        position: Vec3::ZERO,
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::ZERO,
    });
    assert!(stream.push(Command::Play {
        voice: 0,
        clip: 0,
        positional: true,
        looping: true,
        gain: 1.0,
        position: Vec3::new(-1.0, 0.0, -2.0),
        velocity: Vec3::ZERO,
    }));
    ALLOCATED.store(false, Ordering::SeqCst);
    assert!(capture.worker() != 0, "the audio thread has not reached ready");
    WATCH.store(capture.worker(), Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    capture.release(1);
    let began = std::time::Instant::now();
    while capture.filled() < 128 && began.elapsed() < std::time::Duration::from_secs(2) {
        std::thread::yield_now();
    }
    ARMED.store(false, Ordering::SeqCst);
    WATCH.store(0, Ordering::SeqCst);
    let wrote = capture.filled() >= 128;
    capture.release(usize::MAX);
    stream.stop();
    assert!(wrote, "the callback did not write a quantum");
    assert!(!ALLOCATED.load(Ordering::SeqCst), "the callback allocated");
}
