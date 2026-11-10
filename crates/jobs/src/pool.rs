//! Run one frame of jobs. Serial mode is the oracle for parallel mode.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::deque::{Deque, Stolen};
use crate::schedule::{class_rank, conflict_batches, ScheduleError};
use crate::settings::JobSettings;
use crate::{Class, Order};

thread_local! {
    static IN_TASK: Cell<bool> = const { Cell::new(false) };
}

/// True while this thread is inside a job. A job that calls [`Scheduler::run`]
/// is rejected.
#[must_use]
pub fn in_task() -> bool {
    IN_TASK.with(Cell::get)
}

/// A clock the scheduler reads for the Must budget and per-label time.
///
/// Tests inject a manual clock. The debug binary uses [`InstantClock`].
pub trait Clock: Send + Sync {
    /// Milliseconds since an arbitrary origin.
    fn now_ms(&self) -> f64;
}

/// A clock the test moves by hand.
#[derive(Debug)]
pub struct ManualClock {
    bits: AtomicU64Time,
}

struct AtomicU64Time(std::sync::atomic::AtomicU64);

impl std::fmt::Debug for AtomicU64Time {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("AtomicU64Time")
            .field(&f64::from_bits(self.0.load(Ordering::Relaxed)))
            .finish()
    }
}

impl ManualClock {
    /// Start at `ms`.
    #[must_use]
    pub fn new(ms: f64) -> Self {
        Self {
            bits: AtomicU64Time(std::sync::atomic::AtomicU64::new(ms.to_bits())),
        }
    }

    /// Move the clock. Tasks and the test both call this.
    pub fn set(&self, ms: f64) {
        self.bits.0.store(ms.to_bits(), Ordering::Relaxed);
    }

    /// Share this clock with the scheduler.
    #[must_use]
    pub fn handle(clock: &Arc<Self>) -> Arc<dyn Clock> {
        let cloned: Arc<Self> = Arc::clone(clock);
        cloned
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> f64 {
        f64::from_bits(self.bits.0.load(Ordering::Relaxed))
    }
}

/// Wall time from construction.
#[derive(Debug)]
pub struct InstantClock {
    start: std::time::Instant,
}

impl InstantClock {
    /// Start now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }
}

impl Default for InstantClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for InstantClock {
    fn now_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }
}

/// One job. `T` is the value written to `slot[index]`.
pub struct Job<T> {
    /// Stage number. Stages run in ascending order.
    pub stage: u32,
    /// Slot inside the stage. Unique, and the set is `0..len` with no gaps.
    pub index: u32,
    /// Must, Soon, Slack, or Pumped.
    pub class: Class,
    /// Label for the per-system time probe.
    pub label: &'static str,
    /// Declared reads and writes. Empty means the job touches no shared key.
    pub access: Vec<crate::schedule::Access>,
    /// The work. It may only write its own slot plus data covered by `access`.
    pub work: Box<dyn FnOnce() -> T + Send>,
}

/// Slots for one stage, in index order. `None` was not run (deferred or rejected).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageSlots<T> {
    /// Stage number.
    pub stage: u32,
    /// `slot[index]`.
    pub slots: Vec<Option<T>>,
}

