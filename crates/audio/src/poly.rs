//! Cephes single-precision `sinf`, `cosf`, and `expf` for the mix.
//!
//! Range reduction and the polynomial coefficients are the Cephes release
//! (Moshier, public domain). Each step is a separate multiply or add. The mix
//! does not call the platform math library and does not use a fused multiply-add.

/// Sine of `radians`. Pan uses arguments in `[0, π/2]`.
pub(crate) fn sin_f(radians: f32) -> f32 {
    if !radians.is_finite() {
        return 0.0;
    }
    let negate = radians.is_sign_negative();
    let x = radians.abs();
    let (reduced, octant, flip) = reduce_octant(x);
    let sine = octant != 1 && octant != 2;
    circular(reduced, sine, negate != flip)
}

/// Cosine of `radians`. `cos(π/2)` is `+0`.
pub(crate) fn cos_f(radians: f32) -> f32 {
    if radians.abs().to_bits() == core::f32::consts::FRAC_PI_2.to_bits() {
        return 0.0;
    }
    if !radians.is_finite() {
        return 1.0;
    }
    let x = radians.abs();
    let (reduced, octant, mut negate) = reduce_octant(x);
    if octant > 1 {
        negate = !negate;
    }
    let sine = octant == 1 || octant == 2;
    let value = circular(reduced, sine, negate);
    if value.to_bits() == 0x8000_0000 {
        0.0
    } else {
        value
    }
}

/// `e` raised to `value`. A thick barrier underflows to silence.
pub(crate) fn exp_f(value: f32) -> f32 {
    if !value.is_finite() {
        return if value.is_sign_negative() {
            0.0
        } else {
            f32::MAX
        };
    }
    if value > MAX_LOG {
        return f32::MAX;
    }
    if value < MIN_LOG {
        return 0.0;
    }
    let power = floor_f(LOG2_E * value + 0.5);
    let mut reduced = value;
    reduced -= power * LN2_HI;
    reduced -= power * LN2_LO;
    let square = reduced * reduced;
    let series = (((((1.987_569_1e-4 * reduced + 1.398_199_9e-3) * reduced + 8.333_452e-3)
        * reduced
        + 4.166_579_6e-2)
        * reduced
        + 1.666_666_6e-1)
        * reduced
        + 0.5)
        * square
        + reduced
        + 1.0;
    scale_pow2(series, trunc_i32(power))
}

const FOUR_OVER_PI: f32 = 1.273_239_5;
const PI_OVER_4: f32 = core::f32::consts::FRAC_PI_4;
const PI_OVER_4_HI: f32 = 0.785_156_25;
const PI_OVER_4_MID: f32 = 2.418_756_5e-4;
const PI_OVER_4_LO: f32 = 3.774_895e-8;
const LOSS_THRESHOLD: f32 = 8192.0;
const TOTAL_LOSS: f32 = 16_777_215.0;
const LOG2_E: f32 = core::f32::consts::LOG2_E;
const LN2_HI: f32 = 0.693_359_4;
const LN2_LO: f32 = -2.121_944_4e-4;
const MAX_LOG: f32 = 88.722_84;
const MIN_LOG: f32 = -103.278_93;
const SIN_C0: f32 = -1.951_529_6e-4;
const SIN_C1: f32 = 8.332_161e-3;
const SIN_C2: f32 = -1.666_665_5e-1;
const COS_C0: f32 = 2.443_315_7e-5;
const COS_C1: f32 = -1.388_731_6e-3;
const COS_C2: f32 = 4.166_664_6e-2;

fn reduce_octant(mut magnitude: f32) -> (f32, u32, bool) {
    if magnitude > TOTAL_LOSS {
        return (0.0, 0, false);
    }
    let mut octant = trunc_u32(FOUR_OVER_PI * magnitude);
    let mut steps = f32_from_u32(octant);
    if octant & 1 != 0 {
        octant = octant.saturating_add(1);
        steps += 1.0;
    }
    octant &= 7;
    let mut negate = false;
    if octant > 3 {
        negate = true;
        octant -= 4;
    }
    if magnitude > LOSS_THRESHOLD {
        magnitude -= steps * PI_OVER_4;
    } else {
        magnitude =
            ((magnitude - steps * PI_OVER_4_HI) - steps * PI_OVER_4_MID) - steps * PI_OVER_4_LO;
    }
    (magnitude, octant, negate)
}

