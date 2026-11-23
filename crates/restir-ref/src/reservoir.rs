//! Reservoir and the unbiased balance-heuristic merge.
//!
//! Two reservoirs use the same threshold as `genos_render::lamp_pick::pairwise_merge`
//! (Bitterli et al. 2022 pairwise MIS, Jacobian 1). More than two use the balance
//! heuristic over the whole set: one prefix-sum pick, then
//! `W = area * p̂_src / (p̂_q * Σ M_i p̂_i(y))`.
//! That is GRIS equation 22, not Bitterli et al. 2020 Algorithm 6.
//! Section 3 divides by the sum of `M` and ignores the source target.

use crate::rng::Rng;

/// Empty lamp. Matches `LAMP_EMPTY` in `genos_render::lamp_pick`.
pub const LAMP_EMPTY: u32 = u32::MAX;

/// One shaded point's selected lamp.
///
/// `target` is the unshadowed luminance of `lamp` at the receiver that
/// finalized this reservoir. `W = w_sum / (m * target)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reservoir {
    pub lamp: u32,
    pub w_sum: f32,
    pub m: u32,
    pub target: f32,
}

impl Reservoir {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            lamp: LAMP_EMPTY,
            w_sum: 0.0,
            m: 0,
            target: 0.0,
        }
    }

    /// `p̂ * W`, the unshadowed luminance this reservoir stands for.
    #[must_use]
    pub fn estimate(self) -> f32 {
        if self.m == 0 {
            0.0
        } else {
            self.w_sum / m_as_f32(self.m)
        }
    }

    /// True when a lamp was selected.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.m == 0 || self.lamp == LAMP_EMPTY
    }
}

/// `W = w_sum / (M * target)`. Zero when the reservoir is empty.
#[expect(
    clippy::module_name_repetitions,
    reason = "reservoir_w is the W the shade multiplies, named beside Reservoir"
)]
#[must_use]
pub fn reservoir_w(reservoir: Reservoir) -> f32 {
    if reservoir.is_empty() || reservoir.target <= 0.0 {
        0.0
    } else {
        reservoir.w_sum / (m_as_f32(reservoir.m) * reservoir.target)
    }
}

/// Exact integer-to-`f32` for every `m` the reference stores (below 2^24).
#[must_use]
pub fn m_as_f32(value: u32) -> f32 {
    let hi = f32::from(u16::try_from(value >> 16).unwrap_or(0));
    let lo = f32::from(u16::try_from(value & 0xffff).unwrap_or(0));
    hi.mul_add(65536.0, lo)
}

/// Round to nearest even, the IEEE `f32` store.
///
/// A new `as f32` is a lossy cast. This is the same rounding, in integer shifts.
#[must_use]
pub fn f32_from_f64(value: f64) -> f32 {
    let bits = value.to_bits();
    let sign = if bits >> 63 == 0 { 0u32 } else { 1u32 << 31 };
    let exp = (bits >> 52) & 0x7ff;
    let frac = bits & 0x000f_ffff_ffff_ffff;
    if exp == 0 {
        return f32::from_bits(sign);
    }
    if exp == 0x7ff {
        let payload = if frac == 0 { 0 } else { 0x0040_0000 };
        return f32::from_bits(sign | 0x7f80_0000 | payload);
    }
    let exp_i = i64::try_from(exp).unwrap_or(0);
    let f32_exp = exp_i - 1023 + 127;
    if f32_exp >= 0xff {
        return f32::from_bits(sign | 0x7f80_0000);
    }
    if f32_exp > 0 {
        return narrow_normal(sign, f32_exp, frac);
    }
    narrow_subnormal(sign, 1 - f32_exp, frac)
}

fn narrow_normal(sign: u32, f32_exp: i64, frac: u64) -> f32 {
    let mut mant = frac >> 29;
    let rest = frac & 0x1fff_ffff;
    let half = 0x1000_0000;
    let odd = (mant & 1) == 1;
    if rest > half || (rest == half && odd) {
        mant += 1;
    }
    if mant >> 23 != 0 {
        let bumped = f32_exp + 1;
        if bumped >= 0xff {
            return f32::from_bits(sign | 0x7f80_0000);
        }
        let exp_u = u32::try_from(bumped).unwrap_or(0);
        return f32::from_bits(sign | (exp_u << 23));
    }
    let exp_u = u32::try_from(f32_exp).unwrap_or(0);
    let mant_u = u32::try_from(mant).unwrap_or(0);
    f32::from_bits(sign | (exp_u << 23) | mant_u)
}

