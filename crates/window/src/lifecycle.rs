//! App lifecycle. The event loop owns shutdown.
//!
//! Hyprland (Omarchy shows the same dialog) sends `xdg_wm_base.ping` while a
//! window is mapped and treats a missed answer as "Application not responding".
//! The answer is written only when this thread dispatches the seat. A quit that
//! blocks here, in `vkDeviceWaitIdle`, a thread join, or device destruction,
//! misses that ping.
//!
//! PR #63 (merged as `beb9a88`) fixed that only on `genos-lighting`: join the
//! script thread, stop MCP, drop the renderer while the window was still mapped
//! so a side thread could answer pings, then destroy the device. `genos-camera`
//! and `genos-stress` kept a different quit: clear the seat pump, unmap, abandon
//! the device, and `process::exit`. The driver then ran `vkDeviceWaitIdle` from
//! its exit handler while the compositor still had the client. New binaries
//! copied that second path.
//!
//! [`Lifecycle`] is the only owner of that sequence. The frame calls [`Lifecycle::quit`]
//! or drops the lifecycle. Either way the state machine runs on this thread:
//! stop producers, poll GPU fences, destroy resources, unmap, then exit.
//! [`Lifecycle::pump`] is not required. The machine itself dispatches the seat
//! between steps, so the gap between dispatches stays inside [`PUMP_BUDGET`].
//!
//! Safe code cannot move the window or the device out. Both sit in
//! [`ManuallyDrop`]. [`exit`] is the only way out of the process. It flushes
//! and calls `_exit`, so the driver's atexit handler does not run.

use std::io::Write;
use std::mem::ManuallyDrop;
use std::sync::{Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{Window, PUMP_BUDGET};

/// Printed on stdout when shutdown finishes. The quit-gap test reads it.
pub const QUIT_GAP_PREFIX: &str = "lifecycle_quit_gap_ms=";

/// One finished shutdown phase: `lifecycle_phase=<name> ms=<milliseconds>`.
pub const PHASE_PREFIX: &str = "lifecycle_phase=";

/// Printed when shutdown starts: presents the device completed before quit.
pub const PRESENTED_PREFIX: &str = "lifecycle_frames_presented=";

/// Printed immediately before the process image is replaced. Wall time from
/// the quit request through every drop after the window unmaps.
pub const QUIT_TOTAL_PREFIX: &str = "lifecycle_quit_total_ms=";

/// How long a producer may stay unfinished before shutdown detaches it.
const PRODUCER_LIMIT: Duration = Duration::from_secs(2);

/// Bound on the producer stop and the fence drain. A stuck fence must not
/// hold quit: destruction runs anyway, with the seat thread covering the
/// driver call. Two frames fit under the 100 ms ping budget.
const PHASE_LIMIT: Duration = Duration::from_millis(48);

/// Pause between polls. Short enough that the dispatch gap stays inside the budget.
const SLICE: Duration = Duration::from_millis(16);

/// Shutdown phases, in order. The seat is dispatched before each step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The frame is drawing.
    Running,
    /// Script threads and the MCP listener are stopping. Polled, not joined blindly.
    StopProducers,
    /// GPU fences are polled. No `vkDeviceWaitIdle`.
    DrainGpu,
    /// Device resources are destroyed. Each step returns before the next dispatch.
    DestroyResources,
    /// The window is unmapped.
    Unmap,
    /// The process is about to exit.
    Exit,
}

/// What the seat should do during one shutdown step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SeatOp {
    Dispatch,
    Unmap,
}

/// A device the lifecycle can drain and destroy without blocking the seat.
///
/// `genos-render`'s `Renderer` is the production implementation. The methods
/// must not call `vkDeviceWaitIdle` and must not block.
/// [`ShutdownDevice::destroy_step`] may run several times. Each call destroys
/// the next group and returns `false` until nothing remains.
pub trait ShutdownDevice {
    /// Called once, before producers stop. Drop seat-side blockers here.
    fn prepare_shutdown(&mut self) {}

