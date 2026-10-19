//! Gaps between seat dispatches.
//!
//! `xdg_wm_base.ping` is answered only while the frame thread dispatches the
//! Wayland socket. Omarchy shows "Application not responding" when a mapped
//! window misses that ping. [`PUMP_BUDGET`] is the longest stretch this
//! process allows between dispatches.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use std::os::raw::c_void;

/// Longest stretch the seat may go without a dispatch.
pub const PUMP_BUDGET: Duration = Duration::from_millis(100);

/// High-water mark of the gap between dispatches, shared with a seat thread.
pub struct StallMeter {
    origin: Instant,
    last_ns: AtomicU64,
    /// `f64::to_bits` of the longest gap in milliseconds.
    longest_bits: AtomicU64,
}

impl StallMeter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            origin: Instant::now(),
            last_ns: AtomicU64::new(0),
            longest_bits: AtomicU64::new(0.0_f64.to_bits()),
        })
    }

    /// Note that the seat was dispatched now. The first call starts the clock.
    pub fn mark(&self) {
        let now = nanos_u64(self.origin.elapsed());
        let prev = self.last_ns.swap(now, Ordering::Relaxed);
        if prev == 0 {
            return;
        }
        self.note_ms(ms_from_ns(now.saturating_sub(prev)));
    }

    pub fn longest_ms(&self) -> f64 {
        f64::from_bits(self.longest_bits.load(Ordering::Relaxed))
    }

    /// If `published_ms` is below the mark, the caller cleared it. Adopt that.
    pub fn pull_published(&self, published_ms: f64) {
        if !published_ms.is_finite() || published_ms < 0.0 {
            return;
        }
        let published = published_ms.to_bits();
        loop {
            let current_bits = self.longest_bits.load(Ordering::Relaxed);
            let current = f64::from_bits(current_bits);
            if published_ms >= current {
                return;
            }
            if self
                .longest_bits
                .compare_exchange_weak(
                    current_bits,
                    published,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return;
            }
        }
    }

    fn note_ms(&self, gap_ms: f64) {
        if !gap_ms.is_finite() || gap_ms < 0.0 {
            return;
        }
        let next = gap_ms.to_bits();
        loop {
            let current_bits = self.longest_bits.load(Ordering::Relaxed);
            if gap_ms <= f64::from_bits(current_bits) {
                return;
            }
            if self
                .longest_bits
                .compare_exchange_weak(current_bits, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }
}

fn nanos_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn ms_from_ns(ns: u64) -> f64 {
    let whole = ns / 1_000_000;
    let frac = ns % 1_000_000;
    let whole = match u32::try_from(whole) {
        Ok(value) => f64::from(value),
        Err(_) => f64::from(u32::MAX),
    };
    let frac = match u32::try_from(frac) {
        Ok(value) => f64::from(value),
        Err(_) => 0.0,
    };
    whole + frac / 1_000_000.0
}

thread_local! {
    static SEAT_RAW: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SEAT_METER: std::cell::RefCell<Option<Arc<StallMeter>>> =
        const { std::cell::RefCell::new(None) };
    static PUMP_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub(crate) fn remember_seat(raw: *mut c_void, meter: &Arc<StallMeter>) {
    SEAT_RAW.with(|cell| cell.set(raw as usize));
    SEAT_METER.with(|cell| *cell.borrow_mut() = Some(Arc::clone(meter)));
}

pub(crate) fn forget_seat(raw: *mut c_void) {
    let addr = raw as usize;
    SEAT_RAW.with(|cell| {
        if cell.get() == addr {
            cell.set(0);
        }
    });
    if SEAT_RAW.with(std::cell::Cell::get) == 0 {
        SEAT_METER.with(|cell| *cell.borrow_mut() = None);
    }
}

extern "C" {
    fn genos_window_dispatch(window: *mut c_void);
}

struct SeatPump {
    stop: Arc<std::sync::atomic::AtomicBool>,
    pair: Arc<(Mutex<()>, Condvar)>,
    handle: Option<std::thread::JoinHandle<()>>,
    meter: Arc<StallMeter>,
}

thread_local! {
    static SEAT_PUMP: std::cell::RefCell<Option<SeatPump>> = const { std::cell::RefCell::new(None) };
}

/// Start answering compositor pings. Pair this with [`seat_leave`].
///
/// The frame thread keeps running the device work. A nested enter does not
/// start a second thread. No seat installed: this only counts the nesting.
pub fn seat_enter() {
    let depth = PUMP_DEPTH.with(std::cell::Cell::get);
    PUMP_DEPTH.with(|cell| cell.set(depth.saturating_add(1)));
    if depth > 0 {
        return;
    }
    let raw = SEAT_RAW.with(std::cell::Cell::get);
    let meter = SEAT_METER.with(|cell| cell.borrow().clone());
    let Some(meter) = meter else {
        return;
    };
    if raw == 0 {
        return;
    }
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let pair = Arc::new((Mutex::new(()), Condvar::new()));
    let wait = Arc::clone(&pair);
    let meter_thread = Arc::clone(&meter);
    let handle = std::thread::Builder::new()
        .name("genos-seat".into())
        .spawn(move || {
            let window = raw as *mut c_void;
            while !flag.load(Ordering::Acquire) {
                meter_thread.mark();
                // SAFETY: `window` is the seat installed on this thread. The
                // frame thread does not dispatch Wayland until this loop ends.
                // `genos_window_dispatch` only reads the socket and answers pings.
                unsafe { genos_window_dispatch(window) };
                let guard = match wait.0.lock() {
                    Ok(guard) => guard,
                    Err(poison) => poison.into_inner(),
                };
                let _ = wait.1.wait_timeout(guard, Duration::from_millis(16));
            }
            meter_thread.mark();
        });
    SEAT_PUMP.with(|cell| {
        *cell.borrow_mut() = Some(SeatPump {
            stop,
            pair,
            handle: handle.ok(),
            meter,
        });
    });
}

/// Join the seat thread started by [`seat_enter`].
pub fn seat_leave() {
    let depth = PUMP_DEPTH.with(std::cell::Cell::get);
    if depth == 0 {
        return;
    }
    PUMP_DEPTH.with(|cell| cell.set(depth - 1));
    if depth > 1 {
        return;
    }
    let pump = SEAT_PUMP.with(|cell| cell.borrow_mut().take());
    let Some(pump) = pump else {
        return;
    };
    pump.stop.store(true, Ordering::Release);
    pump.pair.1.notify_one();
    if let Some(handle) = pump.handle {
        let _ = handle.join();
    }
    pump.meter.mark();
}

/// Run `body` on this thread. While it runs, another thread dispatches the
/// seat so a compositor ping is answered. `body` must not call Wayland.
///
/// No seat installed: `body` runs as it does today.
pub fn block_with_installed_pump(body: Box<dyn FnOnce()>) {
    seat_enter();
    body();
    seat_leave();
}

/// What the pump callback tells [`run_pumped`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PumpStep {
    /// Keep waiting for the worker.
    Continue,
    /// The window is closing. Stop without the worker's value.
    Close,
}

/// Run `work` on a worker. Call `pump` on this thread until the worker
/// finishes. The gap between pumps stays inside `budget` when `pump` itself
/// returns promptly.
pub fn run_pumped<T: Send + 'static>(
    meter: &StallMeter,
    budget: Duration,
    mut pump: impl FnMut() -> PumpStep,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("genos-scene-load".into())
        .spawn(move || {
            let _ = tx.send(work());
        });
    if spawned.is_err() {
        return Err("scene load thread did not start".into());
    }
    let slice = slice_of(budget);
    loop {
        meter.mark();
        if pump() == PumpStep::Close {
            return Err("window closed during load".into());
        }
        match rx.recv_timeout(slice) {
            Ok(value) => {
                meter.mark();
                return Ok(value);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("scene load stopped".into());
            }
        }
    }
}

