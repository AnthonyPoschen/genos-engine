#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]

//! CPU replica of the point-lamp estimator in `lighting_direct.comp`.
//!
//! Compiled for tests. The shader is the one that shades.
//!
//! The shader is the one that shades. This module locks the weights: systematic
//! samples along the unshadowed prefix (Talbot, Cline, Egbert, EGSR 2005, the
//! same placement as `lamps_light_only`), and Bitterli et al. 2020 Algorithm 6
//! for combining two reservoirs. The sun is not a sample.

/// Luminance contribution of each point lamp.
///
/// `weights` are unshadowed luminances. A non-positive weight is skipped.
/// The returned value is what that lamp adds to the sum: the lamp's own weight
/// when every positive lamp is traced, otherwise `times * step`. The values
/// sum to the total weight. At most `rays` lamps are non-zero when the positive
/// count is above `rays`.
pub fn systematic_contribution(weights: &[f32], rays: u32, u: f32) -> Vec<f32> {
    let rays = rays.max(1);
    let mut out = vec![0.0; weights.len()];
    let total: f32 = weights.iter().copied().filter(|w| *w > 0.0).sum();
    let lit = weights.iter().filter(|w| **w > 0.0).count() as u32;
    if lit == 0 || total <= 0.0 {
        return out;
    }
    let exact = lit <= rays;
    let step = total / rays as f32;
    let mut run = 0.0;
    for (i, &w) in weights.iter().enumerate() {
        if w <= 0.0 {
            continue;
        }
        let times = if exact {
            1.0
        } else {
            let before = (run / step - u).ceil();
            run += w;
            (run / step - u).ceil() - before
        };
        out[i] = if exact { w } else { times * step };
    }
    out
}

/// Point-lamp contributions. `sun` is traced on its own and is not in the sum.
pub fn systematic_point(point: &[f32], sun: f32, rays: u32, u: f32) -> Vec<f32> {
    let _ = sun;
    systematic_contribution(point, rays, u)
}

/// How many point lamps a systematic draw traces. The sun is one extra ray.
pub fn traced_point_lamps(weights: &[f32], rays: u32, u: f32) -> u32 {
    systematic_contribution(weights, rays, u)
        .into_iter()
        .filter(|c| *c > 0.0)
        .count() as u32
}

/// Empty lamp index. Matches `LAMP_ALL` in the shade loop.
pub const LAMP_EMPTY: u32 = u32::MAX;

/// One pixel's reservoir. `target` is the unshadowed luminance of `lamp` at the
/// pixel that produced the reservoir.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reservoir {
    pub lamp: u32,
    pub w_sum: f32,
    pub m: u32,
    pub target: f32,
}

impl Reservoir {
    pub fn empty() -> Self {
        Self {
            lamp: LAMP_EMPTY,
            w_sum: 0.0,
            m: 0,
            target: 0.0,
        }
    }

    /// `p̂ * W`, the luminance the reservoir stands for before visibility.
    pub fn estimate(self) -> f32 {
        if self.m == 0 {
            0.0
        } else {
            self.w_sum / self.m as f32
        }
    }
}

