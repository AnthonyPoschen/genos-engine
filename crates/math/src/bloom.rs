//! Energy-conserving dual-filter bloom (ADR 0028, proposed).
//!
//! The kernel is the Jorge Jimenez downsample / upsample chain (SIGGRAPH 2014):
//! a 13-tap downsample whose weights sum to 1, then a 3×3 tent upsample whose
//! weights sum to 1. The chained sum is divided by the level count and, unless
//! Karis is on, scaled so the blur's sum matches the scene. The result replaces
//! a `scatter` fraction of the scene, so a flat field is unchanged.
//!
//! This module is not called from the renderer. The shipped pass remains
//! `crates/render/src/hdr_output.rs`. Samples outside the image repeat the edge
//! texel. The shipped reference returns 0 there.

use crate::tonemap::karis_weight;

/// Downsample taps. Offsets are source texels. The weights sum to 1.
const DOWNSAMPLE_TAPS: [(i32, i32, f64); 13] = [
    (0, 0, 0.125),
    (-1, 1, 0.125),
    (1, 1, 0.125),
    (-1, -1, 0.125),
    (1, -1, 0.125),
    (0, 2, 0.0625),
    (0, -2, 0.0625),
    (-2, 0, 0.0625),
    (2, 0, 0.0625),
    (-2, 2, 0.03125),
    (2, 2, 0.03125),
    (-2, -2, 0.03125),
    (2, -2, 0.03125),
];

/// RGBA16F bytes per pixel. The traffic model counts one of these per texel.
const BYTES_PER_PIXEL: u64 = 8;

/// A caller cannot ask for more downsample steps than this.
const MAX_LEVELS: u32 = 16;

/// Largest dump [`from_dump`] will allocate, in pixels. 1440p.
const PARSE_MAX_PIXELS: usize = 2_560 * 1_440;

/// RTX 4070 memory bandwidth, in bytes per second. The lighting notes use 504 GB/s.
pub const RTX_4070_BYTES_PER_SEC: f64 = 504.0e9;

/// GTX 1060 6 GB memory bandwidth, in bytes per second (192 GB/s).
pub const GTX_1060_BYTES_PER_SEC: f64 = 192.0e9;

/// Bloom ceiling on the 4070 at 1440p, in milliseconds. The paper floor sits under it.
pub const BUDGET_4070_MS: f64 = 0.30;

/// Bloom ceiling on a GTX 1060 at 1440p, in milliseconds.
pub const BUDGET_1060_MS: f64 = 1.00;

/// Low. Four levels, a small scatter. The 1060 ceiling is [`BUDGET_1060_MS`] at 1440p.
pub const LOW: Params = Params {
    levels: 4,
    scatter: 0.04,
    karis: false,
};

/// Medium. Five levels.
pub const MEDIUM: Params = Params {
    levels: 5,
    scatter: 0.05,
    karis: false,
};

/// High. Six levels. The 4070 ceiling is [`BUDGET_4070_MS`] at 1440p.
pub const HIGH: Params = Params {
    levels: 6,
    scatter: 0.05,
    karis: false,
};

/// Ultra. Seven levels, the extra level from ADR 0027.
pub const ULTRA: Params = Params {
    levels: 7,
    scatter: 0.06,
    karis: false,
};

/// Packed RGB, row-major, three `f64` channels per pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<f64>,
}

/// Knobs for one chain. `scatter` is clamped to 0..=1. `karis` skips the energy restore.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub levels: u32,
    pub scatter: f64,
    pub karis: bool,
}

impl Params {
    /// Linear chain. Karis is off, so the sum is restored.
    #[must_use]
    pub const fn new(levels: u32, scatter: f64) -> Self {
        Self {
            levels,
            scatter,
            karis: false,
        }
    }

    /// Same knobs with the Karis weight on the first downsample.
    #[must_use]
    pub const fn with_karis(self) -> Self {
        Self {
            karis: true,
            ..self
        }
    }
}