    /// `true` when every in-flight fence is signaled.
    fn poll_idle(&mut self) -> bool;

    /// Destroy the next resource group. `true` when the device is gone.
    fn destroy_step(&mut self) -> bool;

    /// Name of the group [`ShutdownDevice::destroy_step`] will destroy next.
    fn next_phase(&self) -> &'static str {
        "destroy"
    }

    /// Block once inside [`ShutdownDevice::poll_idle`], with no seat dispatch.
    ///
    /// The quit harness uses this as a slow fence. Lavapipe signals real fences
    /// immediately, so a regression that waits inside the poll still has to
    /// show up in that phase's time.
    fn plant_slow_fence(&mut self, _stall: Duration) {}

    /// Presents the driver accepted. The quit line reports it, so a run that
    /// lost its surface before the first frame cannot pass as a clean quit.
    fn frames_presented(&self) -> u64 {
        0
    }
}

/// A worker the lifecycle joins without blocking the seat.
///
/// [`ThreadStop::release`] joins once the worker has finished and otherwise
/// detaches. [`ThreadStop::poll`] waits up to two seconds first. [`Drop`]
/// detaches a worker that is still running, so a leftover handle does not
/// join on the event thread.
pub struct ThreadStop<T> {
    handle: Option<JoinHandle<T>>,
    result: Option<T>,
    started: Option<Instant>,
    limit: Duration,
}

impl<T> ThreadStop<T> {
    /// Take ownership of `handle`. The lifecycle polls it during shutdown.
    pub fn new(handle: JoinHandle<T>) -> Self {
        Self {
            handle: Some(handle),
            result: None,
            started: None,
            limit: PRODUCER_LIMIT,
        }
    }

    /// The worker's value, after a successful join.
    pub fn result(&self) -> Option<&T> {
        self.result.as_ref()
    }

    /// Join if the worker has already finished. Does not start the detach timer.
    pub fn try_join(&mut self) -> bool {
        if self.handle.is_none() {
            return true;
        }
        self.take_finished()
    }

    /// Join if the worker has finished. Otherwise detach. Does not wait.
    ///
    /// Shutdown uses this. Waiting out [`PRODUCER_LIMIT`] holds quit past the
    /// seat budget, and joining a worker blocked on the frame thread deadlocks.
    pub fn release(&mut self) -> bool {
        if self.handle.is_none() {
            return true;
        }
        if self.take_finished() {
            return true;
        }
        if let Some(handle) = self.handle.take() {
            // Detach. The process is leaving; the worker must not be joined here.
            std::mem::forget(handle);
        }
        true
    }

    /// Join, or detach once [`PRODUCER_LIMIT`] has passed. `true` when done.
    pub fn poll(&mut self) -> bool {
        if self.handle.is_none() {
            return true;
        }
        if self.take_finished() {
            return true;
        }
        let started = self.started.get_or_insert_with(Instant::now);
        if started.elapsed() >= self.limit {
            if let Some(handle) = self.handle.take() {
                // Detach. Joining here would block the event thread.
                std::mem::forget(handle);
            }
            return true;
        }
        false
    }

    fn take_finished(&mut self) -> bool {
        let finished = self.handle.as_ref().is_some_and(JoinHandle::is_finished);
        if !finished {
            return false;
        }
        if let Some(handle) = self.handle.take() {
            if let Ok(value) = handle.join() {
                self.result = Some(value);
            }
        }
        true
    }
}

impl<T> Drop for ThreadStop<T> {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        if handle.is_finished() {
            let _ = handle.join();
        } else {
            std::mem::forget(handle);
        }
    }
}

type Stop = Box<dyn FnMut() -> bool>;

