//! Per-frame error against a reference picture.
//!
//! Relative luminance is `|Yc - Yr| / max(Yr, rel_floor, rel_mean_frac * mean(Yr))`.
//! The mean term keeps a noisy shadow from dominating. A uniform brightness
//! bias on pixels above that floor is unchanged. FLIP is the second opinion
//! in [`crate::flip`].

use crate::flip::{flip_map, mean_and_p99};
use crate::picture::Picture;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricParams {
    /// Absolute denominator. `max(Yr, rel_floor, rel_mean_frac * mean(Yr))`.
    pub rel_floor: f64,
    /// Dark pixels are compared with this share of the picture's mean luminance,
    /// so a noisy shadow does not dominate the mean.
    pub rel_mean_frac: f64,
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
            rel_mean_frac: 0.25,
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
    /// Mean relative luminance after a box blur. This is the noise-aware error.
    pub blur_rel: Option<f64>,
    /// `mean(Y candidate) / mean(Y reference)`.
    pub luma_ratio: Option<f64>,
    /// `|luma_ratio - 1|`. The gate for a brightness bias.
    pub luma_delta: Option<f64>,
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

/// Mean relative luminance only. The settle loop calls [`blur_relative`] and
/// does not run FLIP.
pub fn mean_relative(
    reference: &Picture,
    candidate: &Picture,
    params: &MetricParams,
) -> Result<f64, String> {
    same_size(reference, candidate)?;
    let rel = relative(reference, candidate, params);
    Ok(mean_p99(&rel).0)
}

/// Radius of the box used by [`blur_relative`].
///
/// Owner pictures are 180 px tall, where this is 4: a 9 px kernel, the blur
/// the owner trial measured. A shorter picture scales the kernel down so a
/// planted defect on a tiny image is not washed out. Height 18 is radius 0.
pub fn blur_radius(height: u32) -> u32 {
    4 * height / 180
}

/// Line that ends a metric settle: the configured threshold, raised when the
/// reference's own noise, after the same blur, sits above it.
///
/// A box of side `2r+1` averages that many independent samples along each
/// axis, so the noise falls by about the kernel width.
pub fn settle_line(threshold: f64, reference_noise: f64, height: u32) -> f64 {
    let kernel = f64::from(blur_radius(height) * 2 + 1);
    threshold.max(reference_noise.max(0.0) / kernel)
}

/// Mean relative luminance after the box blur. Pixel noise falls. A brightness
/// bias does not.
pub fn blur_relative(
    reference: &Picture,
    candidate: &Picture,
    params: &MetricParams,
) -> Result<f64, String> {
    same_size(reference, candidate)?;
    let radius = blur_radius(reference.height);
    let left = box_blur_luma(reference, radius);
    let right = box_blur_luma(candidate, radius);
    let rel = relative_luma(&left, &right, params);
    Ok(mean_p99(&rel).0)
}

/// `mean(Y candidate) / mean(Y reference)`. A black reference with a lit
/// candidate is infinite.
pub fn mean_luma_ratio(reference: &Picture, candidate: &Picture) -> Result<f64, String> {
    same_size(reference, candidate)?;
    let mut reference_sum = 0.0f64;
    let mut candidate_sum = 0.0f64;
    for i in 0..reference.pixels() {
        reference_sum += f64::from(reference.luma_at(i));
        candidate_sum += f64::from(candidate.luma_at(i));
    }
    if reference_sum <= 1.0e-8 {
        return Ok(if candidate_sum <= 1.0e-8 {
            1.0
        } else {
            f64::INFINITY
        });
    }
    Ok(candidate_sum / reference_sum)
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
    let rel = relative(reference, candidate, params);
    let (mean_rel, p99_rel) = mean_p99(&rel);
    let (mean_rel_dark, mean_rel_lit, dark_pixels, lit_pixels) =
        dark_split(reference, &rel, params.dark_max);
    let (flip_mean, flip_p99) = mean_and_p99(&flip_map(reference, candidate)?);
    let unfilled = unfilled_count(reference, candidate, params);
    let blur_rel = blur_relative(reference, candidate, params)?;
    let luma_ratio = mean_luma_ratio(reference, candidate)?;
    let luma_delta = if luma_ratio.is_finite() {
        (luma_ratio - 1.0).abs()
    } else {
        f64::INFINITY
    };
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
        converge_1: converge_at(reference, after_event, 1, params)?,
        converge_2: converge_at(reference, after_event, 2, params)?,
        converge_4: converge_at(reference, after_event, 4, params)?,
        converge_8: converge_at(reference, after_event, 8, params)?,
        unfilled: Some(unfilled),
        pixels: reference.pixels() as u32,
        blur_rel: Some(blur_rel),
        luma_ratio: Some(luma_ratio),
        luma_delta: Some(luma_delta),
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

fn relative(reference: &Picture, candidate: &Picture, params: &MetricParams) -> Vec<f64> {
    let left: Vec<f32> = (0..reference.pixels())
        .map(|i| reference.luma_at(i))
        .collect();
    let right: Vec<f32> = (0..candidate.pixels())
        .map(|i| candidate.luma_at(i))
        .collect();
    relative_luma(&left, &right, params)
}

/// `|Yc - Yr| / max(Yr, rel_floor, rel_mean_frac * mean(Yr))`.
///
/// The fraction is the picture mean. It is not scaled by the reference noise,
/// so a missing bounce still fails and room-a is not given its own constant.
fn relative_luma(reference: &[f32], candidate: &[f32], params: &MetricParams) -> Vec<f64> {
    let mean = if reference.is_empty() {
        0.0
    } else {
        reference.iter().map(|y| f64::from(*y)).sum::<f64>() / reference.len() as f64
    };
    let floor = (params.rel_mean_frac * mean).max(params.rel_floor) as f32;
    reference
        .iter()
        .zip(candidate)
        .map(|(yr, yc)| f64::from((yc - yr).abs() / yr.max(floor)))
        .collect()
}

/// Separable box blur on luminance. Edges repeat the border pixel.
fn box_blur_luma(picture: &Picture, radius: u32) -> Vec<f32> {
    let w = picture.width as usize;
    let h = picture.height as usize;
    let src: Vec<f32> = (0..w * h).map(|i| picture.luma_at(i)).collect();
    if radius == 0 || w == 0 || h == 0 {
        return src;
    }
    let r = radius as i32;
    let mut horizontal = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0f32;
            let mut n = 0.0f32;
            for dx in -r..=r {
                let xx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
                acc += src[y * w + xx];
                n += 1.0;
            }
            horizontal[y * w + x] = acc / n;
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0f32;
            let mut n = 0.0f32;
            for dy in -r..=r {
                let yy = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
                acc += horizontal[yy * w + x];
                n += 1.0;
            }
            out[y * w + x] = acc / n;
        }
    }
    out
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
    params: &MetricParams,
) -> Result<Option<f64>, String> {
    let Some((_, picture)) = frames.iter().find(|(k, _)| *k == after) else {
        return Ok(None);
    };
    Ok(Some(blur_relative(reference, picture, params)?))
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
        assert!(m.blur_rel.unwrap() < 1.0e-6);
        assert!((m.luma_ratio.unwrap() - 1.0).abs() < 1.0e-6);
        assert!(m.luma_delta.unwrap() < 1.0e-6);
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
        let cheap = mean_relative(&reference, &candidate, &MetricParams::default()).unwrap();
        assert!((cheap - m.mean_rel.unwrap()).abs() < 1.0e-9);
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
        // The picture-mean floor lowers this. It stays past the dark fail band (0.287).
        assert!(
            m.mean_rel_dark.unwrap() > 0.40,
            "{}",
            m.mean_rel_dark.unwrap()
        );
        assert!(m.dark_pixels > 0);
    }

    #[test]
    fn a_nine_pixel_blur_drops_checker_noise_and_keeps_a_brightness_bias() {
        let (w, h) = (32u32, 180u32);
        let reference = Picture::solid(w, h, [0.40, 0.40, 0.40]);
        let mut noisy = reference.clone();
        for i in 0..reference.pixels() {
            let gain = if i % 2 == 0 { 1.35 } else { 0.65 };
            let v = 0.40 * gain;
            noisy.set_pixel(i as u32 % w, i as u32 / w, [v, v, v]);
        }
        let params = MetricParams::default();
        let noisy_m = measure(&reference, &noisy, None, &[], &params).unwrap();
        assert!(
            noisy_m.mean_rel.unwrap() > 0.30,
            "{}",
            noisy_m.mean_rel.unwrap()
        );
        assert!(
            noisy_m.blur_rel.unwrap() < 0.08,
            "blur {}",
            noisy_m.blur_rel.unwrap()
        );
        assert!(noisy_m.blur_rel.unwrap() < noisy_m.mean_rel.unwrap() / 4.0);
        let ratio = noisy_m.luma_ratio.unwrap();
        assert!((ratio - 1.0).abs() < 0.02, "{ratio}");
        assert_eq!(blur_radius(h), 4);
        assert_eq!(blur_radius(18), 0);
        let line = settle_line(0.05, 0.31, h);
        assert!((line - 0.05).abs() < 1.0e-9, "{line}");
        let raised = settle_line(0.05, 0.90, h);
        assert!((raised - 0.10).abs() < 1.0e-9, "{raised}");

        let dark = reference.scaled(0.66);
        let biased = measure(&reference, &dark, None, &[], &params).unwrap();
        assert!(biased.blur_rel.unwrap() > 0.30);
        assert!((biased.luma_ratio.unwrap() - 0.66).abs() < 1.0e-3);
        assert!((biased.luma_delta.unwrap() - 0.34).abs() < 1.0e-3);
    }

    #[test]
    fn dark_noise_is_relative_to_the_picture_mean() {
        // A lit field that matches, and a dark band whose absolute error is the
        // reference noise. Dividing by the dark value alone inflates the error.
        let (w, h) = (32u32, 180u32);
        let mut reference = Picture::solid(w, h, [0.50, 0.50, 0.50]);
        let mut candidate = reference.clone();
        reference.fill_rect(0, 120, w, 60, [0.012, 0.012, 0.012]);
        candidate.fill_rect(0, 120, w, 60, [0.028, 0.028, 0.028]);
        let bare = MetricParams {
            rel_mean_frac: 0.0,
            ..MetricParams::default()
        };
        let floored = MetricParams::default();
        let open = measure(&reference, &candidate, None, &[], &bare).unwrap();
        let held = measure(&reference, &candidate, None, &[], &floored).unwrap();
        assert!(
            open.blur_rel.unwrap() > 0.14,
            "unfloored {}",
            open.blur_rel.unwrap()
        );
        assert!(
            held.blur_rel.unwrap() < 0.10,
            "floored {}",
            held.blur_rel.unwrap()
        );
        assert!(held.blur_rel.unwrap() < open.blur_rel.unwrap() / 2.0);

        // A uniform field. The dark band above would shrink a scale error, and
        // hall and corner are a brightness bias on the lit picture.
        let field = Picture::solid(w, h, [0.40, 0.40, 0.40]);
        let hall = measure(&field, &field.scaled(0.83), None, &[], &floored).unwrap();
        assert!(hall.blur_rel.unwrap() > 0.16, "{}", hall.blur_rel.unwrap());
        assert!((hall.luma_ratio.unwrap() - 0.83).abs() < 1.0e-3);
        let corner = measure(&field, &field.scaled(1.23), None, &[], &floored).unwrap();
        assert!(
            corner.blur_rel.unwrap() > 0.22,
            "{}",
            corner.blur_rel.unwrap()
        );
    }
}
