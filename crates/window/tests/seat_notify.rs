//! The outer seat edge notifies the scheduler. No compositor is required.

use std::sync::atomic::{AtomicU32, Ordering};

static ENTERS: AtomicU32 = AtomicU32::new(0);
static LEAVES: AtomicU32 = AtomicU32::new(0);

fn enter() {
    ENTERS.fetch_add(1, Ordering::Relaxed);
}

fn leave() {
    LEAVES.fetch_add(1, Ordering::Relaxed);
}

#[test]
fn seat_enter_notifies_once_per_outer_edge() {
    let enters = ENTERS.load(Ordering::Relaxed);
    let leaves = LEAVES.load(Ordering::Relaxed);
    genos_window::set_seat_notifier(enter, leave);
    genos_window::seat_enter();
    genos_window::seat_enter();
    genos_window::seat_leave();
    genos_window::seat_leave();
    assert_eq!(ENTERS.load(Ordering::Relaxed), enters + 1);
    assert_eq!(LEAVES.load(Ordering::Relaxed), leaves + 1);
}