/// Token from opening a device. [`Lifecycle::new`] consumes it.
///
/// Dropping a live permit panics. The opener returns the permit, and
/// [`Lifecycle::new`] is the constructor that disarms it.
#[must_use = "pass the permit to Lifecycle::new"]
pub struct ShutdownPermit {
    live: bool,
}

impl ShutdownPermit {
    /// Issue a permit for a device the lifecycle will own.
    pub fn issue() -> Self {
        Self { live: true }
    }

    fn release(&mut self) {
        self.live = false;
    }
}

impl Drop for ShutdownPermit {
    #[expect(
        clippy::panic,
        reason = "a live permit dropped outside Lifecycle is a programming error"
    )]
    fn drop(&mut self) {
        if !self.live || std::thread::panicking() {
            return;
        }
        // A live permit means the device was never handed to the lifecycle.
        panic!("genos-window: the device permit was dropped outside the app lifecycle");
    }
}

/// The process's window and device. Shutdown is a state machine on the event thread.
///
/// Construct it with the open window, the device, and the [`ShutdownPermit`] from
/// the open. Draw through [`Self::window`] and [`Self::device`]. Call [`Self::quit`]
/// to leave. Dropping it runs the same machine and returns, so a `?` on the way
/// out still answers pings.
pub struct Lifecycle<D: ShutdownDevice> {
    /// The mapped window. Safe code cannot move it out.
    pub window: ManuallyDrop<Window>,
    /// The device. Safe code cannot move it out. Only shutdown destroys it.
    pub device: ManuallyDrop<D>,
    stops: Vec<Stop>,
    quit: bool,
    exit_code: i32,
    window_alive: bool,
    device_dropped: bool,
    released: bool,
    longest: Duration,
    drain_forced: bool,
}

impl<D: ShutdownDevice> Lifecycle<D> {
    /// Own `window` and `device` until shutdown. `permit` is the token from the open.
    pub fn new(window: Window, device: D, mut permit: ShutdownPermit) -> Self {
        permit.release();
        drop(permit);
        Self {
            window: ManuallyDrop::new(window),
            device: ManuallyDrop::new(device),
            stops: Vec::new(),
            quit: false,
            exit_code: 0,
            window_alive: true,
            device_dropped: false,
            released: false,
            longest: Duration::ZERO,
            drain_forced: false,
        }
    }

    /// Register a producer stop. Return `true` when that producer is gone.
    ///
    /// The machine calls every stop between seat dispatches until all of them
    /// return `true`. A stop must not block. Poll a [`ThreadStop`] instead of joining.
    pub fn add_stop(&mut self, step: impl FnMut() -> bool + 'static) {
        self.stops.push(Box::new(step));
    }

    /// Block once inside the device's fence poll, with no seat dispatch.
    ///
    /// The quit-gap test passes `--lifecycle-block-ms` so a regression that
    /// stalls teardown fails the 100 ms budget. Shipping runs leave this unset.
    pub fn plant_block(&mut self, stall: Duration) {
        self.device.plant_slow_fence(stall);
    }

    /// Remember a quit request. The caller still has to [`Self::quit`] or drop.
    pub fn request_quit(&mut self, code: i32) {
        if !self.quit {
            self.quit = true;
            self.exit_code = code;
        }
    }

    /// `true` after [`Self::request_quit`].
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// The code from the first [`Self::request_quit`], or 0.
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    /// Longest dispatch gap recorded during shutdown, in milliseconds.
    pub fn quit_gap_ms(&self) -> f64 {
        duration_ms(self.longest)
    }

    /// Run shutdown, then [`exit`] with `code`.
    ///
    /// Producers registered with [`Self::add_stop`] run first. The device is
    /// destroyed while the window is still mapped. The window is unmapped only
    /// after that. `_exit` does not run destructors, so this drops the
    /// device and the window first.
    pub fn quit(mut self, code: i32) -> ! {
        if !self.quit {
            self.exit_code = code;
            self.quit = true;
        } else if code != 0 && self.exit_code == 0 {
            self.exit_code = code;
        }
        let code = self.exit_code;
        self.shutdown();
        unsafe {
            if self.window_alive {
                ManuallyDrop::drop(&mut self.window);
                self.window_alive = false;
            }
        }
        self.drop_device();
        std::mem::forget(self);
        exit(code);
    }

