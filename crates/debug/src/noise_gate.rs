#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! s7 lamp-noise grade. Pure measurements: no window and no GPU.
//!
//! Luminance is the tone curve undone (`linear_from_display`), the same
//! quantity [`crate::reference::compare`] uses. A pop is a pixel-frame whose
//! value steps more than [`POP_STEP`] from its centred 9-frame average, the
//! fill-run definition in `docs/systems/lighting/README.md`.

use genos_render::linear_from_display;

use crate::image::Image;

/// Still-flicker pass line: worst 1% of temporal standard deviation, overlap only.
pub const FLICKER_P99: f32 = 1.0e-3;
/// Fixed-pattern error may be this many times the exact-sum error.
pub const FIXED_RATIO: f32 = 1.1;
/// Pan pops may be this many times the exact-sum pop share.
pub const POP_RATIO: f32 = 1.25;
/// Path-trace `mean_rel` may be this many times the exact-sum error.
pub const REL_RATIO: f32 = 1.05;
/// A ratio past this, or flicker past [`FATAL_FLICKER`], is grade F.
pub const FATAL_RATIO: f32 = 2.0;
/// Twice the flicker pass line.
pub const FATAL_FLICKER: f32 = 2.0e-3;
/// Centred 9-frame step that counts as a pop.
pub const POP_STEP: f32 = 0.01;
/// A lamp at or under this share of the solo sum is not a contributor.
pub const CONTRIB_SHARE: f32 = 0.02;
/// Overlap is every contributor at or under this share. Dominant is above it.
pub const SHARE_CUT: f32 = 0.6;
/// A pixel darker than this on every solo is unlit.
pub const LIT_FLOOR: f32 = 1.0e-4;
const RATIO_FLOOR: f32 = 1.0e-6;

/// One capture: still frames, the pan, the reference, and per-lamp solos.
#[derive(Clone, Debug)]
pub struct GateSamples {
    pub width: u32,
    pub height: u32,
    /// Linear luminance, one buffer per still frame.
    pub still: Vec<Vec<f32>>,
    /// Linear luminance, one buffer per pan frame, pose-matched to the other capture.
    pub pan: Vec<Vec<f32>>,
    /// Linear luminance of the path-traced direct reference, still pose.
    pub reference: Vec<f32>,
    /// False where the reference's primary ray missed. Empty means every pixel hit.
    pub hit: Vec<bool>,
    /// One linear-luminance buffer per lamp, still pose.
    pub solos: Vec<Vec<f32>>,
    /// Fresh pixel shadow rays on the last still frame. Zero means the hold ate the path.
    pub still_rays: u32,
    /// Pixels that shaded a reservoir sample on the last still frame.
    pub still_restir: u32,
    /// Pixels that shaded a reservoir sample on the last pan frame.
    pub pan_restir: u32,
    /// Pixels that took the exact sum on the last pan frame.
    pub pan_exact: u32,
}

/// What [`grade`] decided.
#[derive(Clone, Debug, PartialEq)]
pub struct GateReport {
    pub grade: char,
    /// False for C and below, and for F.
    pub exit_ok: bool,
    pub flicker_p99: f32,
    pub fixed: f32,
    pub exact_fixed: f32,
    pub fixed_ratio: f32,
    pub pops: f32,
    pub exact_pops: f32,
    pub pop_ratio: f32,
    pub mean_rel: f32,
    pub exact_mean_rel: f32,
    pub rel_ratio: f32,
    pub overlap_pixels: usize,
    pub dominant_pixels: usize,
    pub still_rays: u32,
    pub still_restir: u32,
    pub pan_restir: u32,
    pub pan_exact: u32,
    /// Flicker p99 of each 4×4 cell, row-major. NaN where the cell is empty.
    pub grid_flicker: Vec<f32>,
    pub dominant_flicker: f32,
    pub failures: Vec<String>,
    pub fatal: bool,
}

