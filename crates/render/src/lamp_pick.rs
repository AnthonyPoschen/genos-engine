#![expect(
    clippy::as_conversions,
    reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_precision_loss,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]

//! CPU replica of the point-lamp estimator in `lighting_direct.comp`.
//!
//! Compiled for tests. The shader is the one that shades.
//!
//! The shader is the one that shades. This module locks the weights: systematic
//! samples along the unshadowed prefix (Talbot, Cline, Egbert, EGSR 2005, the
//! same placement as `lamps_light_only`). Unbiased reservoir merges are pairwise
//! MIS (Bitterli et al. 2022). The biased merge is Bitterli et al. 2020 section 3
//! plus the M clamp. The sun is not a sample.

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

/// One reservoir sample's luminance.
///
/// The shade is `total * visibility` of the lamp the prefix selects: unbiased
/// for `sum(weight * visibility)`, and exact when every visibility matches.
/// `u` is in `[0, 1)`.
pub fn one_sample_luminance(weights: &[f32], visible: &[f32], u: f32) -> f32 {
    let total: f32 = weights.iter().copied().filter(|w| *w > 0.0).sum();
    let lit = weights.iter().filter(|w| **w > 0.0).count();
    if lit == 0 || total <= 0.0 {
        return 0.0;
    }
    let exact = lit <= 1;
    let step = total;
    let mut run = 0.0;
    let mut found = None;
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
        if times > 0.0 {
            found = Some(i);
        }
    }
    found.map_or(0.0, |i| total * visible.get(i).copied().unwrap_or(0.0))
}