/// Counters for one [`Scheduler::run`].
#[derive(Clone, Debug)]
pub struct Stats {
    /// Milliseconds the Must class took on the injected clock.
    pub must_wall_ms: f64,
    /// Milliseconds the Soon class took.
    pub soon_wall_ms: f64,
    /// Milliseconds the Slack class took.
    pub slack_wall_ms: f64,
    /// Soon jobs left for the next frame.
    pub deferred: u32,
    /// Times Must crossed `frame_budget_ms`.
    pub over_budget: u32,
    /// Pumped jobs, plus a nested [`Scheduler::run`].
    pub rejected: u32,
    /// Jobs whose closure panicked.
    pub panics: u32,
    /// Successful steals.
    pub steals: u32,
    /// Slack jobs that ran while a seat was held. Stays 0 when the seat skips Slack.
    pub slack_during_fence: u32,
    /// Summed milliseconds per label.
    pub labels: Vec<(&'static str, f64)>,
}

/// One frame's slots and counters.
#[derive(Clone, Debug)]
pub struct Report<T> {
    /// Stages in ascending order.
    pub stages: Vec<StageSlots<T>>,
    /// Counters.
    pub stats: Stats,
}

/// The scheduler. Workers are spawned for a parallel batch and joined before
/// `run` returns. They are not kept across frames.
pub struct Scheduler {
    order: Order,
    workers: usize,
    budget_ms: f64,
    slack: bool,
    seat: AtomicU32,
    clock: Arc<dyn Clock>,
    rejected: AtomicU32,
    panics: AtomicU32,
    steals: AtomicU32,
    deferred: AtomicU32,
    over_budget: AtomicU32,
    slack_during_fence: AtomicU32,
    must_wall: AtomicU64,
    soon_wall: AtomicU64,
    slack_wall: AtomicU64,
    /// Planted bug for `debug/jobs --mutate`. `0` is correct code.
    plant: AtomicU8,
    /// Fail [`Self::run`]'s worker spawn once this many workers exist.
    /// `usize::MAX` spawns every worker. Tests use a small value.
    spawn_fail_after: AtomicUsize,
    /// When set, that spawn failure waits until a started worker has entered
    /// its job. Production leaves this clear.
    wait_for_entered: AtomicBool,
}

const PLANT_UNORDERED: u8 = 1;
const PLANT_SERVER: u8 = 2;
const PLANT_NESTED: u8 = 3;
const PLANT_SLACK: u8 = 4;
const PLANT_MISLABEL: u8 = 5;

impl Scheduler {
    /// Build from settings and a clock.
    #[must_use]
    pub fn new(settings: JobSettings, clock: Arc<dyn Clock>) -> Self {
        let workers = match settings.order {
            Order::Serial => 0,
            Order::Parallel => usize::try_from(settings.workers).unwrap_or(0),
        };
        Self {
            order: settings.order,
            workers,
            budget_ms: f64::from(settings.frame_budget_ms),
            slack: settings.slack,
            seat: AtomicU32::new(0),
            clock,
            rejected: AtomicU32::new(0),
            panics: AtomicU32::new(0),
            steals: AtomicU32::new(0),
            deferred: AtomicU32::new(0),
            over_budget: AtomicU32::new(0),
            slack_during_fence: AtomicU32::new(0),
            must_wall: AtomicU64::new(0),
            soon_wall: AtomicU64::new(0),
            slack_wall: AtomicU64::new(0),
            plant: AtomicU8::new(0),
            spawn_fail_after: AtomicUsize::new(usize::MAX),
            wait_for_entered: AtomicBool::new(false),
        }
    }

    /// Plant one `--mutate` bug. The next frames misbehave in that way.
    ///
    /// # Errors
    ///
    /// `name` is not a known mutation.
    pub fn plant(&self, name: &str) -> Result<(), String> {
        let code = match name {
            "unordered_slots" => PLANT_UNORDERED,
            "server_runs_parallel" => PLANT_SERVER,
            "nested_wait" => PLANT_NESTED,
            "slack_in_fence" => PLANT_SLACK,
            "mislabeled" => PLANT_MISLABEL,
            other => return Err(format!("unknown plant {other}")),
        };
        self.plant.store(code, Ordering::Relaxed);
        Ok(())
    }

    /// Serial, workers 0, a 6 ms budget, Slack on, manual clock at 0.
    #[must_use]
    pub fn serial() -> Self {
        Self::new(JobSettings::server(), Arc::new(ManualClock::new(0.0)))
    }

    /// Hold the Slack seat. The input fence calls this around pointer work.
    pub fn suspend_slack(&self) {
        self.seat.fetch_add(1, Ordering::Relaxed);
    }

    /// Release one Slack seat. Extra releases stay at zero.
    pub fn resume_slack(&self) {
        match self
            .seat
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |seat| {
                Some(seat.saturating_sub(1))
            }) {
            Ok(_) | Err(_) => {}
        }
    }

