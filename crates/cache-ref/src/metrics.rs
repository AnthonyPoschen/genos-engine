//! Accuracy against the path-traced reference.
//!
//! Blotch mirrors the engine's `gi_check` metric
//! (`examples/stress/src/gi_check/metrics.rs`): error `e = live - ref`,
//! band = `blur(e, s) - blur(e, 6 s)`, blotch = mean `|band| / (blur(ref, s) +
//! 0.05 mean(ref))`. Here the planes are the receiver patches (world pixels,
//! not screen pixels) and the blur skips pixels inside geometry.

use crate::geom::{fl, floor_usize};
use crate::scene::Scene;

/// Energy ratio: summed cache over summed reference on valid pixels.
#[must_use]
pub fn energy(scene: &Scene, live: &[f64], reference: &[f64]) -> f64 {
    let (mut a, mut b) = (0.0, 0.0);
    for (i, px) in scene.pixels.iter().enumerate() {
        if px.valid {
            a += live[i];
            b += reference[i];
        }
    }
    if b > 0.0 {
        a / b
    } else {
        0.0
    }
}

/// Mean reference over valid pixels.
#[must_use]
pub fn mean_valid(scene: &Scene, v: &[f64]) -> f64 {
    let n = scene.valid_pixels();
    let s: f64 = scene
        .pixels
        .iter()
        .zip(v)
        .filter(|(p, _)| p.valid)
        .map(|(_, x)| x)
        .sum();
    s / fl(n.max(1))
}

/// Signed relative error summed over `region`: `(sum live - sum ref) / sum ref`.
#[must_use]
pub fn region_bias(region: &[usize], live: &[f64], reference: &[f64]) -> f64 {
    let a: f64 = region.iter().map(|&i| live[i]).sum();
    let b: f64 = region.iter().map(|&i| reference[i]).sum();
    if b > 0.0 {
        (a - b) / b
    } else {
        0.0
    }
}

fn kernel(sigma: f64) -> Vec<f64> {
    let r = floor_usize(3.0 * sigma).max(1);
    let mut k: Vec<f64> = (0..=2 * r)
        .map(|i| {
            let x = fl(i) - fl(r);
            (-0.5 * x * x / (sigma * sigma)).exp()
        })
        .collect();
    let s: f64 = k.iter().sum();
    for v in &mut k {
        *v /= s;
    }
    k
}

/// Masked separable Gaussian blur of a `w x h` plane.
fn blur(v: &[f64], mask: &[bool], w: usize, h: usize, sigma: f64) -> Vec<f64> {
    let k = kernel(sigma);
    let r = k.len() / 2;
    let pass = |src: &[f64], wsrc: &[f64], horizontal: bool| -> (Vec<f64>, Vec<f64>) {
        let mut out = vec![0.0; w * h];
        let mut wout = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                let (mut a, mut b) = (0.0, 0.0);
                for (i, kv) in k.iter().enumerate() {
                    let (xx, yy) = if horizontal {
                        ((x + i).checked_sub(r), Some(y))
                    } else {
                        (Some(x), (y + i).checked_sub(r))
                    };
                    if let (Some(xx), Some(yy)) = (xx, yy) {
                        if xx < w && yy < h {
                            let j = yy * w + xx;
                            a += kv * src[j];
                            b += kv * wsrc[j];
                        }
                    }
                }
                out[y * w + x] = a;
                wout[y * w + x] = b;
            }
        }
        (out, wout)
    };
    let m: Vec<f64> = mask.iter().map(|&b| if b { 1.0 } else { 0.0 }).collect();
    let vm: Vec<f64> = v.iter().zip(&m).map(|(a, b)| a * b).collect();
    let (t, tw) = pass(&vm, &m, true);
    let (o, ow) = pass(&t, &tw, false);
    o.iter()
        .zip(&ow)
        .map(|(a, b)| if *b > 1e-9 { a / b } else { 0.0 })
        .collect()
}

/// Blotch at blur scale `sigma_m` metres (coarse scale `6 sigma`).
#[must_use]
pub fn blotch(scene: &Scene, live: &[f64], reference: &[f64], sigma_m: f64) -> f64 {
    blotch_in(scene, live, reference, sigma_m, None)
}

/// Blotch averaged over the pixels where `region` is true (all when `None`).
#[must_use]
pub fn blotch_in(
    scene: &Scene,
    live: &[f64],
    reference: &[f64],
    sigma_m: f64,
    region: Option<&[bool]>,
) -> f64 {
    let s = sigma_m / scene.pixel_size;
    let mean_ref = mean_valid(scene, reference);
    let (mut sum, mut count) = (0.0, 0usize);
    for pa in &scene.patches {
        let n = pa.w * pa.h;
        let range = pa.first..pa.first + n;
        let mask: Vec<bool> = scene.pixels[range.clone()]
            .iter()
            .map(|p| p.valid)
            .collect();
        if !mask.iter().any(|&b| b) {
            continue;
        }
        let e: Vec<f64> = range.clone().map(|i| live[i] - reference[i]).collect();
        let fine = blur(&e, &mask, pa.w, pa.h, s);
        let coarse = blur(&e, &mask, pa.w, pa.h, 6.0 * s);
        let base = blur(&reference[range], &mask, pa.w, pa.h, s);
        for i in 0..n {
            if mask[i] && region.is_none_or(|r| r[pa.first + i]) {
                sum += (fine[i] - coarse[i]).abs() / (base[i] + 0.05 * mean_ref);
                count += 1;
            }
        }
    }
    sum / fl(count.max(1))
}

/// Mean absolute relative error (floored at 5% of the mean reference).
#[must_use]
pub fn mean_rel_error(scene: &Scene, live: &[f64], reference: &[f64]) -> f64 {
    let floor = 0.05 * mean_valid(scene, reference);
    let (mut s, mut n) = (0.0, 0usize);
    for (i, px) in scene.pixels.iter().enumerate() {
        if px.valid {
            s += (live[i] - reference[i]).abs() / (reference[i] + floor);
            n += 1;
        }
    }
    s / fl(n.max(1))
}

/// 99th percentile over valid pixels of the frame-to-frame change relative to
/// the reference (floored at 5% of its mean).
#[must_use]
pub fn pop(scene: &Scene, before: &[f64], after: &[f64], reference: &[f64]) -> f64 {
    let floor = 0.05 * mean_valid(scene, reference);
    let mut v: Vec<f64> = scene
        .pixels
        .iter()
        .enumerate()
        .filter(|(_, p)| p.valid)
        .map(|(i, _)| (after[i] - before[i]).abs() / (reference[i] + floor))
        .collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let idx = floor_usize(0.99 * fl(v.len() - 1));
    v[idx]
}