impl std::fmt::Display for GateReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "s7 grade {} exit {}",
            self.grade,
            i32::from(!self.exit_ok)
        )?;
        writeln!(
            f,
            "flicker_p99 {:.6} (limit {FLICKER_P99:.1e}) overlap {} dominant {} ({:.6})",
            self.flicker_p99, self.overlap_pixels, self.dominant_pixels, self.dominant_flicker
        )?;
        writeln!(
            f,
            "fixed {:.6} / exact {:.6} = {:.4} (limit {FIXED_RATIO})",
            self.fixed, self.exact_fixed, self.fixed_ratio
        )?;
        writeln!(
            f,
            "pops {:.6} / exact {:.6} = {:.4} (limit {POP_RATIO})",
            self.pops, self.exact_pops, self.pop_ratio
        )?;
        writeln!(
            f,
            "mean_rel {:.6} / exact {:.6} = {:.4} (limit {REL_RATIO})",
            self.mean_rel, self.exact_mean_rel, self.rel_ratio
        )?;
        writeln!(
            f,
            "still_rays {} still_restir {} pan_restir {} pan_exact {}",
            self.still_rays, self.still_restir, self.pan_restir, self.pan_exact
        )?;
        for fail in &self.failures {
            writeln!(f, "fail: {fail}")?;
        }
        Ok(())
    }
}