    /// Counters for the last frame, including a failed [`Self::run`].
    ///
    /// Wall times are the class totals from that frame, the same numbers
    /// [`Report::stats`] carries after a successful run. The next
    /// [`Self::run`] clears them first. A nested run does not.
    #[must_use]
    pub fn counters(&self) -> Stats {
        self.snapshot(
            load_ms(&self.must_wall),
            load_ms(&self.soon_wall),
            load_ms(&self.slack_wall),
            &[],
        )
    }

    /// Run one frame.
    ///
    /// # Errors
    ///
    /// A duplicate index, a gap, a full queue, a spawn failure, a panic, or a
    /// nested `run`.
    pub fn run<T: Send + 'static>(&self, jobs: Vec<Job<T>>) -> Result<Report<T>, ScheduleError> {
        let nested = in_task();
        if nested && self.plant.load(Ordering::Relaxed) != PLANT_NESTED {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(ScheduleError::Rejected);
        }
        if !nested {
            self.reset_frame();
        }
        let frame_start = self.clock.now_ms();
        let mut stages = BTreeMap::<u32, Vec<Job<T>>>::new();
        for job in jobs {
            stages.entry(job.stage).or_default().push(job);
        }
        let mut out = Vec::new();
        let mut labels = Vec::new();
        let mut must_wall = 0.0;
        let mut soon_wall = 0.0;
        let mut slack_wall = 0.0;
        for (stage, jobs) in stages {
            let mut slots = Vec::new();
            let count = check_indices(stage, &jobs)?;
            slots.resize_with(count, || None);
            let mut by_class = split_classes(jobs);
            for class in [Class::Must, Class::Soon, Class::Slack, Class::Pumped] {
                let class_jobs = by_class.remove(&class_rank(class)).unwrap_or_default();
                let wall =
                    self.run_class(class, class_jobs, &mut slots, &mut labels, frame_start)?;
                match class {
                    Class::Must => {
                        must_wall += wall;
                        store_ms(&self.must_wall, must_wall);
                    }
                    Class::Soon => {
                        soon_wall += wall;
                        store_ms(&self.soon_wall, soon_wall);
                    }
                    Class::Slack => {
                        slack_wall += wall;
                        store_ms(&self.slack_wall, slack_wall);
                    }
                    Class::Pumped => {}
                }
            }
            let plant = self.plant.load(Ordering::Relaxed);
            if plant == PLANT_UNORDERED || plant == PLANT_SERVER {
                slots.reverse();
            }
            out.push(StageSlots { stage, slots });
        }
        if must_wall > self.budget_ms {
            self.over_budget.fetch_add(1, Ordering::Relaxed);
        }
        Ok(Report {
            stages: out,
            stats: self.snapshot(must_wall, soon_wall, slack_wall, &labels),
        })
    }

    fn run_class<T: Send + 'static>(
        &self,
        class: Class,
        jobs: Vec<Job<T>>,
        slots: &mut [Option<T>],
        labels: &mut Vec<(&'static str, f64)>,
        frame_start: f64,
    ) -> Result<f64, ScheduleError> {
        if jobs.is_empty() {
            return Ok(0.0);
        }
        if class == Class::Pumped {
            self.rejected.fetch_add(
                u32::try_from(jobs.len()).unwrap_or(u32::MAX),
                Ordering::Relaxed,
            );
            return Ok(0.0);
        }
        let elapsed = self.clock.now_ms() - frame_start;
        if class == Class::Soon && elapsed >= self.budget_ms {
            self.deferred.fetch_add(
                u32::try_from(jobs.len()).unwrap_or(u32::MAX),
                Ordering::Relaxed,
            );
            return Ok(0.0);
        }
        if class == Class::Slack {
            let fenced = self.seat.load(Ordering::Relaxed) > 0;
            let forced = fenced && self.plant.load(Ordering::Relaxed) == PLANT_SLACK;
            if forced {
                self.slack_during_fence.fetch_add(
                    u32::try_from(jobs.len()).unwrap_or(u32::MAX),
                    Ordering::Relaxed,
                );
            } else if fenced || !self.slack || elapsed >= self.budget_ms {
                return Ok(0.0);
            }
        }
        let start = self.clock.now_ms();
        let parallel = self.order == Order::Parallel && self.workers > 0 && jobs.len() > 1;
        if parallel {
            self.run_parallel(jobs, slots, labels)?;
        } else {
            self.run_serial(jobs, slots, labels)?;
        }
        Ok(self.clock.now_ms() - start)
    }

    fn run_serial<T: Send + 'static>(
        &self,
        jobs: Vec<Job<T>>,
        slots: &mut [Option<T>],
        labels: &mut Vec<(&'static str, f64)>,
    ) -> Result<(), ScheduleError> {
        for job in self.relabel(jobs) {
            let index = usize::try_from(job.index).unwrap_or(0);
            let (value, elapsed, panicked) = invoke(self.clock.as_ref(), job.work);
            labels.push((job.label, elapsed));
            if panicked {
                self.panics.fetch_add(1, Ordering::Relaxed);
                return Err(ScheduleError::Panicked);
            }
            if let Some(slot) = slots.get_mut(index) {
                *slot = value;
            }
        }
        Ok(())
    }

    fn run_parallel<T: Send + 'static>(
        &self,
        jobs: Vec<Job<T>>,
        slots: &mut [Option<T>],
        labels: &mut Vec<(&'static str, f64)>,
    ) -> Result<(), ScheduleError> {
        let jobs = self.relabel(jobs);
        let access: Vec<Vec<_>> = jobs.iter().map(|job| job.access.clone()).collect();
        let batches = conflict_batches(&access);
        let mut parked: Vec<Option<Job<T>>> = jobs.into_iter().map(Some).collect();
        for batch in batches {
            let units = take_batch(&mut parked, &batch);
            run_batch(
                units,
                BatchLimits {
                    workers: self.workers,
                    spawn_fail_after: self.spawn_fail_after.load(Ordering::Relaxed),
                    wait_for_entered: self.wait_for_entered.load(Ordering::Relaxed),
                },
                Arc::clone(&self.clock),
                &self.steals,
                &self.panics,
                slots,
                labels,
            )?;
        }
        Ok(())
    }

    fn reset_frame(&self) {
        self.rejected.store(0, Ordering::Relaxed);
        self.panics.store(0, Ordering::Relaxed);
        self.steals.store(0, Ordering::Relaxed);
        self.deferred.store(0, Ordering::Relaxed);
        self.over_budget.store(0, Ordering::Relaxed);
        self.slack_during_fence.store(0, Ordering::Relaxed);
        store_ms(&self.must_wall, 0.0);
        store_ms(&self.soon_wall, 0.0);
        store_ms(&self.slack_wall, 0.0);
    }

    fn relabel<T>(&self, jobs: Vec<Job<T>>) -> Vec<Job<T>> {
        if self.plant.load(Ordering::Relaxed) != PLANT_MISLABEL {
            return jobs;
        }
        jobs.into_iter()
            .map(|mut job| {
                if job.label == "physics" {
                    job.label = "anim";
                }
                job
            })
            .collect()
    }

    fn snapshot(
        &self,
        must_wall_ms: f64,
        soon_wall_ms: f64,
        slack_wall_ms: f64,
        labels: &[(&'static str, f64)],
    ) -> Stats {
        Stats {
            must_wall_ms,
            soon_wall_ms,
            slack_wall_ms,
            deferred: self.deferred.load(Ordering::Relaxed),
            over_budget: self.over_budget.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            panics: self.panics.load(Ordering::Relaxed),
            steals: self.steals.load(Ordering::Relaxed),
            slack_during_fence: self.slack_during_fence.load(Ordering::Relaxed),
            labels: sum_labels(labels),
        }
    }
}

struct Unit<T> {
    index: usize,
    label: &'static str,
    work: Box<dyn FnOnce() -> T + Send>,
}

fn check_indices<T>(stage: u32, jobs: &[Job<T>]) -> Result<usize, ScheduleError> {
    let mut seen = vec![false; jobs.len()];
    for job in jobs {
        let index = usize::try_from(job.index).unwrap_or(usize::MAX);
        if index >= jobs.len() {
            return Err(ScheduleError::Gap { stage });
        }
        if seen[index] {
            return Err(ScheduleError::DuplicateIndex {
                stage,
                index: job.index,
            });
        }
        seen[index] = true;
    }
    if seen.iter().any(|flag| !flag) {
        return Err(ScheduleError::Gap { stage });
    }
    Ok(jobs.len())
}

fn split_classes<T>(jobs: Vec<Job<T>>) -> BTreeMap<u8, Vec<Job<T>>> {
    let mut map: BTreeMap<u8, Vec<Job<T>>> = BTreeMap::new();
    for job in jobs {
        map.entry(class_rank(job.class)).or_default().push(job);
    }
    for class_jobs in map.values_mut() {
        class_jobs.sort_by_key(|job| job.index);
    }
    map
}

fn take_batch<T: Send + 'static>(jobs: &mut [Option<Job<T>>], batch: &[usize]) -> Vec<Unit<T>> {
    let mut units = Vec::new();
    for position in batch {
        let Some(job) = jobs.get_mut(*position).and_then(Option::take) else {
            continue;
        };
        units.push(Unit {
            index: usize::try_from(job.index).unwrap_or(0),
            label: job.label,
            work: job.work,
        });
    }
    units
}