/// Running mean of unbiased samples. A still pixel shows this, then holds.
pub fn cumulative_mean(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.iter().sum::<f32>() / samples.len() as f32
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

/// `W = w_sum / (M * target)`. Zero when the reservoir is empty.
pub fn reservoir_w(reservoir: Reservoir) -> f32 {
    let m = reservoir.m & 0x00ff_ffff;
    if m == 0 || reservoir.target <= 0.0 || reservoir.lamp == LAMP_EMPTY {
        0.0
    } else {
        reservoir.w_sum / (m as f32 * reservoir.target)
    }
}

/// Pairwise MIS of two reservoirs (Bitterli et al. 2022; Wyman, SIGGRAPH 2023).
///
/// The Jacobian is 1: these are point lamps, not area lights. `phat_q` is the
/// unshadowed luminance at the current pixel. `phat_src` is that luminance at
/// the pixel that produced the reservoir. Identical domains with the same `W`
/// keep `W`. A sample the other domain cannot see is not driven to zero.
pub fn pairwise_merge(
    mut canonical: Reservoir,
    mut history: Reservoir,
    phat_q: impl Fn(u32) -> f32,
    phat_hist: impl Fn(u32) -> f32,
    u: f32,
) -> Reservoir {
    canonical.m &= 0x00ff_ffff;
    history.m &= 0x00ff_ffff;
    if canonical.m == 0 || canonical.lamp == LAMP_EMPTY {
        return if history.m == 0 {
            Reservoir::empty()
        } else {
            history
        };
    }
    let phat_q_can = phat_q(canonical.lamp).max(0.0);
    let phat_q_hist = if history.lamp == LAMP_EMPTY {
        0.0
    } else {
        phat_q(history.lamp).max(0.0)
    };
    let mass_c = canonical.m as f32 * reservoir_w(canonical) * phat_q_can;
    let mass_h = if history.m == 0 {
        0.0
    } else {
        history.m as f32 * reservoir_w(history) * phat_q_hist
    };
    let area = mass_c + mass_h;
    if area <= 0.0 {
        return Reservoir::empty();
    }
    let pick_h = mass_h > 0.0 && u < mass_h / area;
    let lamp = if pick_h { history.lamp } else { canonical.lamp };
    let phat_q_sel = if pick_h { phat_q_hist } else { phat_q_can };
    let phat_src = if pick_h {
        phat_hist(history.lamp).max(0.0)
    } else {
        phat_q_can
    };
    let phat_other = if pick_h {
        phat_q_hist
    } else {
        phat_hist(canonical.lamp).max(0.0)
    };
    let m_sel = if pick_h { history.m } else { canonical.m };
    let m_other = if pick_h { canonical.m } else { history.m };
    let denom = m_sel as f32 * phat_src + m_other as f32 * phat_other;
    if phat_q_sel <= 0.0 || phat_src <= 0.0 || denom <= 0.0 {
        return Reservoir::empty();
    }
    let w_new = area * phat_src / (phat_q_sel * denom);
    let m_out = canonical.m.saturating_add(history.m);
    Reservoir {
        lamp,
        w_sum: w_new * m_out as f32 * phat_q_sel,
        m: m_out,
        target: phat_q_sel,
    }
}

/// Combine two reservoirs.
///
/// `unbiased` is pairwise MIS (Bitterli et al. 2022). It does not clamp `M`.
/// `unbiased == false` is the section-3 merge (always divide by the sum of `M`)
/// plus the `M` clamp from Wyman and Panteleev 2021. `phat_q` and `phat_hist`
/// are the unshadowed luminance of a lamp at the current pixel and at the
/// history pixel.
///
/// Bitterli et al. 2020 Algorithm 6 is not this merge. Its `Z` counts a source
/// only when that source's target of the selected lamp is positive, and that is
/// not the balance heuristic (equation 22). The shader used to follow Algorithm
/// 6 and then shade with `point_total / phat`, which is dark. The unbiased path
/// shades with `W`.
pub fn merge_reservoirs(
    mut canonical: Reservoir,
    mut history: Reservoir,
    phat_q: impl Fn(u32) -> f32,
    phat_hist: impl Fn(u32) -> f32,
    unbiased: bool,
    history_cap: u32,
    u: f32,
) -> Reservoir {
    // The shader stores the quiet-frame count in the high 8 bits of `m`.
    canonical.m &= 0x00ff_ffff;
    history.m &= 0x00ff_ffff;
    if unbiased {
        return pairwise_merge(canonical, history, phat_q, phat_hist, u);
    }
    if canonical.m == 0 || canonical.lamp == LAMP_EMPTY {
        return if history.m == 0 {
            Reservoir::empty()
        } else {
            history
        };
    }
    let hist_m = history.m.min(history_cap.max(1));
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
    let _ = phat_hist;
    let z = canonical.m.saturating_add(hist_m).max(1);
    let m_out = canonical.m.saturating_add(hist_m);
    Reservoir {
        lamp,
        w_sum: m_out as f32 * wsum / z as f32,
        m: m_out,
        target,
    }
}

/// Previous-frame camera, matching the change-block basis the shader reprojects with.
#[derive(Clone, Copy, Debug)]
pub struct ReplayCamera {
    pub eye: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    /// Tangent of half the vertical field of view (`view_right.w`).
    pub tan_half_fov: f32,
    pub aspect: f32,
}

/// Pixel the previous frame would have shaded for `pos`, and the depth along its forward.
///
/// `None` when the point is behind the camera or outside the picture. This is the
/// motion vector: same-pixel history is the special case where the result is the
/// current pixel. Depth and normal rejection happen after, on the stored record.
pub fn reproject_pixel(
    camera: ReplayCamera,
    pos: [f32; 3],
    width: u32,
    height: u32,
) -> Option<(u32, u32, f32)> {
    if camera.tan_half_fov == 0.0 || width == 0 || height == 0 {
        return None;
    }
    let rel = [
        pos[0] - camera.eye[0],
        pos[1] - camera.eye[1],
        pos[2] - camera.eye[2],
    ];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let z = dot(rel, camera.forward);
    if z <= 1.0e-3 {
        return None;
    }
    let ndc_x = dot(rel, camera.right) / (z * camera.tan_half_fov * camera.aspect);
    let ndc_y = dot(rel, camera.up) / (z * camera.tan_half_fov);
    let px = f32::midpoint(ndc_x, 1.0) * u32_as_f32(width);
    let py = f32::midpoint(-ndc_y, 1.0) * u32_as_f32(height);
    let x = pixel_of(px, width)?;
    let y = pixel_of(py, height)?;
    Some((x, y, z))
}

fn u32_as_f32(n: u32) -> f32 {
    let hi = f32::from(u16::try_from(n >> 16).unwrap_or(0));
    let lo = f32::from(u16::try_from(n & 0xffff).unwrap_or(0));
    hi.mul_add(65536.0, lo)
}

/// Greatest integer `<= coord` that is inside `0..limit`.
fn pixel_of(coord: f32, limit: u32) -> Option<u32> {
    if coord
        .partial_cmp(&0.0)
        .is_none_or(std::cmp::Ordering::is_lt)
        || limit == 0
    {
        return None;
    }
    if coord >= u32_as_f32(limit) {
        return None;
    }
    let mut lo = 0u32;
    let mut hi = limit;
    while lo + 1 < hi {
        let mid = lo.midpoint(hi);
        if u32_as_f32(mid) <= coord {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(lo)
}

/// Depth test shared with the shader: reject when the relative gap exceeds 2%
/// and the absolute gap exceeds 5 cm. A non-positive stored depth is a miss.
pub fn depth_accepts(z: f32, z_prev: f32) -> bool {
    if z_prev <= 0.0 || z <= 1.0e-3 {
        return false;
    }
    let gap = (z - z_prev).abs();
    !(gap / z > 0.02 && gap > 0.05)
}

/// Normal test shared with the shader.
pub fn normal_accepts(current: [f32; 3], previous: [f32; 3]) -> bool {
    let dot = current[0] * previous[0] + current[1] * previous[1] + current[2] * previous[2];
    dot > 0.9
}

/// A quiet pixel shades every lamp once this many positive lamps, or fewer.
/// Above it, one visibility sample is the cheaper unbiased estimate.
pub const EXACT_LIT_LAMPS: u32 = 8;

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

    #[test]
    fn merge_ignores_the_quiet_frame_count_in_the_high_bits() {
        let canonical = Reservoir {
            lamp: 1,
            w_sum: 10.0,
            m: 2,
            target: 10.0,
        };
        let history = Reservoir {
            lamp: 1,
            w_sum: 10.0,
            m: 3 | (32 << 24),
            target: 10.0,
        };
        let merged = merge_reservoirs(canonical, history, at(1, 10.0), at(1, 10.0), true, 20, 0.9);
        assert_eq!(merged.m, 5, "the frame count must not enter M");
    }

    #[test]
    fn the_cumulative_mean_converges_to_the_visibility_sum() {
        let weights = [1.0, 3.0, 6.0];
        let visible = [1.0, 0.0, 1.0];
        let reference: f32 = weights.iter().zip(visible).map(|(w, v)| w * v).sum();
        assert!((reference - 7.0).abs() < 1.0e-4);
        let mut samples = Vec::new();
        let mut u = 0.0;
        while u < 1.0 {
            samples.push(one_sample_luminance(&weights, &visible, u));
            u += 1.0 / 64.0;
        }
        let early = cumulative_mean(&samples[..32]);
        let late = cumulative_mean(&samples);
        assert!(
            (late - reference).abs() < 0.35,
            "late mean {late} reference {reference}"
        );
        assert!(
            (late - reference).abs() < (early - reference).abs(),
            "early {early} late {late} reference {reference}"
        );
        let stable = one_sample_luminance(&weights, &visible, 0.2);
        assert_eq!(
            cumulative_mean(&[stable, stable, stable]),
            stable,
            "a frozen seed does not boil"
        );
    }

    fn canonical(lamp: u32, total: f32, phat: f32) -> Reservoir {
        Reservoir {
            lamp,
            w_sum: total,
            m: 1,
            target: phat,
        }
    }

    #[test]
    fn pairwise_of_identical_domains_keeps_w() {
        let left = Reservoir {
            lamp: 1,
            w_sum: 20.0,
            m: 2,
            target: 8.0,
        };
        let right = Reservoir {
            lamp: 1,
            w_sum: 30.0,
            m: 3,
            target: 8.0,
        };
        let w_before = reservoir_w(left);
        let merged = pairwise_merge(left, right, at(1, 8.0), at(1, 8.0), 0.1);
        assert!(
            (reservoir_w(merged) - w_before).abs() < 1.0e-4,
            "W {} became {}",
            w_before,
            reservoir_w(merged)
        );
        assert_eq!(merged.m, 5);
    }

    #[test]
    fn pairwise_does_not_zero_a_sample_the_neighbour_cannot_see() {
        let merged = pairwise_merge(
            canonical(1, 10.0, 10.0),
            Reservoir {
                lamp: 2,
                w_sum: 10.0,
                m: 20,
                target: 10.0,
            },
            at(1, 10.0),
            |_id| 0.0,
            0.99,
        );
        assert!(
            (merged.estimate() - 10.0).abs() < 1.0e-3,
            "{}",
            merged.estimate()
        );
    }

    #[test]
    fn one_sample_w_times_phat_is_the_unshadowed_sum() {
        let sample = canonical(3, 14.0, 6.0);
        let shaded = reservoir_w(sample) * sample.target;
        assert!((shaded - 14.0).abs() < 1.0e-4, "{shaded}");
    }

    #[test]
    fn eight_positive_lamps_is_the_exact_sum() {
        assert_eq!(EXACT_LIT_LAMPS, 8);
    }

    #[test]
    fn reproject_lands_on_the_forward_pixel_and_rejects_a_depth_gap() {
        let camera = ReplayCamera {
            eye: [0.0, 1.0, 4.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            tan_half_fov: 0.5,
            aspect: 16.0 / 9.0,
        };
        let hit = reproject_pixel(camera, [0.0, 1.0, 0.0], 160, 90);
        assert!(hit.is_some(), "centre");
        let (x, y, z) = hit.unwrap_or((0, 0, 0.0));
        assert_eq!((x, y), (80, 45));
        assert!((z - 4.0).abs() < 1.0e-4, "{z}");
        assert!(depth_accepts(z, 4.0));
        assert!(!depth_accepts(z, 3.0), "20 cm at 4 m is over both bars");
        assert!(depth_accepts(z, 3.97), "3 cm is under the absolute bar");
        assert!(normal_accepts([0.0, 1.0, 0.0], [0.0, 1.0, 0.0]));
        assert!(!normal_accepts([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]));
        assert!(reproject_pixel(camera, [0.0, 1.0, 5.0], 160, 90).is_none());
    }
}
