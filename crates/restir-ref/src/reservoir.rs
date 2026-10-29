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
    use super::{merge, MergeInput, Reservoir, Weighting};

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
}