fn join_workers<T>(handles: Vec<thread::JoinHandle<()>>, share: &Share<T>) {
    share.stop.store(true, Ordering::Release);
    for handle in handles {
        let _ = handle.join();
    }
}

fn store_ms(slot: &AtomicU64, ms: f64) {
    slot.store(ms.to_bits(), Ordering::Relaxed);
}

fn load_ms(slot: &AtomicU64) -> f64 {
    f64::from_bits(slot.load(Ordering::Relaxed))
}

fn invoke<T>(clock: &dyn Clock, work: Box<dyn FnOnce() -> T + Send>) -> (Option<T>, f64, bool) {
    IN_TASK.with(|flag| flag.set(true));
    let start = clock.now_ms();
    let value = panic::catch_unwind(AssertUnwindSafe(work));
    let elapsed = clock.now_ms() - start;
    IN_TASK.with(|flag| flag.set(false));
    match value {
        Ok(value) => (Some(value), elapsed, false),
        Err(_) => (None, elapsed, true),
    }
}

#[derive(Clone, Copy)]
struct BatchLimits {
    workers: usize,
    spawn_fail_after: usize,
    wait_for_entered: bool,
}

fn run_batch<T: Send + 'static>(
    units: Vec<Unit<T>>,
    limits: BatchLimits,
    clock: Arc<dyn Clock>,
    steals: &AtomicU32,
    panics: &AtomicU32,
    slots: &mut [Option<T>],
    labels: &mut Vec<(&'static str, f64)>,
) -> Result<(), ScheduleError> {
    if units.len() <= 1 {
        for unit in units {
            let (value, elapsed, panicked) = invoke(clock.as_ref(), unit.work);
            labels.push((unit.label, elapsed));
            if panicked {
                panics.fetch_add(1, Ordering::Relaxed);
                return Err(ScheduleError::Panicked);
            }
            if let Some(slot) = slots.get_mut(unit.index) {
                *slot = value;
            }
        }
        return Ok(());
    }
    let runners = limits.workers + 1;
    let capacity = units.len().next_power_of_two().max(2);
    let mut deques = Vec::with_capacity(runners);
    for _ in 0..runners {
        deques.push(Arc::new(Deque::new(capacity)));
    }
    let count = units.len();
    for (index, unit) in units.into_iter().enumerate() {
        let Some(deque) = deques.get(index % runners) else {
            return Err(ScheduleError::Full);
        };
        if !deque.push(unit) {
            return Err(ScheduleError::Full);
        }
    }
    let share = Share {
        remaining: Arc::new(AtomicUsize::new(count)),
        stop: Arc::new(AtomicBool::new(false)),
        entered: Arc::new(AtomicBool::new(false)),
        written: Arc::new(Mutex::new(Vec::new())),
        labels: Arc::new(Mutex::new(Vec::new())),
        clock,
        steals: Arc::new(AtomicU32::new(0)),
        panics: Arc::new(AtomicU32::new(0)),
    };
    let mut handles = Vec::new();
    for id in 1..runners {
        if handles.len() >= limits.spawn_fail_after {
            if limits.wait_for_entered {
                while !share.entered.load(Ordering::Acquire) {
                    thread::yield_now();
                }
            }
            join_workers(handles, &share);
            return Err(ScheduleError::Spawn);
        }
        let Some(owner) = deques.get(id) else {
            join_workers(handles, &share);
            return Err(ScheduleError::Spawn);
        };
        let owner = Arc::clone(owner);
        let all = deques.clone();
        let share_for_worker = share.clone_handles();
        let spawned = thread::Builder::new()
            .name(format!("genos-job-{id}"))
            .spawn(move || worker_loop(&owner, &all, &share_for_worker));
        match spawned {
            Ok(handle) => handles.push(handle),
            Err(_err) => {
                join_workers(handles, &share);
                return Err(ScheduleError::Spawn);
            }
        }
    }
    let owner = Arc::clone(deques.first().ok_or(ScheduleError::Spawn)?);
    worker_loop(&owner, &deques, &share);
    for handle in handles {
        if handle.join().is_err() {
            share.panics.fetch_add(1, Ordering::Relaxed);
        }
    }
    steals.fetch_add(share.steals.load(Ordering::Relaxed), Ordering::Relaxed);
    let panic_count = share.panics.load(Ordering::Relaxed);
    if panic_count > 0 {
        panics.fetch_add(panic_count, Ordering::Relaxed);
    }
    for (index, value) in lock(&share.written).drain(..) {
        if let Some(slot) = slots.get_mut(index) {
            *slot = Some(value);
        }
    }
    labels.extend(lock(&share.labels).drain(..));
    if panic_count > 0 {
        return Err(ScheduleError::Panicked);
    }
    Ok(())
}