/// Absolute and mean error of a candidate against the reference.
///
/// `candidate` is packed RGB (`width * height * 3`) or RGBA (`* 4`, alpha ignored),
/// the layouts a GPU readback stores. `f32` values are promoted with [`f64::from`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Diff {
    pub max_abs: f64,
    pub mean_abs: f64,
    /// `|sum(candidate) − sum(reference)| / max(|sum(reference)|, 1e−6)`.
    pub energy_rel: f64,
    pub size_mismatch: bool,
}

/// Sum of the 13 downsample weights. A correct table returns 1.
#[must_use]
pub fn downsample_weight_sum() -> f64 {
    DOWNSAMPLE_TAPS.iter().fold(0.0, |sum, tap| sum + tap.2)
}

/// Sum of the 3×3 tent. A correct tent returns 1.
#[must_use]
pub const fn upsample_weight_sum() -> f64 {
    let mut sum = 0.0;
    let mut dy = -1;
    while dy <= 1 {
        let mut dx = -1;
        while dx <= 1 {
            sum += tent_weight(dx, dy);
            dx += 1;
        }
        dy += 1;
    }
    sum
}

/// Bytes moved at `width`×`height` for `levels` steps.
///
/// RGBA16F. Each downsample reads its source once and writes the half image.
/// Each upsample reads the coarse image and the finer level and writes the finer
/// level. The last step reads the half-resolution blur and the scene and writes
/// the output. Tap neighborhoods are counted once: they are assumed to hit cache.
/// A side of 0, or zero levels, is 0 bytes.
#[must_use]
pub fn pyramid_traffic_bytes(width: usize, height: usize, levels: u32) -> u64 {
    let sizes = level_pixels(width, height, levels);
    let steps = sizes.len().saturating_sub(1);
    if steps == 0 {
        return 0;
    }
    let mut total = 0_u64;
    for index in 1..=steps {
        let src = sizes.get(index.saturating_sub(1)).copied().unwrap_or(0);
        let dst = sizes.get(index).copied().unwrap_or(0);
        total = total.saturating_add(src.saturating_add(dst).saturating_mul(BYTES_PER_PIXEL));
    }
    for index in (1..steps).rev() {
        let coarse = sizes.get(index.saturating_add(1)).copied().unwrap_or(0);
        let finer = sizes.get(index).copied().unwrap_or(0);
        let bytes = coarse
            .saturating_add(finer)
            .saturating_add(finer)
            .saturating_mul(BYTES_PER_PIXEL);
        total = total.saturating_add(bytes);
    }
    let full = sizes.first().copied().unwrap_or(0);
    let half = sizes.get(1).copied().unwrap_or(0);
    let last = half
        .saturating_add(full)
        .saturating_add(full)
        .saturating_mul(BYTES_PER_PIXEL);
    total.saturating_add(last)
}

/// Milliseconds to move `bytes` at `bytes_per_sec`. A non-positive rate is infinite.
#[must_use]
pub fn traffic_ms(bytes: u64, bytes_per_sec: f64) -> f64 {
    if !bytes_per_sec.is_finite() || bytes_per_sec <= 0.0 {
        return f64::INFINITY;
    }
    u64_to_f64(bytes) / bytes_per_sec * 1_000.0
}

/// Sum of every finite sample. Non-finite samples add nothing.
#[must_use]
pub fn total(image: &Image) -> f64 {
    let mut sum = 0.0;
    for value in &image.rgb {
        if value.is_finite() {
            sum += *value;
        }
    }
    sum
}

/// `sum(scene) / sum(raw blur)` before the restore. `1` when the raw blur is empty.
///
/// Karis does not change this report. [`bloom`] applies the scale only when Karis is off.
#[must_use]
pub fn filter_gain(image: &Image, params: Params) -> f64 {
    chain(image, params).gain
}

/// The blur that [`bloom`] mixes in. Karis leaves the raw blur. The linear chain
/// multiplies by [`filter_gain`].
#[must_use]
pub fn blur(image: &Image, params: Params) -> Image {
    let built = chain(image, params);
    if params.karis {
        built.raw
    } else {
        scale(&built.raw, built.gain)
    }
}

