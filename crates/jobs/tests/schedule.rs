//! Scheduler behavior: order, the budget, conflicts, and a nested wait.

#![expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use genos_jobs::{
    conflict_batches, ensure_parallel, hash_slots, Access, AccessKey, Class, Job, JobSettings,
    ManualClock, Mode, Order, ScheduleError, Scheduler,
};

fn job(
    stage: u32,
    index: u32,
    class: Class,
    work: impl FnOnce() -> u64 + Send + 'static,
) -> Job<u64> {
    Job {
        stage,
        index,
        class,
        label: "move",
        access: Vec::new(),
        work: Box::new(work),
    }
}

fn settings(order: Order, workers: u32) -> JobSettings {
    JobSettings {
        workers,
        order,
        frame_budget_ms: 6.0,
        slack: true,
    }
}

#[test]
fn serial_order_is_stage_then_index() {
    let scheduler = Scheduler::new(settings(Order::Serial, 0), Arc::new(ManualClock::new(0.0)));
    let report = scheduler
        .run(vec![
            job(1, 0, Class::Must, || 10),
            job(0, 1, Class::Must, || 1),
            job(0, 0, Class::Must, || 0),
        ])
        .unwrap();
    assert_eq!(report.stages.len(), 2);
    assert_eq!(report.stages[0].stage, 0);
    assert_eq!(report.stages[0].slots, vec![Some(0), Some(1)]);
    assert_eq!(report.stages[1].slots, vec![Some(10)]);
}

#[test]
fn parallel_slots_match_serial_hash() {
    let serial = run_grid(Order::Serial, 0);
    let parallel = run_grid(Order::Parallel, 2);
    let again = run_grid(Order::Serial, 0);
    assert_eq!(serial, parallel);
    assert_eq!(serial, again);
    assert_ne!(serial, 0);
}

fn run_grid(order: Order, workers: u32) -> u64 {
    let scheduler = Scheduler::new(settings(order, workers), Arc::new(ManualClock::new(0.0)));
    let mut jobs = Vec::new();
    for stage in 0..2u32 {
        for index in 0..16u32 {
            jobs.push(job(stage, index, Class::Must, move || {
                u64::from(stage) * 100 + u64::from(index)
            }));
        }
    }
    jobs.reverse();
    let report = scheduler.run(jobs).unwrap();
    assert!(report
        .stages
        .iter()
        .all(|stage| stage.slots.iter().all(Option::is_some)));
    hash_slots(&report.stages)
}

#[test]
fn access_conflict_is_rejected() {
    let write = Access {
        key: AccessKey::named("health"),
        mode: Mode::Write,
    };
    let read = Access {
        key: AccessKey::named("health"),
        mode: Mode::Read,
    };
    let position = Access {
        key: AccessKey::named("position"),
        mode: Mode::Write,
    };
    assert!(matches!(
        ensure_parallel(&[vec![write], vec![write]]),
        Err(ScheduleError::Conflict { left: 0, right: 1 })
    ));
    assert!(ensure_parallel(&[vec![read], vec![read]]).is_ok());
    assert!(ensure_parallel(&[vec![read], vec![position]]).is_ok());
    assert_eq!(conflict_batches(&[vec![write], vec![write]]).len(), 2);
    assert_eq!(conflict_batches(&[vec![read], vec![read]]).len(), 1);
}

#[test]
fn conflicting_writes_stay_in_index_order() {
    let write = Access {
        key: AccessKey::named("health"),
        mode: Mode::Write,
    };
    for _ in 0..20 {
        let cell = Arc::new(AtomicU64::new(0));
        let first = Arc::clone(&cell);
        let second = Arc::clone(&cell);
        let scheduler = Scheduler::new(
            settings(Order::Parallel, 2),
            Arc::new(ManualClock::new(0.0)),
        );
        let mut earlier = job(0, 0, Class::Must, move || {
            first.store(1, Ordering::Relaxed);
            1
        });
        earlier.access = vec![write];
        let mut later = job(0, 1, Class::Must, move || second.load(Ordering::Relaxed));
        later.access = vec![write];
        let report = scheduler.run(vec![later, earlier]).unwrap();
        assert_eq!(report.stages[0].slots[1], Some(1));
    }
}

#[test]
fn soon_defers_when_must_meets_the_budget() {
    let (_scheduler, report) = budget_frame();
    assert!(report.stages[0].slots[2].is_none());
    assert_eq!(report.stats.deferred, 1);
}

#[test]
fn must_still_runs_and_counts_over_budget() {
    let (scheduler, report) = budget_frame();
    assert_eq!(report.stages[0].slots[0], Some(1));
    assert_eq!(report.stages[0].slots[1], Some(3));
    assert_eq!(report.stats.over_budget, 1);
    assert!(report.stats.must_wall_ms > 6.0);
    let counters = scheduler.counters();
    assert_eq!(
        counters.must_wall_ms.to_bits(),
        report.stats.must_wall_ms.to_bits()
    );
    assert_eq!(
        counters.soon_wall_ms.to_bits(),
        report.stats.soon_wall_ms.to_bits()
    );
    assert_eq!(
        counters.slack_wall_ms.to_bits(),
        report.stats.slack_wall_ms.to_bits()
    );
}