    fn shutdown(&mut self) {
        if self.released {
            return;
        }
        println!("{PRESENTED_PREFIX}{}", self.device.frames_presented());
        begin_quit_clock();
        let prepared = Instant::now();
        phase_begin("prepare");
        // The seat stays installed. Destroy calls dispatch it from a side
        // thread so a driver round-trip can finish. It is cleared when the
        // window is unmapped.
        self.device.prepare_shutdown();
        phase_end("prepare", prepared.elapsed());
        let device = &mut *self.device;
        let stops = &mut self.stops;
        let window = &mut self.window;
        let alive = &mut self.window_alive;
        let stop = drive_machine(device, stops, |op| match op {
            SeatOp::Dispatch => {
                if *alive {
                    let _ = window.pump();
                }
            }
            SeatOp::Unmap => {
                if *alive {
                    window.clear_seat_pump();
                    // SAFETY: `window` was created in `new` and this flag is the
                    // only other drop. The compositor is told the window is gone.
                    unsafe { ManuallyDrop::drop(window) };
                    *alive = false;
                }
            }
        });
        self.longest = stop.gap;
        self.drain_forced = stop.forced;
        report_gap(stop.gap, stop.forced);
        self.released = true;
    }

    fn drop_device(&mut self) {
        if self.device_dropped {
            return;
        }
        let started = Instant::now();
        phase_begin("device_drop");
        // SAFETY: the flag is the only other drop of this device.
        // Shutdown already destroyed the GPU objects when it ran.
        unsafe { ManuallyDrop::drop(&mut self.device) };
        self.device_dropped = true;
        phase_end("device_drop", started.elapsed());
        mark_quit_tail();
    }
}

impl<D: ShutdownDevice> Drop for Lifecycle<D> {
    fn drop(&mut self) {
        if !self.released {
            self.shutdown();
        }
        unsafe {
            if self.window_alive {
                // SAFETY: the flag is the only other drop of this window.
                ManuallyDrop::drop(&mut self.window);
                self.window_alive = false;
            }
        }
        self.drop_device();
    }
}

/// `true` when `ms` is inside the seat budget. The quit-gap test uses this
/// predicate, including on the planted stall, so a block cannot pass quietly.
pub fn gap_within_budget(ms: f64) -> bool {
    ms.is_finite() && ms >= 0.0 && ms < duration_ms(PUMP_BUDGET)
}

/// Read [`QUIT_GAP_PREFIX`] from a child's stdout or stderr.
pub fn parse_quit_gap(text: &str) -> Option<f64> {
    prefixed_f64(text, QUIT_GAP_PREFIX)
}

/// One `lifecycle_phase=<name> ms=<milliseconds>` line.
#[derive(Clone, Debug, PartialEq)]
pub struct PhaseSample {
    pub name: String,
    pub ms: f64,
}

/// Every finished phase line in `text`, in order.
pub fn parse_quit_phases(text: &str) -> Vec<PhaseSample> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(PHASE_PREFIX) else {
            continue;
        };
        let Some((name, ms)) = rest.split_once(" ms=") else {
            continue;
        };
        let Ok(ms) = ms.parse() else {
            continue;
        };
        out.push(PhaseSample {
            name: name.to_string(),
            ms,
        });
    }
    out
}

/// Read [`PRESENTED_PREFIX`] from a child's stdout or stderr.
pub fn parse_frames_presented(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(PRESENTED_PREFIX))
        .and_then(|value| value.trim().parse().ok())
}

/// Read [`QUIT_TOTAL_PREFIX`] from a child's stdout or stderr.
pub fn parse_quit_total(text: &str) -> Option<f64> {
    prefixed_f64(text, QUIT_TOTAL_PREFIX)
}

