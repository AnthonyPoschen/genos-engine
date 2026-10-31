//! Exposure and display curves for the HDR pipeline (ADR 0015, ADR 0027).
//!
//! These are the closed forms the post passes must match. They are `f64` and
//! need no GPU, so `cargo test -p genos-math` checks them headless.
//! `crates/render/src/hdr_output.rs` holds the same formulas today; ADR 0027
//! moves its scalar curves to call these.

/// Graphics white on both HDR paths, in nits (ITU-R BT.2408).
pub const GRAPHICS_WHITE_NITS: f64 = 203.0;
/// Fixed HDR10 peak until the display's own peak is read.
pub const HDR_PEAK_NITS: f64 = 1000.0;
/// scRGB code 1.0 is 80 nits.
pub const SCRGB_NITS_PER_CODE: f64 = 80.0;
/// Scene mid grey that the `AgX` curve keeps fixed.
pub const MID_GREY: f64 = 0.18;

const LOG_MIN: f64 = -10.0;
const LOG_MAX: f64 = 6.5;
const SIGMOID_SLOPE: f64 = 2.4;
const SIGMOID_POWER: f64 = 1.5;

/// EV100 of an average scene luminance in cd/m², with the reflected-meter constant 12.5.
#[must_use]
pub fn ev100_from_luminance(luminance: f64) -> f64 {
    (luminance * (100.0 / 12.5)).log2()
}

/// Exposure scale `1 / (1.2 × 2^EV100)`. 1.2 is the saturation-based sensor margin (78/65).
#[must_use]
pub fn exposure_from_ev100(ev100: f64) -> f64 {
    1.0 / (1.2 * ev100.exp2())
}

/// EV100 from a physical camera: aperture N, shutter t seconds, ISO S.
#[must_use]
pub fn camera_ev100(aperture: f64, shutter: f64, iso: f64) -> f64 {
    ((aperture * aperture) / shutter * 100.0 / iso).log2()
}

/// Exposure after a compensation in stops. `+1` doubles the scale.
#[must_use]
pub fn exposure_with_compensation(ev100: f64, compensation: f64) -> f64 {
    exposure_from_ev100(ev100 - compensation)
}

/// One exponential adaptation step toward `target`. Holds when the relative
/// gap is under 0.001, so a still scene never drifts. `tau_down` applies when
/// the scene gets brighter (scale falling). It never steps past the target.
#[must_use]
pub fn step_exposure(scale: f64, target: f64, dt: f64, tau_down: f64, tau_up: f64) -> f64 {
    if !target.is_finite() || target < 0.0 {
        return scale;
    }
    if !scale.is_finite() || scale <= 0.0 {
        return target;
    }
    let rel = (target - scale).abs() / scale.abs().max(1.0e-6);
    if rel < 0.001 {
        return scale;
    }
    let tau = if target < scale { tau_down } else { tau_up };
    let tau = if tau > 0.0 { tau } else { dt.abs().max(1.0e-6) };
    let gain = 1.0 - (-dt.abs() / tau).exp();
    scale + (target - scale) * gain
}

/// Karis weight `1 / (1 + Y)` for the first bloom downsample.
#[must_use]
pub fn karis_weight(luma: f64) -> f64 {
    1.0 / (1.0 + luma.max(0.0))
}

/// SMPTE ST 2084 inverse EOTF. Nits in, code in 0..1 out.
#[must_use]
pub fn pq_st2084(nits: f64) -> f64 {
    let c1 = 3424.0 / 4096.0;
    let c2 = 2413.0 / 128.0;
    let c3 = 2392.0 / 128.0;
    let m1 = 2610.0 / 16384.0;
    let m2 = 2523.0 / 32.0;
    let y = (nits / 10_000.0).max(0.0);
    let y_m = if y == 0.0 { 0.0 } else { y.powf(m1) };
    ((c1 + c2 * y_m) / (1.0 + c3 * y_m)).powf(m2)
}

/// Display-linear value to nits. Up to `white` maps linearly to 203 nits,
/// above it linearly to [`HDR_PEAK_NITS`] at 1.
#[must_use]
pub fn nits_from_display(display: f64, white: f64) -> f64 {
    if !display.is_finite() || display <= 0.0 {
        return 0.0;
    }
    let white = white.clamp(1.0e-4, 0.999);
    let d = display.min(1.0);
    if d <= white {
        d / white * GRAPHICS_WHITE_NITS
    } else {
        let t = (d - white) / (1.0 - white);
        GRAPHICS_WHITE_NITS + t * (HDR_PEAK_NITS - GRAPHICS_WHITE_NITS)
    }
}

/// The per-channel `AgX` tone curve: log2 encode around mid grey over
/// [-10, +6.5] stops, the Sobotka sigmoid, then power 2.4 back to display
/// linear. The full transform adds the inset/outset matrices and hue restore
/// around this curve (`agx_rec709` in the renderer).
#[must_use]
pub fn agx_curve(scene_linear: f64) -> f64 {
    if scene_linear.is_nan() || scene_linear <= 0.0 {
        return 0.0;
    }
    let logged = ((scene_linear / MID_GREY).log2() - LOG_MIN) / (LOG_MAX - LOG_MIN);
    let curved = sigmoid(logged);
    if curved <= 0.0 {
        0.0
    } else {
        curved.powf(2.4)
    }
}

fn sigmoid(x: f64) -> f64 {
    let x_pivot = (-LOG_MIN) / (LOG_MAX - LOG_MIN);
    let y_pivot = MID_GREY.powf(1.0 / SIGMOID_SLOPE);
    let scale_toe = -scale_factor(1.0, 1.0, 1.0 - x_pivot, 1.0 - y_pivot);
    let scale_shoulder = scale_factor(1.0, 1.0, x_pivot, y_pivot);
    if x < x_pivot {
        exponential_curve(x, scale_toe, x_pivot, y_pivot)
    } else {
        exponential_curve(x, scale_shoulder, x_pivot, y_pivot)
    }
}

fn scale_factor(limit_x: f64, limit_y: f64, transition_x: f64, transition_y: f64) -> f64 {
    let span_x = limit_x - transition_x;
    let span_y = limit_y - transition_y;
    let term_a = (SIGMOID_SLOPE * span_x).powf(-SIGMOID_POWER);
    let term_b = (SIGMOID_SLOPE * span_x / span_y).powf(SIGMOID_POWER) - 1.0;
    (term_a * term_b).powf(-1.0 / SIGMOID_POWER)
}

fn exponential_curve(x: f64, scale: f64, transition_x: f64, transition_y: f64) -> f64 {
    let inner = SIGMOID_SLOPE * (x - transition_x) / scale;
    let signed = if inner < 0.0 {
        -(-inner).powf(SIGMOID_POWER)
    } else {
        inner.powf(SIGMOID_POWER)
    };
    scale * (inner / (1.0 + signed).powf(1.0 / SIGMOID_POWER)) + transition_y
}
