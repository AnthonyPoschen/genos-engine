//! Sine, cosine, and exponential for the mix.
//!
//! The polynomials are the same sequence of IEEE multiplies and adds on every
//! platform. The mix does not call the platform math library, and it does not
//! use a fused multiply-add.

/// Sine of `radians`. Accurate on the pan range `[0, π/2]`.
pub(crate) fn sin_f(radians: f32) -> f32 {
    if !radians.is_finite() {
        return 0.0;
    }
    let reduced = wrap_pi(radians);
    let (magnitude, negate) = fold_quarter(reduced);
    let value = sin_quarter(magnitude);
    if negate {
        -value
    } else {
        value
    }
}

/// Cosine of `radians`. Accurate on the pan range `[0, π/2]`.
pub(crate) fn cos_f(radians: f32) -> f32 {
    if !radians.is_finite() {
        return 1.0;
    }
    sin_f(radians + HALF_PI)
}

/// `e` raised to `value`, clamped so a thick barrier underflows to silence.
pub(crate) fn exp_f(value: f32) -> f32 {
    if !value.is_finite() {
        return if value.is_sign_negative() {
            0.0
        } else {
            f32::MAX
        };
    }
    if value < -80.0 {
        return 0.0;
    }
    if value > 80.0 {
        return f32::MAX;
    }
    let steps = nearest(value * INV_LN2);
    let reduced = value - steps * LN2;
    let square = reduced * reduced;
    let series = 1.0
        + reduced
        + square * (0.5 + reduced * (1.0 / 6.0 + reduced * (1.0 / 24.0 + reduced * (1.0 / 120.0))));
    scale_pow2(series, trunc_i32(steps))
}

const PI: f32 = std::f32::consts::PI;
const HALF_PI: f32 = PI * 0.5;
const LN2: f32 = std::f32::consts::LN_2;
const INV_LN2: f32 = std::f32::consts::LOG2_E;

fn wrap_pi(radians: f32) -> f32 {
    let turns = nearest(radians * (1.0 / (2.0 * PI)));
    let wrapped = radians - turns * (2.0 * PI);
    if wrapped > PI {
        wrapped - 2.0 * PI
    } else if wrapped < -PI {
        wrapped + 2.0 * PI
    } else {
        wrapped
    }
}

fn fold_quarter(radians: f32) -> (f32, bool) {
    let negative = radians < 0.0;
    let magnitude = if negative { -radians } else { radians };
    if magnitude <= HALF_PI {
        return (magnitude, negative);
    }
    (PI - magnitude, negative)
}

fn sin_quarter(radians: f32) -> f32 {
    if radians > HALF_PI * 0.5 {
        return cos_series(HALF_PI - radians);
    }
    sin_series(radians)
}

fn sin_series(radians: f32) -> f32 {
    let square = radians * radians;
    let cube = square * radians;
    radians - cube * (1.0 / 6.0 - square * (1.0 / 120.0 - square * (1.0 / 5040.0)))
}

fn cos_series(radians: f32) -> f32 {
    let square = radians * radians;
    1.0 - square * (0.5 - square * (1.0 / 24.0 - square * (1.0 / 720.0)))
}

fn scale_pow2(value: f32, power: i32) -> f32 {
    if value == 0.0 || power == 0 {
        return value;
    }
    let bits = value.to_bits();
    let biased = (bits >> 23) & 0xff;
    if biased == 0 || biased == 255 {
        return value;
    }
    let next = i32_from_bits(biased).saturating_add(power);
    if next <= 0 {
        return 0.0;
    }
    if next >= 255 {
        return f32::from_bits((bits & 0x8000_0000) | 0x7f7f_ffff);
    }
    let exponent = u32_from_i32(next) << 23;
    f32::from_bits((bits & 0x807f_ffff) | exponent)
}

fn nearest(value: f32) -> f32 {
    if value >= 0.0 {
        floor_f(value + 0.5)
    } else {
        ceil_f(value - 0.5)
    }
}

fn floor_f(value: f32) -> f32 {
    let truncated = trunc_f(value);
    if value < truncated {
        truncated - 1.0
    } else {
        truncated
    }
}

fn ceil_f(value: f32) -> f32 {
    let truncated = trunc_f(value);
    if value > truncated {
        truncated + 1.0
    } else {
        truncated
    }
}

fn trunc_f(value: f32) -> f32 {
    i32_to_f32(trunc_i32(value))
}

fn trunc_i32(value: f32) -> i32 {
    let bits = value.to_bits();
    let negative = bits & 0x8000_0000 != 0;
    let biased = (bits >> 23) & 0xff;
    if biased < 127 {
        return 0;
    }
    let exponent = biased - 127;
    if exponent >= 31 {
        return if negative { i32::MIN } else { i32::MAX };
    }
    let significand = (bits & 0x007f_ffff) | 0x0080_0000;
    let magnitude = if exponent >= 23 {
        significand << (exponent - 23)
    } else {
        significand >> (23 - exponent)
    };
    i32_from_magnitude(magnitude, negative)
}

fn i32_to_f32(value: i32) -> f32 {
    if value == 0 {
        return 0.0;
    }
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    let top = magnitude.ilog2();
    let mantissa = if top >= 23 {
        magnitude >> (top - 23)
    } else {
        magnitude << (23 - top)
    };
    let fraction = mantissa & 0x007f_ffff;
    let exponent = (top + 127) << 23;
    let mut bits = fraction | exponent;
    if negative {
        bits |= 0x8000_0000;
    }
    f32::from_bits(bits)
}

fn i32_from_magnitude(magnitude: u32, negative: bool) -> i32 {
    let bits = if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    };
    i32::from_ne_bytes(bits.to_ne_bytes())
}

fn i32_from_bits(bits: u32) -> i32 {
    i32::from_ne_bytes(bits.to_ne_bytes())
}

fn u32_from_i32(value: i32) -> u32 {
    u32::from_ne_bytes(value.to_ne_bytes())
}

#[cfg(test)]
mod tests {
    use super::{cos_f, exp_f, sin_f, HALF_PI};

    #[test]
    fn identities_hold_on_the_pan_and_barrier_range() {
        assert_eq!(sin_f(0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(cos_f(0.0).to_bits(), 1.0f32.to_bits());
        assert_eq!(exp_f(0.0).to_bits(), 1.0f32.to_bits());
        assert!(sin_f(HALF_PI) > 0.999);
        assert!(sin_f(HALF_PI) < 1.001);
        assert!(cos_f(HALF_PI).abs() < 0.001);
        assert!(exp_f(1.0) > 2.7 && exp_f(1.0) < 2.72);
        assert!(exp_f(-1.0) > 0.36 && exp_f(-1.0) < 0.38);
        assert_eq!(exp_f(-100.0), 0.0);
    }
}