fn budget_frame() -> (genos_jobs::Scheduler, genos_jobs::Report<u64>) {
    let clock = Arc::new(ManualClock::new(0.0));
    let scheduler = Scheduler::new(settings(Order::Serial, 0), ManualClock::handle(&clock));
    let moved = Arc::clone(&clock);
    let report = scheduler
        .run(vec![
            job(0, 0, Class::Must, move || {
                moved.set(7.0);
                1
            }),
            job(0, 1, Class::Must, || 3),
            job(0, 2, Class::Soon, || 4),
        ])
        .unwrap();
    (scheduler, report)
}

#[test]
fn job_settings_round_trip() {
    let settings = JobSettings::low();
    let parsed = JobSettings::parse(&settings.to_text()).unwrap();
    assert_eq!(parsed.to_text(), settings.to_text());
    assert_eq!(parsed.workers, 3);
    assert_eq!(parsed.order, Order::Parallel);
    assert!(parsed.slack);
    let mut server = JobSettings::server();
    assert!(server.set_key("nope", "1").is_err());
    server.set_key("order", "parallel").unwrap();
    assert_eq!(server.order, Order::Parallel);
    assert!(server.set_key("frame_budget_ms", "-1").is_err());
}

#[test]
fn slack_does_not_run_inside_seat_enter() {
    genos_jobs::note_seat_enter();
    let fenced = Scheduler::new(settings(Order::Serial, 0), Arc::new(ManualClock::new(0.0)));
    let held_by_window = fenced.run(vec![job(0, 0, Class::Slack, || 5)]).unwrap();
    assert!(held_by_window.stages[0].slots[0].is_none());
    assert_eq!(held_by_window.stats.slack_during_fence, 0);
    genos_jobs::note_seat_leave();

    let scheduler = Scheduler::new(settings(Order::Serial, 0), Arc::new(ManualClock::new(0.0)));
    scheduler.suspend_slack();
    let held = scheduler.run(vec![job(0, 0, Class::Slack, || 5)]).unwrap();
    assert!(held.stages[0].slots[0].is_none());
    assert_eq!(held.stats.slack_during_fence, 0);
    scheduler.resume_slack();
    let open = scheduler.run(vec![job(0, 0, Class::Slack, || 5)]).unwrap();
    assert_eq!(open.stages[0].slots[0], Some(5));
}

#[test]
fn nested_wait_is_rejected() {
    let scheduler = Arc::new(Scheduler::serial());
    let nested = Arc::clone(&scheduler);
    let report = scheduler
        .run(vec![job(0, 0, Class::Must, move || {
            let err = nested.run(Vec::<Job<u64>>::new());
            u64::from(matches!(err, Err(ScheduleError::Rejected)))
        })])
        .unwrap();
    assert_eq!(report.stages[0].slots[0], Some(1));
    assert_eq!(report.stats.rejected, 1);
}

#[test]
fn pumped_is_rejected_and_a_duplicate_index_fails() {
    let scheduler = Scheduler::serial();
    let report = scheduler.run(vec![job(0, 0, Class::Pumped, || 1)]).unwrap();
    assert!(report.stages[0].slots[0].is_none());
    assert_eq!(report.stats.rejected, 1);
    let err = scheduler.run(vec![
        job(0, 0, Class::Must, || 1),
        job(0, 0, Class::Must, || 2),
    ]);
    assert!(matches!(
        err,
        Err(ScheduleError::DuplicateIndex { stage: 0, index: 0 })
    ));
}

#[expect(
    clippy::panic,
    reason = "the task panics so the scheduler must count it and return"
)]
#[test]
fn a_panic_is_counted_and_the_workers_return() {
    let scheduler = Scheduler::new(
        settings(Order::Parallel, 2),
        Arc::new(ManualClock::new(0.0)),
    );
    let err = scheduler.run(vec![
        job(0, 0, Class::Must, || 1),
        job(0, 1, Class::Must, || panic!("boom")),
        job(0, 2, Class::Must, || 3),
    ]);
    assert!(matches!(err, Err(ScheduleError::Panicked)));
    assert!(scheduler.counters().panics >= 1);
}

#[test]
fn soon_and_slack_follow_frame_elapsed_and_the_last_frame_stats() {
    let clock = Arc::new(ManualClock::new(100.0));
    let scheduler = Scheduler::new(settings(Order::Serial, 0), ManualClock::handle(&clock));
    let moved = Arc::clone(&clock);
    let first = scheduler
        .run(vec![
            job(0, 0, Class::Must, move || {
                moved.set(107.0);
                1
            }),
            job(0, 1, Class::Soon, || 4),
        ])
        .unwrap();
    assert!(first.stages[0].slots[1].is_none());
    assert_eq!(first.stats.deferred, 1);

    let second = scheduler
        .run(vec![
            job(0, 0, Class::Must, || 1),
            job(0, 1, Class::Soon, || 4),
            job(0, 2, Class::Slack, || 5),
        ])
        .unwrap();
    assert_eq!(second.stages[0].slots[1], Some(4));
    assert_eq!(second.stages[0].slots[2], Some(5));
    assert_eq!(second.stats.deferred, 0);
}

#[test]
fn slack_during_a_fence_is_counted_when_it_runs() {
    let scheduler = Scheduler::new(settings(Order::Serial, 0), Arc::new(ManualClock::new(0.0)));
    scheduler.plant("slack_in_fence").unwrap();
    scheduler.suspend_slack();
    let held = scheduler.run(vec![job(0, 0, Class::Slack, || 5)]).unwrap();
    assert_eq!(held.stages[0].slots[0], Some(5));
    assert_eq!(held.stats.slack_during_fence, 1);
}