/// Combine two reservoirs.
///
/// `unbiased` follows Bitterli et al. 2020 Algorithm 6: `Z` counts a source
/// only when its target of the selected lamp is positive. `unbiased == false`
/// is the section-3 merge (always divide by the sum of `M`) plus the `M` clamp
/// from Wyman and Panteleev 2021. `phat_q` and `phat_hist` are the unshadowed
/// luminance of a lamp at the current pixel and at the history pixel.
pub fn merge_reservoirs(
    canonical: Reservoir,
    history: Reservoir,
    phat_q: impl Fn(u32) -> f32,
    phat_hist: impl Fn(u32) -> f32,
    unbiased: bool,
    history_cap: u32,
    u: f32,
) -> Reservoir {
    if canonical.m == 0 || canonical.lamp == LAMP_EMPTY {
        return if history.m == 0 {
            Reservoir::empty()
        } else {
            history
        };
    }
    let hist_m = if unbiased {
        history.m
    } else {
        history.m.min(history_cap.max(1))
    };
    let phat_can = phat_q(canonical.lamp).max(0.0);
    let phat_h = if history.lamp == LAMP_EMPTY {
        0.0
    } else {
        phat_q(history.lamp).max(0.0)
    };
    let w_can = if canonical.target > 0.0 && phat_can > 0.0 {
        canonical.w_sum * phat_can / canonical.target
    } else {
        0.0
    };
    let w_hist = if hist_m > 0 && history.target > 0.0 && phat_h > 0.0 {
        let clamped = hist_m as f32 / history.m.max(1) as f32;
        history.w_sum * phat_h / history.target * clamped
    } else {
        0.0
    };
    let mut wsum = 0.0;
    let mut lamp = LAMP_EMPTY;
    let mut target = 0.0;
    if w_can > 0.0 {
        wsum = w_can;
        lamp = canonical.lamp;
        target = phat_can;
    }
    if w_hist > 0.0 {
        wsum += w_hist;
        if wsum > 0.0 && u < w_hist / wsum {
            lamp = history.lamp;
            target = phat_h;
        }
    }
    if lamp == LAMP_EMPTY || target <= 0.0 {
        return Reservoir::empty();
    }
    let hist_counts = hist_m > 0 && phat_hist(lamp) > 0.0;
    let can_counts = canonical.m > 0 && phat_q(lamp) > 0.0;
    let z = if unbiased {
        canonical.m * u32::from(can_counts) + hist_m * u32::from(hist_counts)
    } else {
        canonical.m.saturating_add(hist_m)
    }
    .max(1);
    let m_out = canonical.m.saturating_add(hist_m);
    Reservoir {
        lamp,
        w_sum: m_out as f32 * wsum / z as f32,
        m: m_out,
        target,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systematic_weights_sum_and_the_heavy_lamp_keeps_its_share() {
        let weights = [1.0, 1.0, 8.0];
        let sun = 100.0;
        let mut heavy = 0.0;
        for step in 0..64 {
            let u = step as f32 / 64.0;
            let got = systematic_point(&weights, sun, 2, u);
            let sum: f32 = got.iter().sum();
            assert!(
                (sum - 10.0).abs() < 1.0e-4,
                "u {u} summed {sum}, sun {sun} stayed out"
            );
            assert!(got.iter().filter(|c| **c > 0.0).count() <= 2);
            heavy += got[2];
        }
        let mean_share = (heavy / 64.0) / 10.0;
        assert!(
            (mean_share - 0.8).abs() < 0.02,
            "heavy lamp share {mean_share}"
        );
    }

    #[test]
    fn systematic_traces_every_lamp_when_the_sample_count_covers_them() {
        let got = systematic_contribution(&[1.0, 1.0, 8.0], 4, 0.3);
        assert_eq!(got, vec![1.0, 1.0, 8.0]);
    }

    #[test]
    fn traced_count_does_not_grow_with_the_lamp_count() {
        let few = vec![1.0; 3];
        let many = vec![1.0; 12];
        let mut few_rays = 0u32;
        let mut many_rays = 0u32;
        for step in 0..32 {
            let u = step as f32 / 32.0;
            few_rays += traced_point_lamps(&few, 1, u);
            many_rays += traced_point_lamps(&many, 1, u);
        }
        assert_eq!(few_rays, 32);
        assert_eq!(many_rays, 32);
        assert!(traced_point_lamps(&many, 1, 0.2) < traced_point_lamps(&many, 12, 0.2));
    }

    fn at(lamp: u32, value: f32) -> impl Fn(u32) -> f32 {
        move |id| if id == lamp { value } else { 0.0 }
    }

    #[test]
    fn unbiased_merge_keeps_the_estimate_when_the_neighbour_cannot_see_it() {
        let canonical = Reservoir {
            lamp: 1,
            w_sum: 10.0,
            m: 1,
            target: 10.0,
        };
        let history = Reservoir {
            lamp: 2,
            w_sum: 10.0,
            m: 20,
            target: 10.0,
        };
        // The history lamp is dark here, and the history pixel cannot see lamp 1.
        let merged = merge_reservoirs(canonical, history, at(1, 10.0), |_id| 0.0, true, 20, 0.5);
        assert!(
            (merged.estimate() - 10.0).abs() < 1.0e-3,
            "{}",
            merged.estimate()
        );
        let biased = merge_reservoirs(canonical, history, at(1, 10.0), |_id| 0.0, false, 20, 0.5);
        assert!(
            biased.estimate() < 1.0,
            "section 3 darkens, got {}",
            biased.estimate()
        );
    }

    #[test]
    fn unbiased_merge_of_two_live_reservoirs_keeps_the_sum() {
        let canonical = Reservoir {
            lamp: 1,
            w_sum: 20.0,
            m: 2,
            target: 8.0,
        };
        let history = Reservoir {
            lamp: 1,
            w_sum: 30.0,
            m: 3,
            target: 8.0,
        };
        let merged = merge_reservoirs(canonical, history, at(1, 8.0), at(1, 8.0), true, 20, 0.9);
        assert!(
            (merged.estimate() - 10.0).abs() < 1.0e-3,
            "{}",
            merged.estimate()
        );
        assert_eq!(merged.m, 5);
    }
}