fn narrow_subnormal(sign: u32, shift: i64, frac: u64) -> f32 {
    if shift > 25 {
        return f32::from_bits(sign);
    }
    let sig = (1u64 << 52) | frac;
    let cut = u32::try_from(29 + shift).unwrap_or(64);
    if cut >= 64 {
        return f32::from_bits(sign);
    }
    let mut mant = sig >> cut;
    let rest_mask = (1u64 << cut) - 1;
    let rest = sig & rest_mask;
    let half = 1u64 << (cut - 1);
    let odd = (mant & 1) == 1;
    if rest > half || (rest == half && odd) {
        mant += 1;
    }
    let mant_u = u32::try_from(mant).unwrap_or(0);
    if mant_u == 0 {
        return f32::from_bits(sign);
    }
    if mant_u >= 0x0080_0000 {
        return f32::from_bits(sign | (1 << 23));
    }
    f32::from_bits(sign | mant_u)
}

/// One input to a merge. `target_q` is the unshadowed luminance of this
/// reservoir's lamp at the destination receiver.
#[derive(Clone, Copy, Debug)]
pub struct MergeInput {
    pub reservoir: Reservoir,
    pub target_q: f32,
}

/// Which weight the merge writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weighting {
    /// Balance heuristic. Unbiased for a static set of reservoirs.
    Balance,
    /// Bitterli et al. 2020 section 3. Divides by the sum of `M`.
    Section3,
}

/// Merge `inputs[0]` (the canonical sample) with the rest.
///
/// `target_at(input_index, lamp)` is the unshadowed luminance of `lamp` at
/// that input's source receiver. `unit` is in `[0, 1)` and is consumed by
/// the caller even when the merge ignores it.
#[must_use]
pub fn merge(
    inputs: &[MergeInput],
    weighting: Weighting,
    target_at: impl Fn(usize, u32) -> f32,
    unit: f32,
) -> Reservoir {
    if weighting == Weighting::Section3 {
        return section3(inputs, unit);
    }
    if inputs.len() == 2 && inputs[0].reservoir.is_empty() {
        return if inputs[1].reservoir.is_empty() {
            Reservoir::empty()
        } else {
            inputs[1].reservoir
        };
    }
    balance(inputs, target_at, unit)
}

fn masses(inputs: &[MergeInput]) -> (Vec<f32>, f32) {
    let mut out = Vec::with_capacity(inputs.len());
    let mut area = 0.0;
    for input in inputs {
        let reservoir = input.reservoir;
        let mass = if reservoir.is_empty() || input.target_q <= 0.0 {
            0.0
        } else {
            m_as_f32(reservoir.m) * reservoir_w(reservoir) * input.target_q
        };
        area += mass;
        out.push(mass);
    }
    (out, area)
}

fn balance(inputs: &[MergeInput], target_at: impl Fn(usize, u32) -> f32, unit: f32) -> Reservoir {
    let (mass, area) = masses(inputs);
    let Some(selected) = pick(&mass, area, unit, inputs.len() == 2) else {
        return Reservoir::empty();
    };
    let chosen = &inputs[selected];
    let lamp = chosen.reservoir.lamp;
    let target_q = chosen.target_q.max(0.0);
    let target_src = target_at(selected, lamp).max(0.0);
    if target_q <= 0.0 || target_src <= 0.0 {
        return Reservoir::empty();
    }
    let mut denom = 0.0;
    let mut count = 0u32;
    for (index, input) in inputs.iter().enumerate() {
        if input.reservoir.is_empty() {
            continue;
        }
        count = count.saturating_add(input.reservoir.m);
        denom += m_as_f32(input.reservoir.m) * target_at(index, lamp).max(0.0);
    }
    if denom <= 0.0 || count == 0 {
        return Reservoir::empty();
    }
    let w_new = area * target_src / (target_q * denom);
    Reservoir {
        lamp,
        w_sum: w_new * m_as_f32(count) * target_q,
        m: count,
        target: target_q,
    }
}

fn section3(inputs: &[MergeInput], unit: f32) -> Reservoir {
    let (mass, area) = masses(inputs);
    let Some(selected) = pick(&mass, area, unit, inputs.len() == 2) else {
        return Reservoir::empty();
    };
    let mut count = 0u32;
    for input in inputs {
        if !input.reservoir.is_empty() {
            count = count.saturating_add(input.reservoir.m);
        }
    }
    let target_q = inputs[selected].target_q.max(0.0);
    if count == 0 || target_q <= 0.0 || area <= 0.0 {
        return Reservoir::empty();
    }
    let estimate = area / m_as_f32(count);
    Reservoir {
        lamp: inputs[selected].reservoir.lamp,
        w_sum: estimate * m_as_f32(count),
        m: count,
        target: target_q,
    }
}