/// Scene with `scatter` of it replaced by [`blur`].
///
/// Zero levels, a non-positive scatter, or an empty image returns a copy of `image`.
#[must_use]
pub fn bloom(image: &Image, params: Params) -> Image {
    chain(image, params).output
}

/// `candidate` against `reference`. A length that is neither RGB nor RGBA sets
/// [`Diff::size_mismatch`] and infinite errors.
#[must_use]
pub fn compare(reference: &Image, candidate: &[f32]) -> Diff {
    let pixels = reference.width.saturating_mul(reference.height);
    let stride = if candidate.len() == pixels.saturating_mul(3) {
        3
    } else if candidate.len() == pixels.saturating_mul(4) {
        4
    } else {
        return mismatch();
    };
    if pixels == 0 {
        return if candidate.is_empty() {
            Diff {
                max_abs: 0.0,
                mean_abs: 0.0,
                energy_rel: 0.0,
                size_mismatch: false,
            }
        } else {
            mismatch()
        };
    }
    let mut max_abs = 0.0;
    let mut sum_abs = 0.0;
    let mut ref_sum = 0.0;
    let mut got_sum = 0.0;
    let mut count = 0.0;
    for index in 0..pixels {
        let base = index.saturating_mul(3);
        let cand = index.saturating_mul(stride);
        for channel in 0..3 {
            let want = finite_at(&reference.rgb, base.saturating_add(channel));
            let got = match candidate.get(cand.saturating_add(channel)).copied() {
                Some(value) => f64::from(value),
                None => 0.0,
            };
            let err = if got.is_finite() {
                (got - want).abs()
            } else {
                f64::INFINITY
            };
            if err > max_abs {
                max_abs = err;
            }
            sum_abs += err;
            ref_sum += want;
            if got.is_finite() {
                got_sum += got;
            }
            count += 1.0;
        }
    }
    let mean_abs = if count == 0.0 { 0.0 } else { sum_abs / count };
    let energy_rel = (got_sum - ref_sum).abs() / ref_sum.abs().max(1.0e-6);
    Diff {
        max_abs,
        mean_abs,
        energy_rel,
        size_mismatch: false,
    }
}

/// Text form of an image. [`from_dump`] reads it back.
#[must_use]
pub fn to_dump(image: &Image) -> String {
    use std::fmt::Write;
    let mut text = String::new();
    let _ = writeln!(text, "genos-bloom 1");
    let _ = writeln!(text, "{} {}", image.width, image.height);
    let pixels = image.width.saturating_mul(image.height);
    for index in 0..pixels {
        let base = index.saturating_mul(3);
        let r = finite_at(&image.rgb, base);
        let g = finite_at(&image.rgb, base.saturating_add(1));
        let b = finite_at(&image.rgb, base.saturating_add(2));
        let _ = writeln!(text, "{r:?} {g:?} {b:?}");
    }
    text
}

/// Inverse of [`to_dump`]. `None` when the text is not that form, or the image
/// is larger than 1440p.
#[must_use]
pub fn from_dump(text: &str) -> Option<Image> {
    let mut lines = text.lines();
    if lines.next()? != "genos-bloom 1" {
        return None;
    }
    let mut dims = lines.next()?.split_whitespace();
    let width: usize = dims.next()?.parse().ok()?;
    let height: usize = dims.next()?.parse().ok()?;
    if dims.next().is_some() {
        return None;
    }
    let pixels = width.checked_mul(height)?;
    if pixels > PARSE_MAX_PIXELS {
        return None;
    }
    let mut rgb = Vec::with_capacity(pixels.saturating_mul(3));
    for _ in 0..pixels {
        let mut channels = lines.next()?.split_whitespace();
        rgb.push(channels.next()?.parse().ok()?);
        rgb.push(channels.next()?.parse().ok()?);
        rgb.push(channels.next()?.parse().ok()?);
        if channels.next().is_some() {
            return None;
        }
    }
    if lines.next().is_some() {
        return None;
    }
    Some(Image { width, height, rgb })
}