/// Linear luminance of a displayed pixel, the picture's tone curve undone.
#[must_use]
pub fn display_y(pixel: &[u8]) -> f32 {
    let c = [0, 1, 2].map(|i| linear_from_display(f32::from(pixel[i]) / 255.0));
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// One linear luminance per pixel of `image`.
#[must_use]
pub fn frame_luminance(image: &Image) -> Vec<f32> {
    let mut out = Vec::with_capacity(image.rgb.len() / 3);
    let mut i = 0;
    while i + 2 < image.rgb.len() {
        out.push(display_y(&image.rgb[i..i + 3]));
        i += 3;
    }
    out
}

/// Mean of the frames. Empty input is an empty buffer.
#[must_use]
pub fn time_mean(frames: &[Vec<f32>]) -> Vec<f32> {
    let Some(first) = frames.first() else {
        return Vec::new();
    };
    let mut sum = vec![0.0f32; first.len()];
    for frame in frames {
        for (s, v) in sum.iter_mut().zip(frame) {
            *s += *v;
        }
    }
    let n = count_as_f32(frames.len());
    for s in &mut sum {
        *s /= n;
    }
    sum
}

/// Population standard deviation of each pixel across `frames`.
#[must_use]
pub fn temporal_stddev(frames: &[Vec<f32>]) -> Vec<f32> {
    let mean = time_mean(frames);
    if mean.is_empty() || frames.is_empty() {
        return mean;
    }
    let mut var = vec![0.0f32; mean.len()];
    for frame in frames {
        for (i, v) in frame.iter().enumerate() {
            let d = *v - mean[i];
            var[i] += d * d;
        }
    }
    let n = count_as_f32(frames.len());
    var.into_iter().map(|v| (v / n).sqrt()).collect()
}

/// `fraction` through `values` after sorting. Empty is NaN.
#[must_use]
pub fn percentile(values: &mut [f32], fraction: f32) -> f32 {
    if values.is_empty() {
        return f32::NAN;
    }
    values.sort_by(f32::total_cmp);
    let index = rank_index(fraction, values.len());
    values[index]
}

fn masked_percentile(values: &[f32], mask: &[bool], fraction: f32) -> (f32, usize) {
    let mut picked = Vec::new();
    for (v, on) in values.iter().zip(mask) {
        if *on {
            picked.push(*v);
        }
    }
    let n = picked.len();
    (percentile(&mut picked, fraction), n)
}

/// Share of pixel-frames, frames 4..len-4, that step more than [`POP_STEP`]
/// from the centred 9-frame average. Fewer than 9 frames is 0.
#[must_use]
pub fn pop_share(frames: &[Vec<f32>]) -> f32 {
    if frames.len() < 9 {
        return 0.0;
    }
    let mut over = 0u64;
    let mut count = 0u64;
    let last = frames.len() - 4;
    for center_at in 4..last {
        let mut totals = vec![0.0f32; frames[center_at].len()];
        for around in &frames[center_at - 4..=center_at + 4] {
            for (total, sample) in totals.iter_mut().zip(around) {
                *total += *sample;
            }
        }
        for (sample, total) in frames[center_at].iter().zip(&totals) {
            if (sample - total / 9.0).abs() > POP_STEP {
                over += 1;
            }
            count += 1;
        }
    }
    u64_as_f32(over) / u64_as_f32(count.max(1))
}

/// `floor(fraction * (len - 1))`, the percentile rank.
///
/// The search compares `i / (len - 1)` in `f32`. A 1080p frame is under
/// 2^24 pixels, so every index is exact in that type.
fn rank_index(fraction: f32, len: usize) -> usize {
    if len <= 1 {
        return 0;
    }
    let last = len - 1;
    let clamped = fraction.clamp(0.0, 1.0);
    if clamped
        .partial_cmp(&0.0)
        .is_none_or(std::cmp::Ordering::is_le)
    {
        return 0;
    }
    if clamped >= 1.0 {
        return last;
    }
    let mut lo = 0usize;
    let mut hi = last;
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let ratio = count_as_f32(mid) / count_as_f32(last);
        if ratio <= clamped {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

fn count_as_f32(n: usize) -> f32 {
    match u32::try_from(n) {
        Ok(v) => {
            let hi = f32::from(u16::try_from(v >> 16).unwrap_or(0));
            let lo = f32::from(u16::try_from(v & 0xffff).unwrap_or(0));
            hi.mul_add(65536.0, lo)
        }
        Err(_) => f32::MAX,
    }
}

fn u64_as_f32(n: u64) -> f32 {
    match u32::try_from(n) {
        Ok(v) => count_as_f32(usize::try_from(v).unwrap_or(0)),
        Err(_) => f32::MAX,
    }
}

fn at(x: u32, y: u32, width: u32) -> usize {
    let y = usize::try_from(y).unwrap_or(0);
    let width = usize::try_from(width).unwrap_or(0);
    let x = usize::try_from(x).unwrap_or(0);
    y * width + x
}

/// 5×5 box mean. Taps outside the picture are skipped.
#[must_use]
pub fn box5(src: &[f32], width: u32, height: u32) -> Vec<f32> {
    let mut out = vec![0.0f32; src.len()];
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0.0f32;
            let mut n = 0.0f32;
            for dy in -2i32..=2 {
                for dx in -2i32..=2 {
                    let xx = i32::try_from(x).unwrap_or(0) + dx;
                    let yy = i32::try_from(y).unwrap_or(0) + dy;
                    if xx < 0 || yy < 0 {
                        continue;
                    }
                    let (Ok(xu), Ok(yu)) = (u32::try_from(xx), u32::try_from(yy)) else {
                        continue;
                    };
                    if xu >= width || yu >= height {
                        continue;
                    }
                    sum += src[at(xu, yu, width)];
                    n += 1.0;
                }
            }
            out[at(x, y, width)] = sum / n.max(1.0);
        }
    }
    out
}

fn hit_on(hit: &[bool], index: usize) -> bool {
    hit.get(index).copied().unwrap_or(true)
}

/// Mean absolute high-pass (5×5 box removed) of `mean - reference`, over hit pixels.
#[must_use]
pub fn fixed_pattern(
    mean: &[f32],
    reference: &[f32],
    hit: &[bool],
    width: u32,
    height: u32,
) -> f32 {
    let mut diff = vec![0.0f32; mean.len()];
    for (index, (slot, sample)) in diff.iter_mut().zip(mean).enumerate() {
        if hit_on(hit, index) {
            let reference_y = reference.get(index).copied().unwrap_or(0.0);
            *slot = sample - reference_y;
        }
    }
    let low = box5(&diff, width, height);
    let mut sum = 0.0f64;
    let mut n = 0.0f64;
    for (index, (sample, smooth)) in diff.iter().zip(&low).enumerate() {
        if hit_on(hit, index) {
            sum += f64::from((sample - smooth).abs());
            n += 1.0;
        }
    }
    f32_from_f64(sum / n.max(1.0))
}

/// Sum of |live − reference| over the sum of the reference, hit pixels only.
#[must_use]
pub fn mean_rel(live: &[f32], reference: &[f32], hit: &[bool]) -> f32 {
    let mut abs = 0.0f64;
    let mut sum = 0.0f64;
    for (index, sample) in live.iter().enumerate() {
        if !hit_on(hit, index) {
            continue;
        }
        let reference_y = reference.get(index).copied().unwrap_or(0.0);
        abs += f64::from((sample - reference_y).abs());
        sum += f64::from(reference_y);
    }
    f32_from_f64(abs / sum.max(1.0e-9))
}

/// `ours / master`. Both under [`RATIO_FLOOR`] is 1, so two exact matches stay even.
#[must_use]
pub fn error_ratio(ours: f32, master: f32) -> f32 {
    if ours.abs() < RATIO_FLOOR && master.abs() < RATIO_FLOOR {
        1.0
    } else {
        ours / master.max(RATIO_FLOOR)
    }
}

/// Overlap: at least two contributors and none above [`SHARE_CUT`].
/// Dominant: the brightest solo is at least [`SHARE_CUT`] of the sum.
#[must_use]
pub fn lamp_masks(solos: &[Vec<f32>]) -> (Vec<bool>, Vec<bool>) {
    let pixels = solos.first().map_or(0, Vec::len);
    let mut overlap = vec![false; pixels];
    let mut dominant = vec![false; pixels];
    for (index, overlap_px) in overlap.iter_mut().enumerate() {
        let mut total = 0.0f32;
        for solo in solos {
            total += solo.get(index).copied().unwrap_or(0.0).max(0.0);
        }
        if total <= LIT_FLOOR {
            continue;
        }
        let mut top = 0.0f32;
        let mut contributors = 0u32;
        for solo in solos {
            let share = solo.get(index).copied().unwrap_or(0.0).max(0.0) / total;
            top = top.max(share);
            if share > CONTRIB_SHARE {
                contributors += 1;
            }
        }
        if contributors >= 2 && top <= SHARE_CUT {
            *overlap_px = true;
        }
        if top >= SHARE_CUT {
            dominant[index] = true;
        }
    }
    (overlap, dominant)
}

fn f32_from_f64(v: f64) -> f32 {
    // The only narrowing in this file. The sums are f64 so a 1080p frame
    // does not lose the tail of the mean.
    #[expect(clippy::as_conversions, reason = "f64 to f32 has no TryFrom")]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the gate reports f32; the sum is f64"
    )]
    {
        v as f32
    }
}