/// `true` when every phase and the total stay inside the seat budget.
pub fn phases_within_budget(phases: &[PhaseSample], total_ms: f64) -> bool {
    gap_within_budget(total_ms) && phases.iter().all(|phase| gap_within_budget(phase.ms))
}

fn prefixed_f64(text: &str, prefix: &str) -> Option<f64> {
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(prefix) else {
            continue;
        };
        return rest.parse().ok();
    }
    None
}

/// `Some(true)` when `--lifecycle-block-ms` takes the next argument.
/// `Some(false)` when the value is inline (`--lifecycle-block-ms=N`).
/// `None` for any other argument.
pub fn block_ms_needs_value(arg: &str) -> Option<bool> {
    if arg == "--lifecycle-block-ms" {
        Some(true)
    } else if arg.starts_with("--lifecycle-block-ms=") {
        Some(false)
    } else {
        None
    }
}

/// Parse `--lifecycle-block-ms`. `next` is the following argument when
/// [`block_ms_needs_value`] returned `Some(true)`.
pub fn parse_block_ms(arg: &str, next: Option<&str>) -> Result<Duration, String> {
    let inline = arg.strip_prefix("--lifecycle-block-ms=");
    let text = if let Some(value) = inline {
        if value.is_empty() {
            return Err("--lifecycle-block-ms needs a value".into());
        }
        value
    } else if arg == "--lifecycle-block-ms" {
        next.ok_or("--lifecycle-block-ms needs a value")?
    } else {
        return Err(format!("{arg} is not --lifecycle-block-ms"));
    };
    let ms: u64 = text
        .parse()
        .map_err(|_| format!("bad --lifecycle-block-ms {text}"))?;
    Ok(Duration::from_millis(ms))
}

/// `true` when the argument is the early-close test hook.
pub fn close_early_flag(arg: &str) -> bool {
    arg == "--lifecycle-close-early"
}

/// `cli` when the flag was passed. Otherwise `GENOS_LIFECYCLE_CLOSE_EARLY`
/// set to `1` or `true`.
///
/// A value that is anything else is ignored. The flag wins when both are set,
/// because the caller passes `true` only for the argument.
#[allow(
    clippy::disallowed_methods,
    reason = "GENOS_LIFECYCLE_CLOSE_EARLY mirrors --lifecycle-close-early. The argument wins when cli is true. docs/LINT_RULES.md"
)]
pub fn close_early_mirror(cli: bool) -> bool {
    if cli {
        return true;
    }
    matches!(
        std::env::var("GENOS_LIFECYCLE_CLOSE_EARLY").as_deref(),
        Ok("1" | "true")
    )
}

/// `cli` when the flag was passed. Otherwise `GENOS_LIFECYCLE_BLOCK_MS`.
///
/// A value that does not parse is ignored. The flag wins when both are set,
/// because the caller passes `Some` only for the argument.
#[allow(
    clippy::disallowed_methods,
    reason = "GENOS_LIFECYCLE_BLOCK_MS mirrors --lifecycle-block-ms. The argument wins when cli is Some. docs/LINT_RULES.md"
)]
pub fn block_ms_mirror(cli: Option<Duration>) -> Option<Duration> {
    if cli.is_some() {
        return cli;
    }
    let Ok(text) = std::env::var("GENOS_LIFECYCLE_BLOCK_MS") else {
        return None;
    };
    let Ok(ms) = text.parse::<u64>() else {
        return None;
    };
    Some(Duration::from_millis(ms))
}

extern "C" {
    fn _exit(code: i32) -> !;
}