struct Share<T> {
    remaining: Arc<AtomicUsize>,
    /// Shutdown. Distinct from `remaining`, which only counts finished jobs.
    stop: Arc<AtomicBool>,
    /// A worker has entered `invoke`. Tests wait on this before shutdown.
    entered: Arc<AtomicBool>,
    written: Arc<Mutex<Vec<(usize, T)>>>,
    labels: Arc<Mutex<Vec<(&'static str, f64)>>>,
    clock: Arc<dyn Clock>,
    steals: Arc<AtomicU32>,
    panics: Arc<AtomicU32>,
}

impl<T> Share<T> {
    fn clone_handles(&self) -> Self {
        Self {
            remaining: Arc::clone(&self.remaining),
            stop: Arc::clone(&self.stop),
            entered: Arc::clone(&self.entered),
            written: Arc::clone(&self.written),
            labels: Arc::clone(&self.labels),
            clock: Arc::clone(&self.clock),
            steals: Arc::clone(&self.steals),
            panics: Arc::clone(&self.panics),
        }
    }
}

fn worker_loop<T: Send>(
    owner: &Arc<Deque<Unit<T>>>,
    all: &[Arc<Deque<Unit<T>>>],
    share: &Share<T>,
) {
    loop {
        if stopped(share) {
            break;
        }
        let unit = if let Some(unit) = owner.pop() {
            unit
        } else if let Some(unit) = steal_any(all, owner, &share.steals) {
            unit
        } else if stopped(share) {
            break;
        } else {
            thread::yield_now();
            continue;
        };
        let index = unit.index;
        let label = unit.label;
        share.entered.store(true, Ordering::Release);
        let (value, elapsed, panicked) = invoke(share.clock.as_ref(), unit.work);
        push_label(&share.labels, label, elapsed);
        if panicked {
            share.panics.fetch_add(1, Ordering::Relaxed);
        } else if let Some(value) = value {
            lock(&share.written).push((index, value));
        }
        let _ = share
            .remaining
            .try_update(Ordering::Release, Ordering::Relaxed, |left| {
                Some(left.saturating_sub(1))
            });
    }
}

fn stopped<T>(share: &Share<T>) -> bool {
    share.stop.load(Ordering::Acquire) || share.remaining.load(Ordering::Acquire) == 0
}

fn steal_any<T>(
    all: &[Arc<Deque<Unit<T>>>],
    owner: &Arc<Deque<Unit<T>>>,
    steals: &AtomicU32,
) -> Option<Unit<T>> {
    for deque in all {
        if Arc::ptr_eq(deque, owner) {
            continue;
        }
        match deque.steal() {
            Stolen::Task(unit) => {
                steals.fetch_add(1, Ordering::Relaxed);
                return Some(unit);
            }
            Stolen::Retry => match deque.steal() {
                Stolen::Task(unit) => {
                    steals.fetch_add(1, Ordering::Relaxed);
                    return Some(unit);
                }
                Stolen::Empty | Stolen::Retry => {}
            },
            Stolen::Empty => {}
        }
    }
    None
}

fn push_label(labels: &Mutex<Vec<(&'static str, f64)>>, label: &'static str, elapsed: f64) {
    lock(labels).push((label, elapsed));
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn sum_labels(pairs: &[(&'static str, f64)]) -> Vec<(&'static str, f64)> {
    let mut names: Vec<&'static str> = pairs.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let total = pairs
                .iter()
                .filter(|(label, _)| *label == name)
                .map(|(_, elapsed)| *elapsed)
                .sum();
            (name, total)
        })
        .collect()
}

/// FNV-1a over stage numbers and filled `u64` slots. `None` mixes a fixed salt.
#[must_use]
pub fn hash_slots(stages: &[StageSlots<u64>]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for stage in stages {
        hash = mix(hash, u64::from(stage.stage));
        for (index, slot) in stage.slots.iter().enumerate() {
            hash = mix(hash, u64::try_from(index).unwrap_or(0));
            hash = mix(
                hash,
                match slot {
                    Some(value) => *value,
                    None => 0xffff_ffff_ffff_ffff,
                },
            );
        }
    }
    hash
}

fn mix(hash: u64, value: u64) -> u64 {
    hash.wrapping_mul(0x0000_0100_0000_01b3) ^ value
}

#[cfg(all(test, not(feature = "loom")))]
mod spawn_tests {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use super::Scheduler;
    use crate::{Class, Job, JobSettings, ManualClock, Order, ScheduleError};

    #[test]
    fn a_failed_spawn_joins_the_workers_already_started() {
        let scheduler = Arc::new(Scheduler::new(
            JobSettings {
                workers: 4,
                order: Order::Parallel,
                frame_budget_ms: 6.0,
                slack: false,
            },
            Arc::new(ManualClock::new(0.0)),
        ));
        scheduler.spawn_fail_after.store(1, Ordering::Relaxed);
        scheduler.wait_for_entered.store(true, Ordering::Relaxed);
        let jobs = (0..4u32)
            .map(|index| Job {
                stage: 0,
                index,
                class: Class::Must,
                label: "move",
                access: Vec::new(),
                work: Box::new(move || {
                    let (_tx, hold) = mpsc::channel::<()>();
                    assert!(hold.recv_timeout(Duration::from_millis(400)).is_err());
                    u64::from(index)
                }),
            })
            .collect();
        let (tx, rx) = mpsc::channel();
        let runner = Arc::clone(&scheduler);
        thread::spawn(move || {
            let _ = tx.send(runner.run(jobs));
        });
        let err = rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, ScheduleError::Spawn));
        scheduler
            .spawn_fail_after
            .store(usize::MAX, Ordering::Relaxed);
        scheduler.wait_for_entered.store(false, Ordering::Relaxed);
        let report = scheduler
            .run(vec![
                Job {
                    stage: 0,
                    index: 0,
                    class: Class::Must,
                    label: "move",
                    access: Vec::new(),
                    work: Box::new(|| 1u64),
                },
                Job {
                    stage: 0,
                    index: 1,
                    class: Class::Must,
                    label: "move",
                    access: Vec::new(),
                    work: Box::new(|| 2u64),
                },
            ])
            .unwrap();
        assert_eq!(report.stages[0].slots, vec![Some(1), Some(2)]);
    }
}