fn cell_masks(width: u32, height: u32) -> Vec<Vec<bool>> {
    let width_px = usize::try_from(width).unwrap_or(0);
    let height_px = usize::try_from(height).unwrap_or(0);
    let mut cells = vec![vec![false; width_px * height_px]; 16];
    let w = width.max(1);
    let h = height.max(1);
    for y in 0..height {
        for x in 0..width {
            let cx = (u64::from(x) * 4 / u64::from(w)).min(3);
            let cy = (u64::from(y) * 4 / u64::from(h)).min(3);
            let cell = usize::try_from(cy * 4 + cx).unwrap_or(0);
            cells[cell][at(x, y, width)] = true;
        }
    }
    cells
}

fn letter(failures: usize, fatal: bool) -> char {
    if fatal || failures >= 5 {
        'F'
    } else {
        match failures {
            0 => 'A',
            1 => 'B',
            2 => 'C',
            3 => 'D',
            _ => 'E',
        }
    }
}

fn same_len(label: &str, buf: &[f32], n: usize) -> Result<(), String> {
    if buf.len() == n {
        Ok(())
    } else {
        Err(format!("{label} has {} pixels, expected {n}", buf.len()))
    }
}

/// Grade `live` (the random path) against `exact` (the exact-sum path).
///
/// # Errors
///
/// The picture is empty, the two captures differ in size, a buffer does not
/// cover the picture, or the pan is shorter than 9 frames.
pub fn grade(live: &GateSamples, exact: &GateSamples) -> Result<GateReport, String> {
    let pixels = checked_pixels(live, exact)?;
    let (overlap, dominant) = lamp_masks(&live.solos);
    if overlap.len() != pixels {
        return Err("solos do not cover the picture".to_string());
    }
    let live_dev = temporal_stddev(&live.still);
    let (flicker_p99, overlap_pixels) = masked_percentile(&live_dev, &overlap, 0.99);
    let (dominant_flicker, dominant_pixels) = masked_percentile(&live_dev, &dominant, 0.99);
    let live_mean = time_mean(&live.still);
    let exact_mean = time_mean(&exact.still);
    let hit = &live.hit;
    let fixed = fixed_pattern(&live_mean, &live.reference, hit, live.width, live.height);
    let exact_fixed = fixed_pattern(&exact_mean, &live.reference, hit, live.width, live.height);
    let fixed_ratio = error_ratio(fixed, exact_fixed);
    let pops = pop_share(&live.pan);
    let exact_pops = pop_share(&exact.pan);
    let pop_ratio = error_ratio(pops, exact_pops);
    let rel = mean_rel(&live_mean, &live.reference, hit);
    let exact_rel = mean_rel(&exact_mean, &live.reference, hit);
    let rel_ratio = error_ratio(rel, exact_rel);
    let grid_flicker: Vec<f32> = cell_masks(live.width, live.height)
        .iter()
        .map(|mask| masked_percentile(&live_dev, mask, 0.99).0)
        .collect();
    let (failures, fatal) = gate_failures(
        overlap_pixels,
        flicker_p99,
        fixed_ratio,
        pop_ratio,
        rel_ratio,
        GateRays {
            still_rays: live.still_rays,
            still_restir: live.still_restir,
            pan_restir: live.pan_restir,
        },
    );
    let grade_letter = letter(failures.len(), fatal);
    Ok(GateReport {
        grade: grade_letter,
        exit_ok: grade_letter == 'A' || grade_letter == 'B',
        flicker_p99,
        fixed,
        exact_fixed,
        fixed_ratio,
        pops,
        exact_pops,
        pop_ratio,
        mean_rel: rel,
        exact_mean_rel: exact_rel,
        rel_ratio,
        overlap_pixels,
        dominant_pixels,
        still_rays: live.still_rays,
        still_restir: live.still_restir,
        pan_restir: live.pan_restir,
        pan_exact: live.pan_exact,
        grid_flicker,
        dominant_flicker,
        failures,
        fatal,
    })
}

