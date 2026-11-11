//! Integer deposits. One resolve is the only writer of the shown mean.

use genos_math::Vec3;

/// Injected faults the tests require to fail. Each one is independent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "each fault is a separate injection")]
pub struct Faults {
    pub gate1_off: bool,
    pub t_min_off: bool,
    pub count_shared_as_own: bool,
    pub unclamped: bool,
    pub uniform_pick: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Deposit {
    pub(crate) rgb: [u64; 3],
    pub(crate) weight: u64,
    pub own: u32,
    pub shared: u32,
}

const SCALE: f64 = 1_000_000.0;

pub fn add_own(dep: &mut Deposit, radiance: Vec3) {
    add_rgb(dep, radiance, 1.0);
    dep.own = dep.own.saturating_add(1);
}

pub fn add_shared(dep: &mut Deposit, radiance: Vec3, weight: f32, faults: Faults) {
    add_rgb(dep, radiance, weight);
    dep.shared = dep.shared.saturating_add(1);
    if faults.count_shared_as_own {
        dep.own = dep.own.saturating_add(1);
    }
}

fn add_rgb(dep: &mut Deposit, radiance: Vec3, weight: f32) {
    let w = quant(weight);
    dep.rgb[0] = dep.rgb[0].saturating_add(quant(radiance.x * weight));
    dep.rgb[1] = dep.rgb[1].saturating_add(quant(radiance.y * weight));
    dep.rgb[2] = dep.rgb[2].saturating_add(quant(radiance.z * weight));
    dep.weight = dep.weight.saturating_add(w);
}

fn quant(v: f32) -> u64 {
    if !v.is_finite() || v <= 0.0 {
        return 0;
    }
    whole_u64(f64::from(v) * SCALE)
}

fn whole_u64(v: f64) -> u64 {
    if !v.is_finite() || v <= 0.0 {
        return 0;
    }
    if v >= f64::from(u32::MAX) {
        return u64::from(u32::MAX);
    }
    let bits = v.to_bits();
    let exp = i32::from(u16::try_from((bits >> 52) & 0x7ff).unwrap_or(0)) - 1023;
    if exp < 0 {
        return 0;
    }
    let frac = (bits & ((1_u64 << 52) - 1)) | (1_u64 << 52);
    let shift = 52 - exp;
    if shift <= 0 {
        return u64::from(u32::MAX);
    }
    frac >> u32::try_from(shift).unwrap_or(52)
}

#[must_use]
pub fn mean_rgb(dep: &Deposit) -> Vec3 {
    if dep.weight == 0 {
        return Vec3::ZERO;
    }
    Vec3::new(
        channel(dep.rgb[0], dep.weight),
        channel(dep.rgb[1], dep.weight),
        channel(dep.rgb[2], dep.weight),
    )
}

#[must_use]
pub fn mean_luma(dep: &Deposit) -> f32 {
    let rgb = mean_rgb(dep);
    0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z
}

fn channel(sum: u64, weight: u64) -> f32 {
    let value = f64_from_u64(sum) / f64_from_u64(weight);
    f32_from_f64(value)
}

fn f64_from_u64(v: u64) -> f64 {
    let mut out = 0.0_f64;
    let mut bit = 1.0_f64;
    let mut n = v;
    while n > 0 {
        if n & 1 == 1 {
            out += bit;
        }
        bit *= 2.0;
        n >>= 1;
    }
    out
}

fn f32_from_f64(v: f64) -> f32 {
    if !v.is_finite() || v <= 0.0 {
        return 0.0;
    }
    if v >= f64::from(u16::MAX) {
        return f32::from(u16::MAX);
    }
    let bits = v.to_bits();
    let exp = i32::from(u16::try_from((bits >> 52) & 0x7ff).unwrap_or(0)) - 1023;
    if exp < -40 {
        return 0.0;
    }
    let whole = whole_u64(v);
    let frac_scale = 1024.0_f64;
    let frac = whole_u64((v - f64_from_u64(whole)) * frac_scale);
    let base = f32::from(u16::try_from(whole).unwrap_or(u16::MAX));
    let frac_f = f32::from(u16::try_from(frac).unwrap_or(0)) / 1024.0;
    base + frac_f
}

/// A record built only from shared samples did not raise `own`.
#[must_use]
pub fn own_excludes_shared(dep: &Deposit) -> bool {
    dep.shared == 0 || dep.own == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shared_sample_does_not_raise_own_unless_the_fault_is_on() {
        let mut clean = Deposit::default();
        add_shared(&mut clean, Vec3::new(1.0, 1.0, 1.0), 1.0, Faults::default());
        assert!(own_excludes_shared(&clean));
        assert_eq!(clean.own, 0);
        assert_eq!(clean.shared, 1);
        let mut broken = Deposit::default();
        add_shared(
            &mut broken,
            Vec3::new(1.0, 1.0, 1.0),
            1.0,
            Faults {
                count_shared_as_own: true,
                ..Faults::default()
            },
        );
        assert!(!own_excludes_shared(&broken));
    }

    #[test]
    fn two_orders_deposit_the_same_sum() {
        let sample = Vec3::new(0.25, 0.5, 0.125);
        let mut a = Deposit::default();
        let mut b = Deposit::default();
        add_own(&mut a, sample);
        add_own(&mut a, sample * 2.0);
        add_own(&mut b, sample * 2.0);
        add_own(&mut b, sample);
        assert_eq!(a, b);
    }
}