/// Two inputs use the shader test `u < mass_history / area` (history is input 1).
/// A longer list uses a prefix sum in input order.
fn pick(mass: &[f32], area: f32, unit: f32, pair: bool) -> Option<usize> {
    if area <= 0.0 {
        return None;
    }
    if pair && mass.len() == 2 {
        let history = mass[1];
        if history > 0.0 && unit < history / area {
            return Some(1);
        }
        if mass[0] > 0.0 {
            return Some(0);
        }
        return if history > 0.0 { Some(1) } else { None };
    }
    let mark = unit * area;
    let mut acc = 0.0;
    let mut last = None;
    for (index, weight) in mass.iter().enumerate() {
        if *weight <= 0.0 {
            continue;
        }
        last = Some(index);
        acc += *weight;
        if mark < acc {
            return Some(index);
        }
    }
    last
}

/// Talbot update. Always draws one `f32` from `rng`.
pub fn stream_update(reservoir: &mut Reservoir, lamp: u32, stream: f32, rng: &mut Rng) {
    let unit = rng.next_f32();
    reservoir.m = reservoir.m.saturating_add(1);
    if stream > 0.0 {
        reservoir.w_sum += stream;
        if unit * reservoir.w_sum < stream {
            reservoir.lamp = lamp;
        }
    }
}

/// Write `target` for the selected lamp. An empty pick becomes `empty`.
pub fn finalize_initial(reservoir: &mut Reservoir, target: f32) {
    if reservoir.is_empty() || target <= 0.0 || reservoir.w_sum <= 0.0 {
        *reservoir = Reservoir::empty();
        return;
    }
    reservoir.target = target;
}

#[cfg(test)]
mod tests {
    use super::{f32_from_f64, merge, MergeInput, Reservoir, Weighting};

    fn input(lamp: u32, w_sum: f32, m: u32, target: f32, target_q: f32) -> MergeInput {
        MergeInput {
            reservoir: Reservoir {
                lamp,
                w_sum,
                m,
                target,
            },
            target_q,
        }
    }

    #[test]
    fn identical_domains_keep_w() {
        let left = input(1, 20.0, 2, 8.0, 8.0);
        let right = input(1, 30.0, 3, 8.0, 8.0);
        let before = super::reservoir_w(left.reservoir);
        let merged = merge(
            &[left, right],
            Weighting::Balance,
            |_index, lamp| {
                if lamp == 1 {
                    8.0
                } else {
                    0.0
                }
            },
            0.1,
        );
        let after = super::reservoir_w(merged);
        assert!((after - before).abs() < 1.0e-4, "W {before} became {after}");
        assert_eq!(merged.m, 5);
    }

    #[test]
    fn a_sample_the_neighbour_cannot_see_is_not_zeroed() {
        let canonical = input(1, 10.0, 1, 10.0, 10.0);
        let neighbour = input(2, 10.0, 20, 10.0, 0.0);
        let merged = merge(
            &[canonical, neighbour],
            Weighting::Balance,
            |index, lamp| {
                if index == 0 && lamp == 1 {
                    10.0
                } else {
                    0.0
                }
            },
            0.99,
        );
        assert!(
            (merged.estimate() - 10.0).abs() < 1.0e-3,
            "{}",
            merged.estimate()
        );
    }

    #[test]
    fn section3_estimate_differs_from_balance_when_domains_differ() {
        let canonical = input(2, 8.0, 1, 4.0, 4.0);
        let history = input(1, 40.0, 4, 20.0, 1.0);
        let target_at = |index: usize, lamp: u32| {
            if lamp == 2 {
                if index == 0 {
                    4.0
                } else {
                    1.0
                }
            } else if lamp == 1 {
                if index == 1 {
                    20.0
                } else {
                    1.0
                }
            } else {
                0.0
            }
        };
        let balance = merge(&[canonical, history], Weighting::Balance, target_at, 0.99);
        let biased = merge(&[canonical, history], Weighting::Section3, target_at, 0.99);
        assert!(
            (balance.estimate() - 5.0).abs() < 1.0e-3,
            "{}",
            balance.estimate()
        );
        assert!(
            (biased.estimate() - 2.0).abs() < 1.0e-3,
            "{}",
            biased.estimate()
        );
    }

    #[test]
    fn f32_from_f64_matches_the_widen_of_an_f32() {
        let patterns = [
            0u32,
            1,
            0x007f_ffff,
            0x0080_0000,
            0x3f80_0000,
            0x3f80_0001,
            0x7f7f_ffff,
            0x7f80_0000,
            0x7fc0_0000,
            0x8000_0000,
            0xbf80_0000,
            0xff80_0000,
        ];
        for bits in patterns {
            let exact = f64::from(f32::from_bits(bits));
            assert_eq!(f32_from_f64(exact).to_bits(), bits, "{bits:#x}");
        }
        // Halfway between 1 and the next f32. The even mantissa is 1.
        let tie = f64::from_bits((1023u64 << 52) | (1u64 << 28));
        assert_eq!(f32_from_f64(tie).to_bits(), 0x3f80_0000);
        let above = f64::from_bits((1023u64 << 52) | (1u64 << 28) | 1);
        assert_eq!(f32_from_f64(above).to_bits(), 0x3f80_0001);
    }
}
