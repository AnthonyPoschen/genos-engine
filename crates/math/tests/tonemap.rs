//! CPU reference for the HDR exposure and display curves (ADR 0027).
//! Each test names the published value it checks, so a wrong constant fails here
//! before any shader is compared against it.

use genos_math::tonemap::{
    agx_curve, camera_ev100, ev100_from_luminance, exposure_from_ev100, exposure_with_compensation,
    karis_weight, nits_from_display, pq_st2084, step_exposure, GRAPHICS_WHITE_NITS, HDR_PEAK_NITS,
    MID_GREY,
};

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn ev100_zero_is_one_over_one_point_two() {
    assert!(close(exposure_from_ev100(0.0), 1.0 / 1.2, 1e-12));
    // One stop brighter halves the scale.
    assert!(close(exposure_from_ev100(1.0), 0.5 / 1.2, 1e-12));
}

#[test]
fn camera_f1_one_second_iso100_is_ev0() {
    assert!(close(camera_ev100(1.0, 1.0, 100.0), 0.0, 1e-12));
}

#[test]
fn sunny_sixteen_is_about_ev15() {
    // f/16, 1/100 s, ISO 100: log2(256 × 100) = 14.64.
    let ev = camera_ev100(16.0, 1.0 / 100.0, 100.0);
    assert!(close(ev, 25_600.0_f64.log2(), 1e-12));
    assert!(ev > 14.5 && ev < 15.0);
}

#[test]
fn luminance_meter_matches_k_12_5() {
    // 12.5 cd/m² × 100/12.5 = 100 → EV100 = log2(100).
    assert!(close(ev100_from_luminance(12.5), 100.0_f64.log2(), 1e-12));
    // Metering then exposing puts the average at 1/(1.2 × 8) = mid-grey-ish 0.104.
    let l = 1000.0;
    let lit = l * exposure_from_ev100(ev100_from_luminance(l));
    assert!(close(lit, 12.5 / (1.2 * 100.0), 1e-12));
}

#[test]
fn compensation_plus_one_doubles() {
    let base = exposure_with_compensation(5.0, 0.0);
    assert!(close(
        exposure_with_compensation(5.0, 1.0),
        2.0 * base,
        1e-15
    ));
}

#[test]
fn still_scene_holds_exposure() {
    let t = exposure_from_ev100(3.0);
    let mut s = t;
    for _ in 0..600 {
        s = step_exposure(s, t, 1.0 / 60.0, 0.4, 0.8);
    }
    assert!(close(s, t, 0.0));
}

#[test]
fn four_stop_adaptation_reaches_90_percent_in_window_without_overshoot() {
    let dt = 1.0 / 60.0;
    let start = exposure_from_ev100(0.0);
    let target = exposure_from_ev100(4.0);
    let mut s = start;
    let mut t90 = None;
    for frame in 1..=600 {
        s = step_exposure(s, target, dt, 0.4, 0.8);
        assert!(s >= target, "stepped past the target");
        let progress = (start - s) / (start - target);
        if t90.is_none() && progress >= 0.9 {
            t90 = Some(f64::from(frame) * dt);
        }
    }
    assert!(t90.is_some(), "never reached 90%");
    let t90 = t90.unwrap_or(f64::INFINITY);
    // ln(10) × 0.4 s = 0.92 s.
    assert!((0.4..=2.0).contains(&t90), "t90 {t90}");
    assert!(close(t90, 10.0_f64.ln() * 0.4, 2.0 * dt));
}

#[test]
fn karis_weight_tames_a_firefly() {
    assert!(close(karis_weight(0.0), 1.0, 0.0));
    // A 1000-nit firefly contributes Y/(1+Y) < 1 instead of 1000.
    let contrib = 1000.0 * karis_weight(1000.0);
    assert!(contrib < 1.0);
}

#[test]
fn pq_matches_st2084_table() {
    // Values from ITU-R BT.2100 / ST 2084 reference tables.
    assert!(close(
        pq_st2084(0.0),
        (3424.0_f64 / 4096.0).powf(2523.0 / 32.0),
        1e-15
    ));
    assert!(pq_st2084(0.0) < 1e-6);
    assert!(close(pq_st2084(10_000.0), 1.0, 1e-12));
    assert!(close(pq_st2084(100.0), 0.508_078, 1e-5));
    assert!(close(pq_st2084(203.0), 0.580_689, 1e-5));
    assert!(close(pq_st2084(1000.0), 0.751_827, 1e-5));
}

#[test]
fn pq_is_monotone() {
    let mut prev = pq_st2084(0.0);
    for i in 1..=10_000 {
        let v = pq_st2084(f64::from(i));
        assert!(v > prev);
        prev = v;
    }
}

#[test]
fn nit_map_pins_white_and_peak() {
    let white = 0.6;
    assert!(close(
        nits_from_display(white, white),
        GRAPHICS_WHITE_NITS,
        1e-9
    ));
    assert!(close(nits_from_display(1.0, white), HDR_PEAK_NITS, 1e-9));
    assert!(close(nits_from_display(5.0, white), HDR_PEAK_NITS, 1e-9));
    assert!(close(nits_from_display(0.0, white), 0.0, 0.0));
    assert!(close(nits_from_display(f64::NAN, white), 0.0, 0.0));
}

#[test]
fn agx_keeps_mid_grey() {
    assert!(close(agx_curve(MID_GREY), MID_GREY, 1e-12));
}

#[test]
fn agx_is_monotone_and_bounded_in_its_log_range() {
    let mut prev = 0.0;
    // -12 to +12 stops around mid grey in 1/16 stop steps.
    for i in -192..=192 {
        let x = MID_GREY * (f64::from(i) / 16.0).exp2();
        let y = agx_curve(x);
        assert!(y.is_finite());
        assert!(y >= prev, "not monotone at {x}");
        if i <= 104 {
            // Inside the log range (up to +6.5 stops) the code stays in 0..=1.
            assert!(y <= 1.0 + 1e-12, "over 1 at {x}");
        }
        prev = y;
    }
    // +6.5 stops, the top of the log range, is display white.
    assert!(close(agx_curve(MID_GREY * 6.5_f64.exp2()), 1.0, 1e-9));
    assert!(close(agx_curve(0.0), 0.0, 0.0));
    assert!(close(agx_curve(-1.0), 0.0, 0.0));
}

#[test]
fn agx_toe_lifts_less_than_linear_shoulder_compresses() {
    // Below grey, the curve is darker than identity; above, it compresses.
    assert!(agx_curve(0.02) < 0.02);
    assert!(agx_curve(2.0) < 2.0 && agx_curve(2.0) > MID_GREY);
}
