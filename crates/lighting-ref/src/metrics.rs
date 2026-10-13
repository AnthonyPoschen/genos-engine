//! Per-frame error against a reference picture.
//!
//! Relative luminance is `|Yc - Yr| / max(Yr, floor)`. The floor keeps a dark
//! pixel from exploding. FLIP is the second opinion in [`crate::flip`].

use crate::flip::{flip_map, mean_and_p99};
use crate::picture::Picture;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricParams {
    pub rel_floor: f64,
    pub dark_max: f64,
    /// Candidate luminance at or below this can count as unfilled.
    pub unfilled_candidate_max: f64,
    /// Only where the reference is at least this bright.
    pub unfilled_reference_min: f64,
}

impl Default for MetricParams {
    fn default() -> Self {
        Self {
            rel_floor: 0.02,
            dark_max: 0.05,
            unfilled_candidate_max: 0.004,
            unfilled_reference_min: 0.02,
        }
    }
}

/// `None` means the metric was not measured, not that the error is zero.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameMetrics {
    pub mean_rel: Option<f64>,
    pub p99_rel: Option<f64>,
    pub mean_rel_dark: Option<f64>,
    pub mean_rel_lit: Option<f64>,
    pub dark_pixels: u32,
    pub lit_pixels: u32,
    pub flip_mean: Option<f64>,
    pub flip_p99: Option<f64>,
    pub flicker: Option<f64>,
    pub converge_1: Option<f64>,
    pub converge_2: Option<f64>,
    pub converge_4: Option<f64>,
    pub converge_8: Option<f64>,
    pub unfilled: Option<u32>,
    pub pixels: u32,
}

impl FrameMetrics {
    pub fn unfilled_share(&self) -> Option<f64> {
        let count = self.unfilled?;
        if self.pixels == 0 {
            return None;
        }
        Some(f64::from(count) / f64::from(self.pixels))
    }
}

/// `still` is a fixed camera over time. `after_event` is `(frames after, picture)`
/// compared with `reference`, which is the settled picture after the change.
pub fn measure(
    reference: &Picture,
    candidate: &Picture,
    still: Option<&[Picture]>,
    after_event: &[(u32, &Picture)],
    params: &MetricParams,
) -> Result<FrameMetrics, String> {
    same_size(reference, candidate)?;
    let rel = relative(reference, candidate, params.rel_floor);
    let (mean_rel, p99_rel) = mean_p99(&rel);
    let (mean_rel_dark, mean_rel_lit, dark_pixels, lit_pixels) =
        dark_split(reference, &rel, params.dark_max);
    let (flip_mean, flip_p99) = mean_and_p99(&flip_map(reference, candidate)?);
    let unfilled = unfilled_count(reference, candidate, params);
    Ok(FrameMetrics {
        mean_rel: Some(mean_rel),
        p99_rel: Some(p99_rel),
        mean_rel_dark,
        mean_rel_lit,
        dark_pixels,
        lit_pixels,
        flip_mean: Some(flip_mean),
        flip_p99: Some(flip_p99),
        flicker: flicker(still, params.rel_floor)?,
        converge_1: converge_at(reference, after_event, 1, params.rel_floor)?,
        converge_2: converge_at(reference, after_event, 2, params.rel_floor)?,
        converge_4: converge_at(reference, after_event, 4, params.rel_floor)?,
        converge_8: converge_at(reference, after_event, 8, params.rel_floor)?,
        unfilled: Some(unfilled),
        pixels: reference.pixels() as u32,
    })
}

fn same_size(reference: &Picture, candidate: &Picture) -> Result<(), String> {
    if reference.width != candidate.width || reference.height != candidate.height {
        return Err(format!(
            "size {}x{} vs {}x{}",
            reference.width, reference.height, candidate.width, candidate.height
        ));
    }
    Ok(())
}

fn relative(reference: &Picture, candidate: &Picture, floor: f64) -> Vec<f64> {
    let floor = floor as f32;
    (0..reference.pixels())
        .map(|i| {
            let yr = reference.luma_at(i);
            let yc = candidate.luma_at(i);
            f64::from((yc - yr).abs() / yr.max(floor))
        })
        .collect()
}

fn mean_p99(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = (0.99 * sorted.len() as f64).ceil() as usize;
    let p99 = sorted[rank.saturating_sub(1)];
    (mean, p99)
}

fn dark_split(
    reference: &Picture,
    rel: &[f64],
    dark_max: f64,
) -> (Option<f64>, Option<f64>, u32, u32) {
    let mut dark = 0.0f64;
    let mut lit = 0.0f64;
    let mut dark_n = 0u32;
    let mut lit_n = 0u32;
    for (i, err) in rel.iter().enumerate() {
        if f64::from(reference.luma_at(i)) < dark_max {
            dark += err;
            dark_n += 1;
        } else {
            lit += err;
            lit_n += 1;
        }
    }
    (
        (dark_n > 0).then_some(dark / f64::from(dark_n)),
        (lit_n > 0).then_some(lit / f64::from(lit_n)),
        dark_n,
        lit_n,
    )
}

