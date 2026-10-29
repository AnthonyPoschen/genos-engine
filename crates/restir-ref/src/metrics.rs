//! Receiver-grid stand-ins for the oracle keys in ADR 0022.
//!
//! `ref_err_near` is `sum(w |live-ref|) / sum(w ref)` in Rec. 709 luminance,
//! `w = 1 / max(distance, 0.5 m)` from the scene camera. `grid_worst` is the
//! checkerboard score from `scripts/lighting/grid_metric.py` (tile 1) on the
//! frame sequence. A step under 0.002 is skipped, so a still picture scores 0.
//! `frames_to_ref` is the first 1-based frame whose error stays within 1.1×
//! the last frame. These grade the multi-seed mean of the unoccluded gate
//! scene. The product gates stay `make lighting-oracle` and the 4070 timestamp.

use genos_math::Vec3;

use crate::shade::{luminance, Scene};

/// Ceilings for a reference self-check. `frame_ms` is the CPU microbench, not the 3 ms 4070 budget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GateLimits {
    pub ref_err_near: f32,
    pub grid_worst: f32,
    pub frames_to_ref: u32,
    pub frame_ms: f32,
}

impl GateLimits {
    /// Bias 5%, checkerboard 0.15 (ADR 0022 stage C), settle by frame 8, CPU step under 50 ms.
    #[must_use]
    pub const fn reference() -> Self {
        Self {
            ref_err_near: 0.05,
            grid_worst: 0.15,
            frames_to_ref: 8,
            frame_ms: 50.0,
        }
    }
}

/// Near-weighted relative luminance error of `estimate` against `brute`.
#[must_use]
pub fn ref_err_near(estimate: &[Vec3], brute: &[Vec3], scene: &Scene) -> f32 {
    let live: Vec<f32> = estimate.iter().copied().map(luminance).collect();
    ref_err_luma(&live, brute, scene)
}

#[must_use]
pub fn ref_err_luma(live: &[f32], brute: &[Vec3], scene: &Scene) -> f32 {
    let mut num = 0.0;
    let mut den = 0.0;
    for (index, reference) in brute.iter().enumerate() {
        let Some(receiver) = scene.receivers.get(index) else {
            continue;
        };
        let distance = (receiver.position - scene.camera).length().max(0.5);
        let weight = 1.0 / distance;
        let got = live.get(index).copied().unwrap_or(0.0);
        let expect = luminance(*reference);
        num += weight * (got - expect).abs();
        den += weight * expect;
    }
    if den <= 1.0e-8 {
        0.0
    } else {
        num / den
    }
}

/// Worst frame-to-frame checkerboard score. `0` when the grid is under 3×3 or the step is tiny.
#[must_use]
pub fn grid_worst(frames: &[Vec<f32>], width: usize) -> f32 {
    let mut worst = 0.0f32;
    for index in 1..frames.len() {
        let Some(prev) = frames.get(index - 1) else {
            continue;
        };
        let Some(cur) = frames.get(index) else {
            continue;
        };
        let (score, step) = grid_score(prev, cur, width);
        if step < 0.002 {
            continue;
        }
        worst = worst.max(score);
    }
    worst
}

/// 1-based index of the first frame that stays within 1.1× the final error.
/// One past the end when the last frame itself is outside that band.
#[must_use]
pub fn frames_to_ref(errors: &[f32]) -> u32 {
    let Some(settled) = errors.last().copied() else {
        return 1;
    };
    let limit = settled * 1.1;
    let mut last_bad = None;
    for (index, error) in errors.iter().enumerate() {
        if *error > limit {
            last_bad = Some(index);
        }
    }
    match last_bad {
        None => 1,
        Some(index) => u32::try_from(index.saturating_add(2)).unwrap_or(u32::MAX),
    }
}

fn grid_score(prev: &[f32], cur: &[f32], width: usize) -> (f32, f32) {
    let Some((left, tiles_w)) = tile_means(prev, width) else {
        return (0.0, 0.0);
    };
    let Some((right, _)) = tile_means(cur, width) else {
        return (0.0, 0.0);
    };
    let tiles_h = left.len() / tiles_w.max(1);
    let mut delta = vec![0.0; left.len()];
    for (index, (a, b)) in left.iter().zip(&right).enumerate() {
        delta[index] = b - a;
    }
    let rough = roughness(&delta, tiles_w, tiles_h);
    let mean_abs = delta.iter().map(|value| value.abs()).sum::<f32>()
        / crate::reservoir::m_as_f32(u32::try_from(delta.len()).unwrap_or(1));
    (rough / (mean_abs + 1.0e-3), mean_abs)
}

