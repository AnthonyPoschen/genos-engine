//! Fiber-free work-stealing scheduler.
//!
//! A stage is a list of jobs with a class (Must, Soon, Slack, Pumped), an
//! index, and declared reads and writes. Conflicting jobs run in later
//! batches. Serial mode is the oracle: parallel mode writes the same slots.
//! A job that calls [`Scheduler::run`] is rejected. Workers are joined before
//! `run` returns.
//!
//! Networking: jobs do not replicate. The server runs [`Order::Serial`] with
//! zero workers so a headless tick matches the client oracle. Authority,
//! prediction, and rollback stay with the scene and the future replication
//! layer. This crate only orders the work.

#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

mod deque;
mod pool;
mod schedule;
mod settings;

pub use pool::{
    hash_slots, in_task, note_seat_enter, note_seat_leave, Clock, InstantClock, Job, ManualClock,
    Report, Scheduler, StageSlots, Stats,
};
pub use schedule::{
    class_rank, conflict_batches, conflicts, ensure_parallel, Access, AccessKey, Mode,
    ScheduleError,
};
pub use settings::JobSettings;

/// Where a job sits in the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Runs. Crossing the budget is counted, not a reason to stop.
    Must,
    /// Runs only while this frame's elapsed time is under the budget.
    Soon,
    /// Runs only with no seat held, Slack enabled, and time left in the frame.
    Slack,
    /// Never runs. Counted as rejected.
    Pumped,
}

/// Serial is the server and the replay. Parallel must hash-equal serial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// One thread, index order.
    Serial,
    /// Conflict-free batches on worker threads. The caller helps.
    Parallel,
}