/// Fixed 8×8 input the GPU harness diffs against.
#[must_use]
pub fn fixture_input() -> Image {
    let width: usize = 8;
    let height: usize = 8;
    let mut rgb = Vec::with_capacity(width.saturating_mul(height).saturating_mul(3));
    for y in 0..height {
        for x in 0..width {
            let bright = x == 3 && y == 3;
            rgb.push(if bright { 12.0 } else { 0.04 });
            rgb.push(if bright { 8.0 } else { 0.03 });
            rgb.push(if bright { 2.0 } else { 0.02 });
        }
    }
    Image { width, height, rgb }
}

/// Knobs for [`fixture_input`]. Three levels, scatter 0.05, Karis off.
#[must_use]
pub const fn fixture_params() -> Params {
    Params::new(3, 0.05)
}

/// [`bloom`] of [`fixture_input`] at [`fixture_params`].
#[must_use]
pub fn fixture_output() -> Image {
    bloom(&fixture_input(), fixture_params())
}

struct Chain {
    raw: Image,
    gain: f64,
    output: Image,
}

fn chain(image: &Image, params: Params) -> Chain {
    let scatter = clamp_unit(params.scatter);
    let levels = params.levels.min(MAX_LEVELS);
    if levels == 0 || scatter == 0.0 || image.width == 0 || image.height == 0 {
        return Chain {
            raw: image.clone(),
            gain: 1.0,
            output: image.clone(),
        };
    }
    let mips = build_mips(image, levels, params.karis);
    let Some(raw) = reconstruct(image, &mips) else {
        return Chain {
            raw: image.clone(),
            gain: 1.0,
            output: image.clone(),
        };
    };
    let gain = gain_of(image, &raw);
    let mixed_blur = if params.karis {
        raw.clone()
    } else {
        scale(&raw, gain)
    };
    let output = mix(image, &mixed_blur, scatter);
    Chain { raw, gain, output }
}

fn gain_of(scene: &Image, raw: &Image) -> f64 {
    let got = total(raw);
    if !got.is_finite() || got.abs() < 1.0e-18 {
        1.0
    } else {
        total(scene) / got
    }
}

fn build_mips(image: &Image, levels: u32, karis: bool) -> Vec<Image> {
    let mut mips = Vec::new();
    let mut current = image.clone();
    let steps = usize::try_from(levels).unwrap_or(0);
    for step in 0..steps {
        if current.width <= 1 && current.height <= 1 {
            break;
        }
        let next = if step == 0 && karis {
            downsample_karis(&current)
        } else {
            downsample(&current)
        };
        if next.width >= current.width && next.height >= current.height {
            break;
        }
        mips.push(next);
        if let Some(last) = mips.last() {
            current = last.clone();
        }
    }
    mips
}

fn reconstruct(full: &Image, mips: &[Image]) -> Option<Image> {
    let count = mips.len();
    if count == 0 {
        return None;
    }
    let mut acc = mips.last()?.clone();
    if count > 1 {
        for index in (0..count.saturating_sub(1)).rev() {
            let finer = mips.get(index)?;
            let up = upsample(&acc, finer.width, finer.height);
            acc = add(&up, finer);
        }
    }
    let up = upsample(&acc, full.width, full.height);
    Some(scale(&up, 1.0 / usize_to_f64(count)))
}

fn mix(scene: &Image, blurred: &Image, scatter: f64) -> Image {
    let keep = 1.0 - scatter;
    let pixels = scene.width.saturating_mul(scene.height);
    let mut rgb = Vec::with_capacity(pixels.saturating_mul(3));
    for index in 0..pixels.saturating_mul(3) {
        let sample = finite_at(&scene.rgb, index);
        let wide = finite_at(&blurred.rgb, index);
        rgb.push(keep * sample + scatter * wide);
    }
    Image {
        width: scene.width,
        height: scene.height,
        rgb,
    }
}