fn checked_pixels(live: &GateSamples, exact: &GateSamples) -> Result<usize, String> {
    let pixels =
        usize::try_from(live.width).unwrap_or(0) * usize::try_from(live.height).unwrap_or(0);
    if pixels == 0 {
        return Err("the picture has no pixels".to_string());
    }
    if live.width != exact.width || live.height != exact.height {
        return Err("live and exact sizes differ".to_string());
    }
    require_frames("live still", &live.still, pixels)?;
    require_frames("exact still", &exact.still, pixels)?;
    if live.pan.len() < 9 || exact.pan.len() < 9 {
        return Err("the pan needs at least 9 frames".to_string());
    }
    for frame in live.pan.iter().chain(&exact.pan) {
        same_len("pan", frame, pixels)?;
    }
    same_len("reference", &live.reference, pixels)?;
    if !live.hit.is_empty() && live.hit.len() != pixels {
        return Err(format!(
            "hit has {} pixels, expected {pixels}",
            live.hit.len()
        ));
    }
    Ok(pixels)
}

fn require_frames(label: &str, frames: &[Vec<f32>], pixels: usize) -> Result<(), String> {
    if frames.is_empty() {
        return Err(format!("{label} has no frames"));
    }
    for frame in frames {
        same_len(label, frame, pixels)?;
    }
    Ok(())
}

fn past(value: f32, limit: f32) -> bool {
    value
        .partial_cmp(&limit)
        .is_none_or(std::cmp::Ordering::is_gt)
}

#[derive(Clone, Copy)]
struct GateRays {
    still_rays: u32,
    still_restir: u32,
    pan_restir: u32,
}