fn circular(reduced: f32, sine: bool, negate: bool) -> f32 {
    let square = reduced * reduced;
    let value = if sine {
        let mut poly = SIN_C0;
        poly = poly * square + SIN_C1;
        poly = poly * square + SIN_C2;
        poly * square * reduced + reduced
    } else {
        let mut poly = COS_C0;
        poly = poly * square + COS_C1;
        poly = poly * square + COS_C2;
        poly *= square * square;
        poly -= 0.5 * square;
        poly + 1.0
    };
    if negate {
        -value
    } else {
        value
    }
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

fn floor_f(value: f32) -> f32 {
    let truncated = trunc_f(value);
    if value < truncated {
        truncated - 1.0
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

fn trunc_u32(value: f32) -> u32 {
    if value <= 0.0 {
        return 0;
    }
    let bits = value.to_bits();
    let biased = (bits >> 23) & 0xff;
    if biased < 127 {
        return 0;
    }
    let exponent = biased - 127;
    if exponent >= 31 {
        return u32::MAX;
    }
    let significand = (bits & 0x007f_ffff) | 0x0080_0000;
    if exponent >= 23 {
        significand << (exponent - 23)
    } else {
        significand >> (23 - exponent)
    }
}

fn f32_from_u32(value: u32) -> f32 {
    if value == 0 {
        return 0.0;
    }
    let top = value.ilog2();
    let mantissa = if top >= 23 {
        value >> (top - 23)
    } else {
        value << (23 - top)
    };
    let fraction = mantissa & 0x007f_ffff;
    let exponent = (top + 127) << 23;
    f32::from_bits(fraction | exponent)
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
    use super::{cos_f, exp_f, sin_f};

    #[test]
    fn anchors_are_exact_and_the_right_angle_is_positive_zero() {
        assert_eq!(sin_f(0.0).to_bits(), 0);
        assert_eq!(cos_f(0.0).to_bits(), 1.0f32.to_bits());
        assert_eq!(exp_f(0.0).to_bits(), 1.0f32.to_bits());
        assert_eq!(cos_f(core::f32::consts::FRAC_PI_2).to_bits(), 0);
        assert_eq!(exp_f(-100.0), 0.0);
    }

    #[test]
    fn cephes_error_stays_inside_the_measured_bound() {
        let mut sin_abs = 0.0f64;
        let mut cos_abs = 0.0f64;
        let mut cursor = 0.001f32;
        while cursor < core::f32::consts::FRAC_PI_2 {
            let wide = f64::from(cursor);
            sin_abs = sin_abs.max((f64::from(sin_f(cursor)) - wide.sin()).abs());
            cos_abs = cos_abs.max((f64::from(cos_f(cursor)) - wide.cos()).abs());
            cursor += 0.001;
        }
        let mut exp_rel = 0.0f64;
        let mut argument = -80.0f32;
        while argument < 0.0 {
            let wide = f64::from(argument).exp();
            let got = f64::from(exp_f(argument));
            exp_rel = exp_rel.max((got - wide).abs() / wide);
            argument += 0.05;
        }
        assert!(
            sin_abs <= 6.0e-8,
            "sine absolute error {sin_abs} at a non-trivial angle"
        );
        assert!(
            cos_abs <= 6.0e-8,
            "cosine absolute error {cos_abs} at a non-trivial angle"
        );
        assert!(
            exp_rel <= 8.0e-8,
            "exp relative error {exp_rel} on a reduced argument"
        );
        let reduced = exp_f(-4.0);
        assert!(reduced > 0.018 && reduced < 0.019, "exp(-4) = {reduced}");
    }
}