fn scale(image: &Image, factor: f64) -> Image {
    let mut rgb = Vec::with_capacity(image.rgb.len());
    for value in &image.rgb {
        let sample = if value.is_finite() { *value } else { 0.0 };
        rgb.push(sample * factor);
    }
    Image {
        width: image.width,
        height: image.height,
        rgb,
    }
}

fn add(left: &Image, right: &Image) -> Image {
    let pixels = left.width.saturating_mul(left.height);
    let mut rgb = Vec::with_capacity(pixels.saturating_mul(3));
    for index in 0..pixels.saturating_mul(3) {
        rgb.push(finite_at(&left.rgb, index) + finite_at(&right.rgb, index));
    }
    Image {
        width: left.width,
        height: left.height,
        rgb,
    }
}

fn downsample(image: &Image) -> Image {
    let dst_w = (image.width / 2).max(1);
    let dst_h = (image.height / 2).max(1);
    let mut rgb = vec![0.0; dst_w.saturating_mul(dst_h).saturating_mul(3)];
    for y in 0..dst_h {
        for x in 0..dst_w {
            let u = (usize_to_f64(x) + 0.5) / usize_to_f64(dst_w.max(1));
            let v = (usize_to_f64(y) + 0.5) / usize_to_f64(dst_h.max(1));
            let mut acc = [0.0; 3];
            for (offset_x, offset_y, weight) in DOWNSAMPLE_TAPS {
                let su = u + f64::from(offset_x) / usize_to_f64(image.width.max(1));
                let sv = v + f64::from(offset_y) / usize_to_f64(image.height.max(1));
                let sample = bilinear(image, su, sv);
                acc[0] += sample[0] * weight;
                acc[1] += sample[1] * weight;
                acc[2] += sample[2] * weight;
            }
            put(&mut rgb, dst_w, x, y, acc);
        }
    }
    Image {
        width: dst_w,
        height: dst_h,
        rgb,
    }
}

fn downsample_karis(image: &Image) -> Image {
    let dst_w = (image.width / 2).max(1);
    let dst_h = (image.height / 2).max(1);
    let mut rgb = vec![0.0; dst_w.saturating_mul(dst_h).saturating_mul(3)];
    for y in 0..dst_h {
        for x in 0..dst_w {
            let ox = i32_from_usize(x.saturating_mul(2));
            let oy = i32_from_usize(y.saturating_mul(2));
            let mut acc = [0.0; 3];
            let mut sum_w = 0.0;
            weighted_box(&mut acc, &mut sum_w, image, ox, oy, 0.5);
            weighted_box(&mut acc, &mut sum_w, image, ox.saturating_add(2), oy, 0.125);
            weighted_box(&mut acc, &mut sum_w, image, ox.saturating_sub(2), oy, 0.125);
            weighted_box(&mut acc, &mut sum_w, image, ox, oy.saturating_add(2), 0.125);
            weighted_box(&mut acc, &mut sum_w, image, ox, oy.saturating_sub(2), 0.125);
            if sum_w > 0.0 {
                acc[0] /= sum_w;
                acc[1] /= sum_w;
                acc[2] /= sum_w;
            }
            put(&mut rgb, dst_w, x, y, acc);
        }
    }
    Image {
        width: dst_w,
        height: dst_h,
        rgb,
    }
}

fn weighted_box(acc: &mut [f64; 3], sum_w: &mut f64, image: &Image, x: i32, y: i32, base: f64) {
    let color = box2(image, x, y);
    let weight = base * karis_weight(rec709_y(color));
    *sum_w += weight;
    acc[0] += color[0] * weight;
    acc[1] += color[1] * weight;
    acc[2] += color[2] * weight;
}

fn box2(image: &Image, x: i32, y: i32) -> [f64; 3] {
    let mut acc = [0.0; 3];
    for dy in [0_i32, 1] {
        for dx in [0_i32, 1] {
            let sample = at_rgb(image, x.saturating_add(dx), y.saturating_add(dy));
            acc[0] += sample[0];
            acc[1] += sample[1];
            acc[2] += sample[2];
        }
    }
    [acc[0] / 4.0, acc[1] / 4.0, acc[2] / 4.0]
}

