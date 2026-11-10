//! Device outputs. Each one writes an already mixed stereo buffer.
//!
//! A console crate implements [`DevicePath`] and passes it to [`crate::start_with`].
//! This repository does not name that crate.

use std::cell::UnsafeCell;
#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64")
)))]
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

/// Id of the calling thread. [`FakeCapture::worker`] stores this from the audio thread.
#[must_use]
pub fn calling_thread() -> u64 {
    #[cfg(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64")
    ))]
    {
        linux_tid()
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64")
    )))]
    {
        local_token()
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn linux_tid() -> u64 {
    let id: u64;
    unsafe {
        // SAFETY: syscall 186 is `gettid`. It reads this thread's id and writes nothing.
        std::arch::asm!(
            "syscall",
            in("rax") 186u64,
            lateout("rax") id,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    id
}

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
fn linux_tid() -> u64 {
    let id: u64;
    unsafe {
        // SAFETY: syscall 178 is `gettid`. It reads this thread's id and writes nothing.
        std::arch::asm!(
            "svc #0",
            in("x8") 178u64,
            lateout("x0") id,
            options(nostack),
        );
    }
    id
}

#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64")
)))]
fn local_token() -> u64 {
    thread_local! {
        static TOKEN: Cell<u64> = const { Cell::new(0) };
    }
    TOKEN.with(|slot| {
        if slot.get() == 0 {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            slot.set(NEXT.fetch_add(1, Ordering::Relaxed));
        }
        slot.get()
    })
}

/// Rate, period, and channel count the device accepted.
#[derive(Clone, Copy, Debug)]
pub struct OpenedDevice {
    pub rate: u32,
    pub period_frames: u32,
    pub channels: u16,
}

/// A held output. `open` runs once on the `genos-audio` thread. `write` is the callback.
pub trait DevicePath: Send {
    /// Open the device and keep it open.
    ///
    /// # Errors
    ///
    /// Returns the operating-system text when the device cannot be opened.
    fn open(&mut self) -> Result<OpenedDevice, String>;
    /// Write one interleaved stereo quantum. This must not allocate.
    ///
    /// # Errors
    ///
    /// Returns a static reason when the device did not accept the quantum.
    fn write(&mut self, interleaved: &[i16]) -> Result<(), &'static str>;
    /// Release the device.
    fn close(&mut self);

    /// False asks the thread to spin and check stop, without mixing.
    fn ready(&mut self) -> bool {
        true
    }
}

/// Headless output. `write` discards the buffer.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullDevice;

impl DevicePath for NullDevice {
    fn open(&mut self) -> Result<OpenedDevice, String> {
        Ok(OpenedDevice {
            rate: 48_000,
            period_frames: 480,
            channels: 2,
        })
    }

    fn write(&mut self, interleaved: &[i16]) -> Result<(), &'static str> {
        if !interleaved.len().is_multiple_of(2) {
            return Err("stereo");
        }
        Ok(())
    }

    fn close(&mut self) {}
}

struct Capture {
    samples: UnsafeCell<Box<[i16]>>,
    filled: AtomicUsize,
    /// `usize::MAX` runs freely. Zero parks the callback before the mix.
    permits: AtomicUsize,
    parked: AtomicUsize,
    worker: AtomicU64,
}

// SAFETY: the audio thread writes slots below `filled` and then releases `filled`.
// A reader acquires `filled` and only reads those slots, after the writer has stopped
// or after that release.
unsafe impl Send for Capture {}
// SAFETY: same sharing rule as `Send`. The atomics publish the written prefix.
unsafe impl Sync for Capture {}

/// Records quanta for tests. The capture buffer is allocated before `open`.
pub struct FakeDevice {
    rate: u32,
    period_frames: u32,
    fail_writes: u32,
    capture: Arc<Capture>,
}

