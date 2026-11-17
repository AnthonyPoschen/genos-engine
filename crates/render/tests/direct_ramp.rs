//! The one-percent direct test compares against the last marked sample, per
//! channel. A ramp of sub-percent steps has to add up. A dim channel on a
//! bright pixel has to be able to go out.

const EPS: f32 = 0.01;
const FLOOR: f32 = 1.0e-4;

/// `lighting_compose_direct_moved` for one held frame that is not the drift pass.
fn marks(curr: [f32; 3], prev: [f32; 3]) -> bool {
    if curr == prev {
        return false;
    }
    let mut peak = 0.0f32;
    for c in 0..3 {
        peak = peak.max(curr[c]).max(prev[c]);
    }
    let floor_rel = FLOOR * peak.max(FLOOR);
    for c in 0..3 {
        let scale = curr[c].max(prev[c]).max(floor_rel);
        if (curr[c] - prev[c]).abs() > EPS * scale {
            return true;
        }
    }
    false
}

#[test]
fn a_sub_percent_ramp_marks_once_it_passes_the_last_commit() {
    let mut value = 1.0f32;
    let committed = 1.0f32;
    let mut marked_at = None;
    for frame in 1..=12 {
        value *= 1.004;
        let sample = [value, value, value];
        let prior = [committed, committed, committed];
        if marks(sample, prior) {
            marked_at = Some(frame);
            break;
        }
    }
    let at = marked_at.expect("the ramp never marked");
    assert!(
        (3..=4).contains(&at),
        "0.4 % steps should cross 1 % of the committed sample on frame 3 or 4, marked at {at}"
    );

    // The old test wrote the tracking word before the compare, so each step
    // was measured against the step just stored and a 0.4 % ramp never marked.
    let mut previous = 1.0f32;
    let mut value = 1.0f32;
    for _ in 1..=12 {
        value *= 1.004;
        let sample = [value, value, value];
        let prior = [previous, previous, previous];
        assert!(
            !marks(sample, prior),
            "a single 0.4 % step must stay under the test"
        );
        previous = value;
    }
}

#[test]
fn a_dim_channel_on_a_bright_pixel_cannot_vanish() {
    let bright = [1.0, 0.002, 0.0];
    let dim_gone = [1.0, 0.0, 0.0];
    assert!(
        marks(dim_gone, bright),
        "the green channel dropped out and the test did not mark"
    );
    // Under the old peak scale the threshold was 1 % of 1.0, and 0.002 did not mark.
    let peak = 1.0f32;
    let old = (0.002f32).abs() > EPS * peak.max(FLOOR);
    assert!(!old, "the fixture no longer describes the old peak test");
}

#[test]
fn the_tracking_word_is_stored_after_the_compare() {
    let src = include_str!("../shaders/lighting_direct.comp");
    let moved = src
        .find("lighting_compose_direct_moved(packed, prev_d)")
        .expect("the compare");
    let store = src[moved..]
        .find("uintBitsToFloat(packed)")
        .expect("the tracking store");
    assert!(store > 0, "the packed sample is stored before the compare");
    let before = &src[..moved];
    assert!(
        !before.contains("] = uintBitsToFloat(packed);"),
        "the tracking word is still written before the one-percent test"
    );
}