fn upsample(image: &Image, dst_w: usize, dst_h: usize) -> Image {
    let mut rgb = vec![0.0; dst_w.saturating_mul(dst_h).saturating_mul(3)];
    let src_w = usize_to_f64(image.width.max(1));
    let src_h = usize_to_f64(image.height.max(1));
    for y in 0..dst_h {
        for x in 0..dst_w {
            let u = (usize_to_f64(x) + 0.5) / usize_to_f64(dst_w.max(1));
            let v = (usize_to_f64(y) + 0.5) / usize_to_f64(dst_h.max(1));
            let mut acc = [0.0; 3];
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let weight = tent_weight(dx, dy);
                    let su = u + f64::from(dx) / src_w;
                    let sv = v + f64::from(dy) / src_h;
                    let sample = bilinear(image, su, sv);
                    acc[0] += sample[0] * weight;
                    acc[1] += sample[1] * weight;
                    acc[2] += sample[2] * weight;
                }
            }
            put(&mut rgb, dst_w, x, y, acc);
        }
    }
    Image {
        width: dst_w,
        height: dst_h,
        rgb,
    }
}

const fn tent_weight(dx: i32, dy: i32) -> f64 {
    if dx == 0 && dy == 0 {
        0.25
    } else if dx == 0 || dy == 0 {
        0.125
    } else {
        0.0625
    }
}

fn bilinear(image: &Image, u: f64, v: f64) -> [f64; 3] {
    if image.width == 0 || image.height == 0 {
        return [0.0; 3];
    }
    let px = u * usize_to_f64(image.width) - 0.5;
    let py = v * usize_to_f64(image.height) - 0.5;
    let x0 = floor_i32(px);
    let y0 = floor_i32(py);
    let tx = px - f64::from(x0);
    let ty = py - f64::from(y0);
    let s00 = at_rgb(image, x0, y0);
    let s10 = at_rgb(image, x0.saturating_add(1), y0);
    let s01 = at_rgb(image, x0, y0.saturating_add(1));
    let s11 = at_rgb(image, x0.saturating_add(1), y0.saturating_add(1));
    let mut out = [0.0; 3];
    for channel in 0..3 {
        let top = s00[channel] * (1.0 - tx) + s10[channel] * tx;
        let bottom = s01[channel] * (1.0 - tx) + s11[channel] * tx;
        out[channel] = top * (1.0 - ty) + bottom * ty;
    }
    out
}

fn at_rgb(image: &Image, x: i32, y: i32) -> [f64; 3] {
    if image.width == 0 || image.height == 0 {
        return [0.0; 3];
    }
    let xu = clamp_coord(x, image.width);
    let yu = clamp_coord(y, image.height);
    let index = yu
        .saturating_mul(image.width)
        .saturating_add(xu)
        .saturating_mul(3);
    [
        finite_at(&image.rgb, index),
        finite_at(&image.rgb, index.saturating_add(1)),
        finite_at(&image.rgb, index.saturating_add(2)),
    ]
}

fn put(rgb: &mut [f64], width: usize, x: usize, y: usize, sample: [f64; 3]) {
    let index = y.saturating_mul(width).saturating_add(x).saturating_mul(3);
    if let Some(slot) = rgb.get_mut(index) {
        *slot = sample[0];
    }
    if let Some(slot) = rgb.get_mut(index.saturating_add(1)) {
        *slot = sample[1];
    }
    if let Some(slot) = rgb.get_mut(index.saturating_add(2)) {
        *slot = sample[2];
    }
}