/// Leave the process. Binaries call this instead of `std::process::exit`.
///
/// Rust buffers stdout, so this flushes first. The total line is the wall time
/// from the quit request to this call, including drops after the window
/// unmapped. `_exit` skips the Vulkan driver's atexit handler. That handler
/// waits again after `vkDestroyInstance`, and on a real device the wait is the
/// multi-second tail after the quit lines.
pub fn exit(code: i32) -> ! {
    if let Some(tail) = take_quit_tail() {
        phase_end("exit_tail", tail);
    }
    if let Some(total) = quit_total() {
        println!("{QUIT_TOTAL_PREFIX}{:.3}", duration_ms(total));
    }
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // SAFETY: stdout is flushed. The lifecycle has destroyed the device and
    // unmapped the window before this call. `_exit` does not run atexit.
    unsafe { _exit(code) }
}

struct MachineStop {
    gap: Duration,
    forced: bool,
}

struct OpenPhase {
    name: &'static str,
    started: Instant,
}

fn drive_machine<D: ShutdownDevice>(
    device: &mut D,
    stops: &mut [Stop],
    mut seat: impl FnMut(SeatOp),
) -> MachineStop {
    let mut last = Instant::now();
    let mut longest = Duration::ZERO;
    let mut phase = Phase::StopProducers;
    let mut forced = false;
    let mut phase_deadline = Instant::now() + PHASE_LIMIT;
    let mut open: Option<OpenPhase> = None;
    let mut destroys = 0u8;
    let note = |last: &mut Instant, longest: &mut Duration| {
        let gap = last.elapsed();
        if gap > *longest {
            *longest = gap;
        }
        *last = Instant::now();
    };
    let close_open = |open: &mut Option<OpenPhase>| {
        if let Some(phase) = open.take() {
            phase_end(phase.name, phase.started.elapsed());
        }
    };
    let open_phase = |open: &mut Option<OpenPhase>, name: &'static str| {
        if open.as_ref().is_some_and(|phase| phase.name == name) {
            return;
        }
        if let Some(phase) = open.take() {
            phase_end(phase.name, phase.started.elapsed());
        }
        phase_begin(name);
        *open = Some(OpenPhase {
            name,
            started: Instant::now(),
        });
    };
    loop {
        seat(SeatOp::Dispatch);
        note(&mut last, &mut longest);
        match phase {
            Phase::Running | Phase::Exit => {
                close_open(&mut open);
                return MachineStop {
                    gap: longest,
                    forced,
                };
            }
            Phase::StopProducers => {
                open_phase(&mut open, "stop_producers");
                let done = stops.iter_mut().all(|stop| stop());
                if done || Instant::now() >= phase_deadline {
                    close_open(&mut open);
                    phase = Phase::DrainGpu;
                    phase_deadline = Instant::now() + PHASE_LIMIT;
                } else {
                    wait_for(SLICE);
                }
            }
            Phase::DrainGpu => {
                open_phase(&mut open, "drain_gpu");
                if device.poll_idle() {
                    close_open(&mut open);
                    phase = Phase::DestroyResources;
                } else if Instant::now() >= phase_deadline {
                    forced = true;
                    close_open(&mut open);
                    phase = Phase::DestroyResources;
                } else {
                    wait_for(SLICE);
                }
            }
            Phase::DestroyResources => {
                let name = device.next_phase();
                open_phase(&mut open, name);
                let done = device.destroy_step();
                destroys = destroys.saturating_add(1);
                close_open(&mut open);
                if done || destroys >= 32 {
                    phase = Phase::Unmap;
                }
            }
            Phase::Unmap => {
                close_open(&mut open);
                open_phase(&mut open, "unmap");
                seat(SeatOp::Unmap);
                close_open(&mut open);
                phase = Phase::Exit;
            }
        }
    }
}

fn phase_begin(name: &str) {
    println!("lifecycle_phase_begin={name}");
    let _ = std::io::stdout().flush();
}

fn phase_end(name: &str, took: Duration) {
    println!("{PHASE_PREFIX}{name} ms={:.3}", duration_ms(took));
    let _ = std::io::stdout().flush();
}