fn tile_means(image: &[f32], width: usize) -> Option<(Vec<f32>, usize)> {
    if width < 3 || !image.len().is_multiple_of(width) {
        return None;
    }
    let height = image.len() / width;
    if height < 3 {
        return None;
    }
    Some((image.to_vec(), width))
}

fn roughness(delta: &[f32], width: usize, height: usize) -> f32 {
    let padded_w = width.saturating_add(2);
    let padded_h = height.saturating_add(2);
    let mut padded = vec![0.0; padded_w.saturating_mul(padded_h)];
    for y in 0..height {
        for x in 0..width {
            let value = delta
                .get(y.saturating_mul(width).saturating_add(x))
                .copied()
                .unwrap_or(0.0);
            let at = (y + 1).saturating_mul(padded_w).saturating_add(x + 1);
            if let Some(slot) = padded.get_mut(at) {
                *slot = value;
            }
        }
    }
    for x in 0..width {
        copy_pad(&mut padded, padded_w, 0, x + 1, 1, x + 1);
        copy_pad(
            &mut padded,
            padded_w,
            padded_h - 1,
            x + 1,
            padded_h - 2,
            x + 1,
        );
    }
    for y in 0..padded_h {
        copy_pad(&mut padded, padded_w, y, 0, y, 1);
        copy_pad(&mut padded, padded_w, y, padded_w - 1, y, padded_w - 2);
    }
    let mut sum = 0.0;
    for y in 0..height {
        for x in 0..width {
            let value = delta
                .get(y.saturating_mul(width).saturating_add(x))
                .copied()
                .unwrap_or(0.0);
            let median = neighbor_median(&padded, padded_w, x + 1, y + 1);
            sum += (value - median).abs();
        }
    }
    sum / crate::reservoir::m_as_f32(u32::try_from(delta.len()).unwrap_or(1))
}

fn copy_pad(
    padded: &mut [f32],
    width: usize,
    y_to: usize,
    x_to: usize,
    y_from: usize,
    x_from: usize,
) {
    let from = y_from.saturating_mul(width).saturating_add(x_from);
    let to = y_to.saturating_mul(width).saturating_add(x_to);
    let value = padded.get(from).copied().unwrap_or(0.0);
    if let Some(slot) = padded.get_mut(to) {
        *slot = value;
    }
}

fn neighbor_median(padded: &[f32], width: usize, x: usize, y: usize) -> f32 {
    const OFFSETS: [(isize, isize); 8] = [
        (-1, -1),
        (-1, 0),
        (-1, 1),
        (0, -1),
        (0, 1),
        (1, -1),
        (1, 0),
        (1, 1),
    ];
    let mut values = [0.0; 8];
    for (index, (dy, dx)) in OFFSETS.iter().enumerate() {
        let py = isize_add(y, *dy);
        let px = isize_add(x, *dx);
        let at = py.saturating_mul(width).saturating_add(px);
        values[index] = padded.get(at).copied().unwrap_or(0.0);
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    f32::midpoint(values[3], values[4])
}

fn isize_add(value: usize, delta: isize) -> usize {
    let base = isize::try_from(value).unwrap_or(0);
    usize::try_from(base.saturating_add(delta)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{frames_to_ref, grid_worst};

    #[test]
    fn a_flat_step_is_not_a_checkerboard() {
        let frame_a = vec![0.4; 64];
        let frame_b = vec![0.5; 64];
        let worst = grid_worst(&[frame_a, frame_b], 8);
        assert!(worst < 0.15, "flat {worst}");
    }

    #[test]
    fn a_checkerboard_step_fails_the_gate() {
        let mut frame_b = vec![0.5; 64];
        for y in 0..8 {
            for x in 0..8 {
                let sign = if (x + y) % 2 == 0 { 0.25 } else { -0.25 };
                frame_b[y * 8 + x] = 0.5 + sign;
            }
        }
        let worst = grid_worst(&[vec![0.5; 64], frame_b], 8);
        assert!(worst > 0.15, "checker {worst}");
    }

    #[test]
    fn frames_to_ref_waits_until_the_tail_stays_close() {
        let errors = [0.20, 0.10, 0.04, 0.03];
        assert_eq!(frames_to_ref(&errors), 4);
        assert_eq!(frames_to_ref(&[0.01, 0.01, 0.01]), 1);
    }
}
