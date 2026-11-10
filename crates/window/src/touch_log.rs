//! The C touch log shared by the window backends.

use std::ffi::{c_int, c_void};

const BEGAN: c_int = 0;
const MOVED: c_int = 1;
const ENDED: c_int = 2;
const CAP: usize = 8;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Sample {
    id: u64,
    x: f32,
    y: f32,
    phase: c_int,
}

unsafe extern "C" {
    fn genos_touch_log_size() -> c_int;
    fn genos_touch_log_push(log: *mut c_void, id: u64, x: f32, y: f32, phase: c_int);
    fn genos_touch_log_copy(log: *mut c_void, out: *mut Sample) -> c_int;
}

fn fresh_log() -> Vec<u8> {
    let bytes = usize::try_from(unsafe { genos_touch_log_size() }).expect("touch log size");
    vec![0u8; bytes]
}

fn push(log: &mut [u8], id: u64, x: f32, y: f32, phase: c_int) {
    unsafe { genos_touch_log_push(log.as_mut_ptr().cast(), id, x, y, phase) };
}

fn copy_out(log: &mut [u8]) -> Vec<Sample> {
    let mut out = [Sample {
        id: 0,
        x: 0.0,
        y: 0.0,
        phase: 0,
    }; CAP];
    let count = unsafe { genos_touch_log_copy(log.as_mut_ptr().cast(), out.as_mut_ptr()) };
    let count = usize::try_from(count).expect("touch count");
    out.into_iter().take(count.min(CAP)).collect()
}

#[test]
fn a_full_touch_log_still_reports_every_lift() {
    let mut log = fresh_log();
    for id in 1..=8u64 {
        push(&mut log, id, 0.0, 0.0, BEGAN);
        push(&mut log, id, 10.0, 1.0, MOVED);
        push(&mut log, id, 20.0, 2.0, MOVED);
    }
    for id in 1..=8u64 {
        push(&mut log, id, 48.0, 3.0, ENDED);
    }
    let lifted = copy_out(&mut log);
    assert_eq!(
        lifted.len(),
        8,
        "eight lifts still fit after the log overflowed"
    );
    for id in 1..=8u64 {
        let sample = lifted
            .iter()
            .find(|sample| sample.id == id)
            .expect("finger was dropped");
        assert_eq!(sample.phase, ENDED, "finger {id} lifted");
        assert!(
            (sample.x - 48.0).abs() < 1.0e-4,
            "finger {id} kept the lift position"
        );
    }
    assert!(
        copy_out(&mut log).is_empty(),
        "a reported lift is not still down"
    );

    for id in 1..=8u64 {
        push(&mut log, id, 0.0, 0.0, BEGAN);
    }
    let _began = copy_out(&mut log);
    for id in 1..=8u64 {
        push(&mut log, id, 4.0, 4.0, MOVED);
    }
    push(&mut log, 9, 9.0, 9.0, ENDED);
    let first = copy_out(&mut log);
    let second = copy_out(&mut log);
    assert!(
        first
            .iter()
            .any(|sample| sample.id == 9 && sample.phase == ENDED),
        "a new lift evicts a move, got {first:?}"
    );
    for id in 1..=8u64 {
        let still_down = first
            .iter()
            .chain(second.iter())
            .any(|sample| sample.id == id && sample.phase == MOVED);
        assert!(
            still_down,
            "finger {id} stayed down after its move was evicted"
        );
        let false_lift = first
            .iter()
            .chain(second.iter())
            .any(|sample| sample.id == id && sample.phase == ENDED);
        assert!(!false_lift, "finger {id} was not a lift");
    }
}
