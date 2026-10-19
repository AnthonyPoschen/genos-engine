#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_wrap, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::doc_markdown, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_midpoint, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::missing_errors_doc, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::redundant_closure_for_method_calls, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::similar_names, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]

//! LDR-FLIP, written from the paper, not from NVIDIA's binary.
//!
//! Pontus Andersson, Jim Nilsson, and Tomas Akenine-Möller, "FLIP: A Difference
//! Evaluator for Alternating Images", *Proceedings of the ACM on Computer
//! Graphics and Interactive Techniques* 3, 2 (2020). <https://doi.org/10.1145/3406183>
//!
//! Departures, so a number from here is not a number from NVIDIA's `flip` tool:
//! - The paper's CSF kernels are tabulated. Here Y is blurred with a Gaussian
//!   of σ = 0.04° and Cx, Cy with σ = 0.15°, at 67 pixels per degree (a 0.7 m
//!   display, 0.7 m away, 1920 px). A uniform field is unchanged by either.
//! - The paper's color difference uses Hunt's model and their published
//!   constants. Here it is `1 - exp(-d)` on a weighted YCxCy distance.
//! - Features are the absolute difference of Sobel magnitudes on Y.
//! - The per-pixel combination `color^(1 - feature)` follows the paper: when
//!   the edges agree, the error is the color error.

use crate::picture::{luma, Picture};

const PPD: f32 = 67.0;
const SIGMA_Y_DEG: f32 = 0.04;
const SIGMA_C_DEG: f32 = 0.15;

/// Per-pixel error in 0..1. `reference` and `candidate` must match in size.
pub fn flip_map(reference: &Picture, candidate: &Picture) -> Result<Vec<f32>, String> {
    if reference.width != candidate.width || reference.height != candidate.height {
        return Err("flip inputs differ in size".to_string());
    }
    let w = reference.width;
    let h = reference.height;
    let (y0, cx0, cy0) = planes(reference);
    let (y1, cx1, cy1) = planes(candidate);
    let y0b = blur(&y0, w, h, SIGMA_Y_DEG * PPD);
    let y1b = blur(&y1, w, h, SIGMA_Y_DEG * PPD);
    let cx0b = blur(&cx0, w, h, SIGMA_C_DEG * PPD);
    let cx1b = blur(&cx1, w, h, SIGMA_C_DEG * PPD);
    let cy0b = blur(&cy0, w, h, SIGMA_C_DEG * PPD);
    let cy1b = blur(&cy1, w, h, SIGMA_C_DEG * PPD);
    let e0 = sobel(&y0, w, h);
    let e1 = sobel(&y1, w, h);
    let n = reference.pixels();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let dy = (y1b[i] - y0b[i]) / 0.20;
        let dx = (cx1b[i] - cx0b[i]) / 0.10;
        let dz = (cy1b[i] - cy0b[i]) / 0.10;
        let dist = (dy * dy + dx * dx + dz * dz).sqrt();
        let color = 1.0 - (-dist).exp();
        let feature = (e1[i] - e0[i]).abs() / (e0[i].max(e1[i]) + 0.05);
        let feature = feature.clamp(0.0, 1.0);
        let value = color.powf(1.0 - feature).clamp(0.0, 1.0);
        out.push(value);
    }
    Ok(out)
}

pub fn mean_and_p99(values: &[f32]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let mut sorted: Vec<f64> = values.iter().map(|v| f64::from(*v)).collect();
    let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = (0.99 * sorted.len() as f64).ceil() as usize;
    let p99 = sorted[rank.saturating_sub(1).min(sorted.len() - 1)];
    (mean, p99)
}

fn planes(picture: &Picture) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let n = picture.pixels();
    let mut y = vec![0.0; n];
    let mut cx = vec![0.0; n];
    let mut cy = vec![0.0; n];
    for i in 0..n {
        let rgb = picture.pixel(i as u32 % picture.width, i as u32 / picture.width);
        y[i] = luma(rgb);
        cx[i] = 0.5 * (rgb[0] - rgb[1]);
        cy[i] = 0.5 * (0.5 * (rgb[0] + rgb[1]) - rgb[2]);
    }
    (y, cx, cy)
}

fn blur(src: &[f32], w: u32, h: u32, sigma: f32) -> Vec<f32> {
    if sigma < 0.25 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let radius = (3.0 * sigma).ceil() as i32;
    let mut kernel = Vec::with_capacity((radius * 2 + 1) as usize);
    let mut sum = 0.0f32;
    for i in -radius..=radius {
        let x = i as f32 / sigma;
        let weight = (-0.5 * x * x).exp();
        kernel.push(weight);
        sum += weight;
    }
    for weight in &mut kernel {
        *weight /= sum;
    }
    let mut tmp = vec![0.0f32; src.len()];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut acc = 0.0f32;
            for (k, weight) in kernel.iter().enumerate() {
                let xx = (x + k as i32 - radius).clamp(0, w as i32 - 1) as u32;
                acc += src[(y as u32 * w + xx) as usize] * weight;
            }
            tmp[(y as u32 * w + x as u32) as usize] = acc;
        }
    }
    let mut out = vec![0.0f32; src.len()];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut acc = 0.0f32;
            for (k, weight) in kernel.iter().enumerate() {
                let yy = (y + k as i32 - radius).clamp(0, h as i32 - 1) as u32;
                acc += tmp[(yy * w + x as u32) as usize] * weight;
            }
            out[(y as u32 * w + x as u32) as usize] = acc;
        }
    }
    out
}

fn sobel(src: &[f32], w: u32, h: u32) -> Vec<f32> {
    let mut out = vec![0.0f32; src.len()];
    if w < 3 || h < 3 {
        return out;
    }
    let at = |x: i32, y: i32| {
        src[(y.clamp(0, h as i32 - 1) as u32 * w + x.clamp(0, w as i32 - 1) as u32) as usize]
    };
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let gx = -at(x - 1, y - 1) + at(x + 1, y - 1) - 2.0 * at(x - 1, y) + 2.0 * at(x + 1, y)
                - at(x - 1, y + 1)
                + at(x + 1, y + 1);
            let gy = -at(x - 1, y - 1) - 2.0 * at(x, y - 1) - at(x + 1, y - 1)
                + at(x - 1, y + 1)
                + 2.0 * at(x, y + 1)
                + at(x + 1, y + 1);
            out[(y as u32 * w + x as u32) as usize] = (gx * gx + gy * gy).sqrt();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picture::Picture;

    #[test]
    fn identical_pictures_have_no_flip_error() {
        let picture = Picture::solid(16, 8, [0.4, 0.35, 0.2]);
        let map = flip_map(&picture, &picture).unwrap();
        let (mean, p99) = mean_and_p99(&map);
        assert!(mean < 1.0e-5, "{mean}");
        assert!(p99 < 1.0e-5, "{p99}");
    }

    #[test]
    fn a_brightness_bias_raises_flip() {
        let reference = Picture::solid(16, 8, [0.4, 0.4, 0.4]);
        let candidate = reference.scaled(1.4);
        let (mean, _) = mean_and_p99(&flip_map(&reference, &candidate).unwrap());
        assert!(mean > 0.2, "{mean}");
    }

    #[test]
    fn a_black_block_raises_the_flip_tail() {
        let reference = Picture::solid(16, 8, [0.45, 0.42, 0.38]);
        let mut candidate = reference.clone();
        candidate.fill_rect(2, 2, 4, 4, [0.0, 0.0, 0.0]);
        let (_, p99) = mean_and_p99(&flip_map(&reference, &candidate).unwrap());
        assert!(p99 > 0.4, "{p99}");
    }
}