fn rec709_y(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

fn finite_at(src: &[f64], index: usize) -> f64 {
    match src.get(index).copied() {
        Some(value) if value.is_finite() => value,
        _ => 0.0,
    }
}

fn clamp_coord(value: i32, limit: usize) -> usize {
    if value <= 0 || limit == 0 {
        return 0;
    }
    match usize::try_from(value) {
        Ok(index) if index < limit => index,
        _ => limit.saturating_sub(1),
    }
}

fn clamp_unit(value: f64) -> f64 {
    if !value.is_finite() || value <= 0.0 {
        0.0
    } else if value >= 1.0 {
        1.0
    } else {
        value
    }
}

fn level_pixels(width: usize, height: usize, levels: u32) -> Vec<u64> {
    let mut sizes = Vec::new();
    if width == 0 || height == 0 || levels == 0 {
        return sizes;
    }
    sizes.push(pixel_count(width, height));
    let mut w = width;
    let mut h = height;
    let steps = usize::try_from(levels.min(MAX_LEVELS)).unwrap_or(0);
    for _ in 0..steps {
        let next_w = w / 2;
        let next_h = h / 2;
        if next_w == 0 || next_h == 0 {
            break;
        }
        w = next_w;
        h = next_h;
        sizes.push(pixel_count(w, h));
    }
    sizes
}

fn pixel_count(width: usize, height: usize) -> u64 {
    u64::try_from(width)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(height).unwrap_or(u64::MAX))
}

fn usize_to_f64(value: usize) -> f64 {
    match u32::try_from(value) {
        Ok(narrow) => f64::from(narrow),
        Err(_) => f64::from(u32::MAX),
    }
}

fn u64_to_f64(value: u64) -> f64 {
    if let Ok(narrow) = u32::try_from(value) {
        return f64::from(narrow);
    }
    let hi = u32::try_from(value >> 32).unwrap_or(u32::MAX);
    let lo = u32::try_from(value & 0xffff_ffff).unwrap_or(0);
    f64::from(hi) * 4_294_967_296.0 + f64::from(lo)
}

fn i32_from_usize(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Floor of a finite value, clamped to `i32`. `NaN` is 0.
fn floor_i32(value: f64) -> i32 {
    if value.is_nan() || value <= f64::from(i32::MIN) {
        return if value.is_nan() { 0 } else { i32::MIN };
    }
    if value >= f64::from(i32::MAX) {
        return i32::MAX;
    }
    let bits = value.to_bits();
    let biased = (bits >> 52) & 0x7ff;
    let exp = i32::from(u16::try_from(biased).unwrap_or(0)) - 1023;
    if exp < 0 {
        return if value < 0.0 { -1 } else { 0 };
    }
    let frac = bits & 0x000f_ffff_ffff_ffff;
    let sig = (1_u64 << 52) | frac;
    let shift = u32::try_from(52 - exp).unwrap_or(52);
    let mag = sig >> shift;
    let frac_bits = sig & ((1_u64 << shift) - 1);
    let mag_i = i32::try_from(mag).unwrap_or(i32::MAX);
    if bits >> 63 == 0 {
        mag_i
    } else if frac_bits == 0 {
        mag_i.saturating_neg()
    } else {
        mag_i.saturating_neg().saturating_sub(1)
    }
}

fn mismatch() -> Diff {
    Diff {
        max_abs: f64::INFINITY,
        mean_abs: f64::INFINITY,
        energy_rel: f64::INFINITY,
        size_mismatch: true,
    }
}

#[cfg(test)]
mod tests {
    use super::floor_i32;

    #[test]
    fn floor_matches_known_values() {
        assert_eq!(floor_i32(-1.5), -2);
        assert_eq!(floor_i32(-1.0), -1);
        assert_eq!(floor_i32(-0.1), -1);
        assert_eq!(floor_i32(0.0), 0);
        assert_eq!(floor_i32(-0.0), 0);
        assert_eq!(floor_i32(0.9), 0);
        assert_eq!(floor_i32(1.0), 1);
        assert_eq!(floor_i32(1.5), 1);
        assert_eq!(floor_i32(2.5), 2);
        assert_eq!(floor_i32(2_559.7), 2_559);
        assert_eq!(floor_i32(f64::NAN), 0);
        assert_eq!(floor_i32(f64::INFINITY), i32::MAX);
        assert_eq!(floor_i32(f64::NEG_INFINITY), i32::MIN);
    }
}