fn report_gap(gap: Duration, forced: bool) {
    let ms = duration_ms(gap);
    println!("{QUIT_GAP_PREFIX}{ms:.3}");
    if forced {
        println!("lifecycle_drain_forced=1");
    }
    let _ = std::io::stdout().flush();
}

struct QuitClock {
    start: Option<Instant>,
    tail: Option<Instant>,
}

fn quit_clock() -> std::sync::MutexGuard<'static, QuitClock> {
    static CLOCK: std::sync::Mutex<QuitClock> = std::sync::Mutex::new(QuitClock {
        start: None,
        tail: None,
    });
    CLOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn begin_quit_clock() {
    let mut clock = quit_clock();
    let now = Instant::now();
    clock.start = Some(now);
    clock.tail = None;
}

fn mark_quit_tail() {
    quit_clock().tail = Some(Instant::now());
}

fn take_quit_tail() -> Option<Duration> {
    quit_clock().tail.take().map(|start| start.elapsed())
}

fn quit_total() -> Option<Duration> {
    quit_clock().start.map(|start| start.elapsed())
}

fn duration_ms(gap: Duration) -> f64 {
    gap.as_secs_f64() * 1000.0
}

fn wait_for(duration: Duration) {
    if duration.is_zero() {
        return;
    }
    let deadline = Instant::now() + duration;
    let pair = (Mutex::new(()), Condvar::new());
    while Instant::now() < deadline {
        let guard = match pair.0.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        let _ = pair.1.wait_timeout(guard, remaining);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted {
        polls: u8,
        steps: u8,
        log: std::sync::Arc<Mutex<Vec<&'static str>>>,
        block: Option<Duration>,
        fence: Option<Duration>,
    }

    impl ShutdownDevice for Scripted {
        fn prepare_shutdown(&mut self) {
            self.push("prepare");
        }

        fn poll_idle(&mut self) -> bool {
            if let Some(fence) = self.fence.take() {
                // No seat dispatch. This is the slow-fence plant.
                wait_for(fence);
            }
            self.push("poll");
            self.polls = self.polls.saturating_add(1);
            self.polls >= 2
        }

        fn destroy_step(&mut self) -> bool {
            if let Some(block) = self.block.take() {
                wait_for(block);
            }
            self.push("destroy");
            self.steps = self.steps.saturating_add(1);
            self.steps >= 2
        }

        fn plant_slow_fence(&mut self, stall: Duration) {
            self.fence = Some(stall);
        }
    }

    impl Scripted {
        fn push(&self, step: &'static str) {
            self.log
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(step);
        }
    }

    fn log_of(log: &std::sync::Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
        log.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    #[test]
    fn shutdown_dispatches_between_stop_drain_destroy_and_unmap() {
        let log = std::sync::Arc::new(Mutex::new(Vec::new()));
        let mut device = Scripted {
            polls: 0,
            steps: 0,
            log: std::sync::Arc::clone(&log),
            block: None,
            fence: None,
        };
        let mut hits = 0u8;
        let mut stops: Vec<Stop> = vec![Box::new(move || {
            hits = hits.saturating_add(1);
            hits >= 2
        })];
        let mut seat_log = Vec::new();
        let stop = drive_machine(&mut device, &mut stops, |op| match op {
            SeatOp::Dispatch => seat_log.push("dispatch"),
            SeatOp::Unmap => seat_log.push("unmap"),
        });
        let gap = stop.gap;
        let forced = stop.forced;
        assert!(!forced);
        assert!(
            gap_within_budget(duration_ms(gap)),
            "idle shutdown gap {} ms",
            duration_ms(gap)
        );
        let trace = log_of(&log);
        let poll_at = trace.iter().position(|step| *step == "poll").expect("poll");
        let destroy_at = trace
            .iter()
            .position(|step| *step == "destroy")
            .expect("destroy");
        let unmap_at = seat_log
            .iter()
            .position(|step| *step == "unmap")
            .expect("unmap");
        assert!(poll_at < destroy_at, "{trace:?}");
        assert!(seat_log[..unmap_at].contains(&"dispatch"));
        assert!(seat_log.iter().filter(|step| **step == "dispatch").count() >= 4);
        assert_eq!(trace.iter().filter(|step| **step == "destroy").count(), 2);
        assert_eq!(trace.first().copied(), Some("poll"));
    }

    #[test]
    fn a_planted_block_fails_the_budget_predicate() {
        let log = std::sync::Arc::new(Mutex::new(Vec::new()));
        let mut device = Scripted {
            polls: 1,
            steps: 1,
            log,
            block: Some(Duration::from_millis(2000)),
            fence: None,
        };
        let mut stops: Vec<Stop> = Vec::new();
        let stop = drive_machine(&mut device, &mut stops, |_| {});
        let ms = duration_ms(stop.gap);
        println!("MEASURE planted_gap_ms={ms:.1}");
        assert!(
            !gap_within_budget(ms),
            "a 2s teardown block must fail the budget, gap {ms:.1} ms"
        );
        assert!(ms >= 1000.0, "planted gap was only {ms:.1} ms");
    }

    #[test]
    fn a_stall_between_dispatches_fails_the_same_predicate() {
        let log = std::sync::Arc::new(Mutex::new(Vec::new()));
        let mut device = Scripted {
            polls: 1,
            steps: 1,
            log,
            block: None,
            fence: Some(Duration::from_millis(2000)),
        };
        let mut stops: Vec<Stop> = Vec::new();
        let stop = drive_machine(&mut device, &mut stops, |_| {});
        let ms = duration_ms(stop.gap);
        assert!(!gap_within_budget(ms), "stall gap {ms:.1} ms");
    }

    #[test]
    fn the_gap_line_parses() {
        let text = "noise\nlifecycle_phase_begin=drain_gpu\nlifecycle_phase=drain_gpu ms=4.000\nlifecycle_phase=destroy_swapchain ms=80.500\nlifecycle_quit_gap_ms=12.500\nlifecycle_quit_total_ms=90.250\n";
        assert_eq!(parse_quit_gap(text), Some(12.5));
        assert_eq!(parse_quit_total(text), Some(90.25));
        let phases = parse_quit_phases(text);
        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].name, "drain_gpu");
        assert_eq!(phases[1].name, "destroy_swapchain");
        assert!(phases_within_budget(&phases, 90.25));
        assert!(!phases_within_budget(&phases, 2000.0));
        assert!(gap_within_budget(12.5));
        assert!(!gap_within_budget(100.0));
        assert!(!gap_within_budget(2000.0));
    }

    #[test]
    fn block_flag_parses_inline_and_separate() {
        assert_eq!(block_ms_needs_value("--lifecycle-block-ms"), Some(true));
        assert_eq!(block_ms_needs_value("--lifecycle-block-ms=20"), Some(false));
        assert_eq!(block_ms_needs_value("--frames"), None);
        assert_eq!(
            parse_block_ms("--lifecycle-block-ms", Some("40")).expect("parse"),
            Duration::from_millis(40)
        );
        assert_eq!(
            parse_block_ms("--lifecycle-block-ms=15", None).expect("inline"),
            Duration::from_millis(15)
        );
        assert!(parse_block_ms("--lifecycle-block-ms", None).is_err());
        assert!(close_early_flag("--lifecycle-close-early"));
        assert!(!close_early_flag("--frames"));
        assert!(close_early_mirror(true));
    }

    #[test]
    fn a_live_permit_drop_panics() {
        let permit = ShutdownPermit::issue();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(permit)));
        assert!(caught.is_err(), "a live permit must refuse to drop");
    }

    #[test]
    fn a_released_permit_drops() {
        let mut permit = ShutdownPermit::issue();
        permit.release();
        drop(permit);
    }
}