fn gate_failures(
    overlap_pixels: usize,
    flicker_p99: f32,
    fixed_ratio: f32,
    pop_ratio: f32,
    rel_ratio: f32,
    rays: GateRays,
) -> (Vec<String>, bool) {
    let GateRays {
        still_rays,
        still_restir,
        pan_restir,
    } = rays;
    let mut failures = Vec::new();
    let mut fatal = false;
    if overlap_pixels == 0 {
        failures.push("overlap mask is empty".to_string());
        fatal = true;
    } else if past(flicker_p99, FLICKER_P99) {
        failures.push(format!("overlap flicker p99 {flicker_p99:.6}"));
        fatal |= past(flicker_p99, FATAL_FLICKER);
    }
    if past(fixed_ratio, FIXED_RATIO) {
        failures.push(format!("fixed-pattern ratio {fixed_ratio:.4}"));
        fatal |= past(fixed_ratio, FATAL_RATIO);
    }
    if past(pop_ratio, POP_RATIO) {
        failures.push(format!("pan pop ratio {pop_ratio:.4}"));
        fatal |= past(pop_ratio, FATAL_RATIO);
    }
    if past(rel_ratio, REL_RATIO) {
        failures.push(format!("reference mean_rel ratio {rel_ratio:.4}"));
        fatal |= past(rel_ratio, FATAL_RATIO);
    }
    // The random path has to have run. A hold, or the exact sum, leaves these at 0.
    if still_rays == 0 || still_restir == 0 || pan_restir == 0 {
        failures.push(format!(
            "still rays {still_rays} restir {still_restir} pan restir {pan_restir}"
        ));
    }
    (failures, fatal)
}