impl FakeDevice {
    /// `period_frames` is the quantum. `capacity_frames` is how much `write` keeps.
    #[must_use]
    pub fn new(period_frames: u32, capacity_frames: u32) -> (Self, FakeCapture) {
        let count = usize::try_from(capacity_frames)
            .unwrap_or(0)
            .saturating_mul(2);
        let samples = vec![0i16; count].into_boxed_slice();
        let capture = Arc::new(Capture {
            samples: UnsafeCell::new(samples),
            filled: AtomicUsize::new(0),
            permits: AtomicUsize::new(usize::MAX),
            parked: AtomicUsize::new(0),
            worker: AtomicU64::new(0),
        });
        (
            Self {
                rate: 48_000,
                period_frames,
                fail_writes: 0,
                capture: Arc::clone(&capture),
            },
            FakeCapture { capture },
        )
    }

    /// The next `count` writes return an error and do not store samples.
    pub fn fail_next(&mut self, count: u32) {
        self.fail_writes = count;
    }
}

/// Samples [`FakeDevice`] stored. Read this after the stream stops, or after `filled` advances.
pub struct FakeCapture {
    capture: Arc<Capture>,
}

impl FakeCapture {
    #[must_use]
    pub fn filled(&self) -> usize {
        self.capture.filled.load(Ordering::Acquire)
    }

    /// Park the callback before the next mix.
    pub fn hold(&self) {
        self.capture.permits.store(0, Ordering::Release);
    }

    /// Allow `count` mixes. `usize::MAX` runs freely.
    pub fn release(&self, count: usize) {
        if count == usize::MAX {
            self.capture.permits.store(usize::MAX, Ordering::Release);
            return;
        }
        self.capture.permits.fetch_add(count, Ordering::Release);
    }

    #[must_use]
    pub fn parked(&self) -> usize {
        self.capture.parked.load(Ordering::Acquire)
    }

    /// [`calling_thread`] of the audio thread, once `ready` has run.
    #[must_use]
    pub fn worker(&self) -> u64 {
        self.capture.worker.load(Ordering::Acquire)
    }

    /// Samples stored so far.
    ///
    /// Call this after [`crate::Stream::stop`], when the audio thread has joined.
    #[must_use]
    pub fn samples(&self) -> &[i16] {
        let stored = self.capture.samples.get();
        // SAFETY: the audio thread has stopped, so nothing else writes `samples`.
        let samples = unsafe { &*stored };
        let count = self.filled().min(samples.len());
        &samples[..count]
    }
}

impl DevicePath for FakeDevice {
    fn open(&mut self) -> Result<OpenedDevice, String> {
        Ok(OpenedDevice {
            rate: self.rate,
            period_frames: self.period_frames,
            channels: 2,
        })
    }

    fn write(&mut self, interleaved: &[i16]) -> Result<(), &'static str> {
        if self.fail_writes > 0 {
            self.fail_writes -= 1;
            return Err("xrun");
        }
        let filled = self.capture.filled.load(Ordering::Relaxed);
        let stored = self.capture.samples.get();
        // SAFETY: this thread is the only writer. `filled` is the prefix already published.
        let samples = unsafe { &mut *stored };
        let room = samples.len().saturating_sub(filled);
        let count = room.min(interleaved.len());
        if count > 0 {
            samples[filled..filled + count].copy_from_slice(&interleaved[..count]);
            self.capture.filled.store(filled + count, Ordering::Release);
        }
        Ok(())
    }

    fn close(&mut self) {}

    fn ready(&mut self) -> bool {
        self.capture
            .worker
            .store(calling_thread(), Ordering::Release);
        let permits = self.capture.permits.load(Ordering::Acquire);
        if permits == 0 {
            self.capture.parked.fetch_add(1, Ordering::Release);
            return false;
        }
        if permits != usize::MAX {
            let next = permits.saturating_sub(1);
            if self
                .capture
                .permits
                .compare_exchange(permits, next, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return false;
            }
        }
        true
    }
}