fn slice_of(budget: Duration) -> Duration {
    let half = budget / 2;
    if half < Duration::from_millis(1) {
        budget
    } else {
        half
    }
}

/// Run `leave` (destroy the window), then `drop_heavy` on another thread.
/// Returns the time spent on this thread. The heavy drop is not joined.
pub fn leave_seat_then<T: Send + 'static>(
    leave: impl FnOnce(),
    heavy: T,
    drop_heavy: impl FnOnce(T) + Send + 'static,
) -> Duration {
    let started = Instant::now();
    leave();
    let _ = std::thread::Builder::new()
        .name("genos-teardown".into())
        .spawn(move || drop_heavy(heavy));
    started.elapsed()
}

/// Free `value` off the frame thread. A failed spawn drops it here.
pub fn retire<T: Send + 'static>(value: T) {
    let _ = std::thread::Builder::new()
        .name("genos-retire".into())
        .spawn(move || drop(value));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_ms(ms: u64) {
        let pair = (Mutex::new(()), Condvar::new());
        let guard = match pair.0.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        let _ = pair.1.wait_timeout(guard, Duration::from_millis(ms));
    }

    #[test]
    fn a_gap_on_this_thread_is_the_stall() {
        let meter = StallMeter::new();
        meter.mark();
        wait_ms(120);
        meter.mark();
        println!("MEASURE before_pump_stall_ms={:.1}", meter.longest_ms());
        assert!(
            meter.longest_ms() >= 100.0,
            "a blocked pump records the gap, got {}",
            meter.longest_ms()
        );
    }

    #[test]
    fn a_worker_load_stays_inside_the_budget() {
        let meter = StallMeter::new();
        let started = Instant::now();
        let value = run_pumped(
            &meter,
            PUMP_BUDGET,
            || PumpStep::Continue,
            || {
                wait_ms(300);
                7_u32
            },
        )
        .expect("load");
        assert_eq!(value, 7);
        assert!(started.elapsed() >= Duration::from_millis(300));
        println!("MEASURE after_load_stall_ms={:.1}", meter.longest_ms());
        assert!(
            meter.longest_ms() <= 100.0,
            "load stalled the pump for {} ms",
            meter.longest_ms()
        );
    }

    #[test]
    fn quit_leaves_the_seat_before_a_slow_teardown() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        let elapsed = leave_seat_then(
            {
                let order = Arc::clone(&order);
                move || {
                    order
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push("window");
                }
            },
            (),
            {
                let order = Arc::clone(&order);
                move |()| {
                    order
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push("gpu");
                    wait_ms(400);
                    let _ = tx.send(());
                }
            },
        );
        println!(
            "MEASURE after_quit_ms={:.1}",
            elapsed.as_secs_f64() * 1000.0
        );
        assert!(
            elapsed < PUMP_BUDGET,
            "quit blocked the frame thread for {elapsed:?}"
        );
        let _ = rx.recv_timeout(Duration::from_secs(2));
        let steps = order
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert_eq!(steps.first().copied(), Some("window"));
        assert_eq!(steps.get(1).copied(), Some("gpu"));
    }
}