/// `metrics.json` for `report`.
#[must_use]
pub fn report_json(report: &GateReport) -> String {
    let grid = report
        .grid_flicker
        .iter()
        .map(|value| {
            if value.is_finite() {
                format!("{value:.6}")
            } else {
                "null".to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let fails = report
        .failures
        .iter()
        .map(|fail| format!("\"{}\"", fail.replace('"', "'")))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{{\n  \"grade\": \"{}\",\n  \"exit_ok\": {},\n  \"flicker_p99\": {:.6},\n  \"fixed\": {:.6},\n  \"exact_fixed\": {:.6},\n  \"fixed_ratio\": {:.6},\n  \"pops\": {:.6},\n  \"exact_pops\": {:.6},\n  \"pop_ratio\": {:.6},\n  \"mean_rel\": {:.6},\n  \"exact_mean_rel\": {:.6},\n  \"rel_ratio\": {:.6},\n  \"overlap_pixels\": {},\n  \"dominant_pixels\": {},\n  \"dominant_flicker\": {:.6},\n  \"still_rays\": {},\n  \"still_restir\": {},\n  \"pan_restir\": {},\n  \"pan_exact\": {},\n  \"grid_flicker\": [{grid}],\n  \"failures\": [{fails}]\n}}\n",
        report.grade,
        report.exit_ok,
        report.flicker_p99,
        report.fixed,
        report.exact_fixed,
        report.fixed_ratio,
        report.pops,
        report.exact_pops,
        report.pop_ratio,
        report.mean_rel,
        report.exact_mean_rel,
        report.rel_ratio,
        report.overlap_pixels,
        report.dominant_pixels,
        report.dominant_flicker,
        report.still_rays,
        report.still_restir,
        report.pan_restir,
        report.pan_exact,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(n: usize, v: f32) -> Vec<f32> {
        vec![v; n]
    }

    fn samples(still_v: f32, rays: u32, restir: u32) -> GateSamples {
        let n = 16;
        let frame = flat(n, still_v);
        let solos = vec![flat(n, 0.4), flat(n, 0.4)];
        GateSamples {
            width: 4,
            height: 4,
            still: vec![frame.clone(); 4],
            pan: vec![frame; 9],
            reference: flat(n, still_v),
            hit: vec![true; n],
            solos,
            still_rays: rays,
            still_restir: restir,
            pan_restir: restir,
            pan_exact: 0,
        }
    }

    #[test]
    fn identical_captures_grade_a() {
        let live = samples(0.2, 4, 4);
        let exact = samples(0.2, 4, 4);
        let report = grade(&live, &exact).unwrap();
        assert_eq!(report.grade, 'A', "{report}");
        assert!(report.exit_ok);
        assert!(report.flicker_p99 <= FLICKER_P99);
        assert!((report.fixed_ratio - 1.0).abs() < 1.0e-5);
        assert!((report.pop_ratio - 1.0).abs() < 1.0e-5);
        assert!((report.rel_ratio - 1.0).abs() < 1.0e-5);
        assert!(report.overlap_pixels > 0);
    }

    #[test]
    fn one_gate_is_b_and_twice_the_line_is_f() {
        let mut live = samples(0.2, 4, 4);
        // Two pixels alternate by 0.0015. On a 16-pixel picture the 99th
        // percentile is the second-worst pixel, so one hot pixel would hide.
        for i in 0..2 {
            live.still[0][i] = 0.2015;
            live.still[1][i] = 0.1985;
            live.still[2][i] = 0.2015;
            live.still[3][i] = 0.1985;
        }
        let exact = samples(0.2, 4, 4);
        let report = grade(&live, &exact).unwrap();
        assert!(
            report.flicker_p99 > FLICKER_P99 && report.flicker_p99 <= FATAL_FLICKER,
            "{report}"
        );
        assert_eq!(report.grade, 'B', "{report}");
        assert!(report.exit_ok);

        for v in &mut live.still[0] {
            *v += 0.01;
        }
        let report = grade(&live, &exact).unwrap();
        assert!(report.flicker_p99 > FATAL_FLICKER, "{report}");
        assert_eq!(report.grade, 'F', "{report}");
        assert!(!report.exit_ok);
    }

    #[test]
    fn an_empty_overlap_is_f() {
        let mut live = samples(0.2, 4, 4);
        live.solos = vec![flat(16, 1.0), flat(16, 0.0)];
        let exact = samples(0.2, 4, 4);
        let report = grade(&live, &exact).unwrap();
        assert_eq!(report.overlap_pixels, 0);
        assert_eq!(report.grade, 'F');
    }

    #[test]
    fn a_ratio_over_two_is_f_and_a_small_pair_is_even() {
        assert!((error_ratio(1.0e-8, 1.0e-8) - 1.0).abs() < 1.0e-6);
        let mut live = samples(0.2, 4, 4);
        live.reference = flat(16, 0.05);
        let mut exact = samples(0.2, 4, 4);
        exact.still = vec![flat(16, 0.06); 4];
        exact.reference.clone_from(&live.reference);
        // live sits 0.15 off the reference; the exact sum sits 0.01 off. The ratio is past 2.
        let report = grade(&live, &exact).unwrap();
        assert!(report.rel_ratio > FATAL_RATIO, "{report}");
        assert_eq!(report.grade, 'F', "{report}");
    }

    #[test]
    fn a_centred_ramp_does_not_pop_and_a_spike_does() {
        let mut frames = Vec::new();
        for i in 0..12 {
            frames.push(vec![count_as_f32(i) * 0.001]);
        }
        assert_eq!(pop_share(&frames), 0.0);
        frames[6][0] += 0.05;
        assert!(pop_share(&frames) > 0.0);
    }

    #[test]
    fn a_flat_bias_has_no_fixed_pattern_and_a_checker_does() {
        let mean = flat(16, 0.3);
        let reference = flat(16, 0.2);
        let hit = vec![true; 16];
        let flat_err = fixed_pattern(&mean, &reference, &hit, 4, 4);
        assert!(flat_err < 1.0e-6, "{flat_err}");
        let mut checker = mean;
        for (index, pixel) in checker.iter_mut().enumerate() {
            if index % 2 == 0 {
                *pixel = 0.5;
            }
        }
        let grain = fixed_pattern(&checker, &reference, &hit, 4, 4);
        assert!(grain > flat_err + 0.01, "{grain}");
    }

    #[test]
    fn unset_rays_fail_the_settle_check() {
        let live = samples(0.2, 0, 0);
        let exact = samples(0.2, 4, 4);
        let report = grade(&live, &exact).unwrap();
        assert_eq!(report.grade, 'B', "{report}");
        assert!(report.failures.iter().any(|f| f.contains("still rays")));
    }
}