fn unfilled_count(reference: &Picture, candidate: &Picture, params: &MetricParams) -> u32 {
    (0..reference.pixels())
        .filter(|&i| {
            f64::from(candidate.luma_at(i)) <= params.unfilled_candidate_max
                && f64::from(reference.luma_at(i)) >= params.unfilled_reference_min
        })
        .count() as u32
}

fn flicker(still: Option<&[Picture]>, floor: f64) -> Result<Option<f64>, String> {
    let Some(frames) = still else {
        return Ok(None);
    };
    if frames.len() < 2 {
        return Ok(None);
    }
    let first = &frames[0];
    for frame in frames {
        same_size(first, frame)?;
    }
    let n = first.pixels();
    let mut acc = 0.0f64;
    for i in 0..n {
        let mut mean = 0.0f64;
        for frame in frames {
            mean += f64::from(frame.luma_at(i));
        }
        mean /= frames.len() as f64;
        let mut var = 0.0f64;
        for frame in frames {
            let d = f64::from(frame.luma_at(i)) - mean;
            var += d * d;
        }
        var /= frames.len() as f64;
        acc += var.sqrt() / mean.max(floor);
    }
    Ok(Some(acc / n as f64))
}

fn converge_at(
    reference: &Picture,
    frames: &[(u32, &Picture)],
    after: u32,
    floor: f64,
) -> Result<Option<f64>, String> {
    let Some((_, picture)) = frames.iter().find(|(k, _)| *k == after) else {
        return Ok(None);
    };
    same_size(reference, picture)?;
    let rel = relative(reference, picture, floor);
    Ok(Some(mean_p99(&rel).0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(v: f32) -> Picture {
        Picture::solid(16, 8, [v, v, v])
    }

    #[test]
    fn identical_frames_are_zero_error() {
        let picture = gray(0.4);
        let m = measure(
            &picture,
            &picture,
            Some(&[picture.clone(), picture.clone()]),
            &[],
            &MetricParams::default(),
        )
        .unwrap();
        assert!(m.mean_rel.unwrap() < 1.0e-6);
        assert!(m.p99_rel.unwrap() < 1.0e-6);
        assert!(m.flip_mean.unwrap() < 1.0e-5);
        assert_eq!(m.unfilled, Some(0));
        assert!(m.flicker.unwrap() < 1.0e-6);
        assert!(m.converge_1.is_none());
    }

    #[test]
    fn a_brightness_bias_is_the_relative_luminance() {
        let reference = gray(0.4);
        let candidate = reference.scaled(1.4);
        let m = measure(&reference, &candidate, None, &[], &MetricParams::default()).unwrap();
        assert!((m.mean_rel.unwrap() - 0.4).abs() < 1.0e-4);
        assert!(m.flip_mean.unwrap() > 0.2);
        assert!(m.flicker.is_none());
    }

    #[test]
    fn a_black_block_counts_as_unfilled_and_lifts_p99() {
        let reference = gray(0.45);
        let mut candidate = reference.clone();
        candidate.fill_rect(1, 1, 4, 4, [0.0, 0.0, 0.0]);
        let m = measure(&reference, &candidate, None, &[], &MetricParams::default()).unwrap();
        assert_eq!(m.unfilled, Some(16));
        assert!(m.p99_rel.unwrap() > 0.9);
    }

    #[test]
    fn flicker_is_temporal_std_on_a_still_camera() {
        let reference = gray(0.4);
        let hi = gray(0.5);
        let lo = gray(0.3);
        let average = Picture::average(&[hi.clone(), lo.clone()]).unwrap();
        let m = measure(
            &reference,
            &average,
            Some(&[hi, lo]),
            &[],
            &MetricParams::default(),
        )
        .unwrap();
        assert!(m.mean_rel.unwrap() < 1.0e-4);
        let flicker = m.flicker.unwrap();
        assert!((flicker - 0.25).abs() < 1.0e-3, "{flicker}");
    }

    #[test]
    fn convergence_reads_error_at_one_two_four_and_eight() {
        let reference = gray(0.5);
        let stale = gray(0.2);
        let frames = [(1u32, &stale), (2, &stale), (4, &stale), (8, &stale)];
        let m = measure(&reference, &stale, None, &frames, &MetricParams::default()).unwrap();
        for value in [m.converge_1, m.converge_2, m.converge_4, m.converge_8] {
            assert!((value.unwrap() - 0.6).abs() < 1.0e-4);
        }
    }

    #[test]
    fn a_missing_bounce_shows_up_in_the_dark_split() {
        let mut reference = Picture::solid(32, 8, [0.50, 0.48, 0.45]);
        reference.fill_rect(16, 0, 16, 8, [0.09, 0.015, 0.012]);
        let mut candidate = Picture::solid(32, 8, [0.50, 0.48, 0.45]);
        candidate.fill_rect(16, 0, 16, 8, [0.003, 0.003, 0.003]);
        let m = measure(&reference, &candidate, None, &[], &MetricParams::default()).unwrap();
        assert!(m.mean_rel_lit.unwrap() < 0.02);
        assert!(m.mean_rel_dark.unwrap() > 0.5);
        assert!(m.dark_pixels > 0);
    }
}
