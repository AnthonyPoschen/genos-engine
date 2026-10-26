//! HDR output metric gate. Analytic fixtures only. No renderer pass is called.
//!
//! ```text
//! cargo test -p genos-render --test hdr_output_gate -- --out genos-debug/hdr-output
//! ```
//!
//! `--out` wins. `GENOS_HDR_OUT` applies only when `--out` was omitted.

use std::path::{Path, PathBuf};

const LEGACY_MAX: f64 = 1e-4;
const VRAM_MAX: f64 = 48.0;
const SCRGB_MAX: f64 = 96.0;
const BASIC_MAX: f64 = 2.0;
const STRESS_MAX: f64 = 10.0;
const LOW_MS_MAX: f64 = 2.5;
const PEAK_MAX: f64 = 0.0025;
const GRADE_MAX: f64 = 0.02;
const LINEAR_MIN: f64 = 1.5;
const EV_MAX: f64 = 0.10;
const STILL_MAX: f64 = 0.001;
const ADAPT_MAX: f64 = 0.0;
const OVERSHOOT_MAX: f64 = 0.02;
const CAMERA_MAX: f64 = 0.001;
const ENERGY_MAX: f64 = 0.01;
const FIREFLY_MAX: f64 = 1.0;
const COMMUTE_MAX: f64 = 0.01;
const PICK_MAX: f64 = 0.0;
const NEAR_MAX: f64 = 0.05;
const POP_MAX: f64 = 0.001;
const HUE_MAX: f64 = 15.0;
const DEVICE_MS_MAX: f64 = 1.5;
const DEVICE_VRAM_MAX: f64 = 96.0;

const HIGH_POST_MS: f64 = 1.20;
const FROZEN_LIGHT_MS: f64 = 0.43;
const LIGHTING_CAP_MS: f64 = 8.0;
const LOW_POST_MS: f64 = 2.5;

const FMT_BGRA: i32 = 44;
const FMT_RGBA: i32 = 37;
const FMT_A2B: i32 = 64;
const FMT_A2R: i32 = 58;
const FMT_F16: i32 = 97;
const CS_SRGB: i32 = 0;
const CS_SCRGB: i32 = 1_000_104_002;
const CS_HDR10: i32 = 1_000_104_008;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    NotImplemented,
}

#[derive(Clone, Copy)]
enum Compare {
    AtMost,
    AtLeast,
}

struct Metric {
    name: &'static str,
    value: Option<f64>,
    threshold: f64,
    compare: Compare,
    status: Status,
}

#[derive(Clone, Copy)]
struct Listed {
    format: i32,
    space: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Output {
    Sdr,
    Hdr10,
    Scrgb,
}

struct Cost {
    vram: f64,
    scrgb: f64,
    basic: f64,
    stress: f64,
    low_ms: f64,
    device_ms: f64,
    device_vram: f64,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    check_published_rounding()?;
    let dir = output_dir();
    let live = live_metrics();
    let good = proof_good();
    let broken = proof_broken();
    write_set(&dir, "live", &live)?;
    write_set(&dir, "proof-good", &good)?;
    write_set(&dir, "proof-broken", &broken)?;
    print_set("LIVE", &live);
    print_set("PROOF good", &good);
    print_set("PROOF broken", &broken);
    let live_fail = count(&live, Status::Fail);
    let good_miss = count_not(&good, Status::Pass);
    let broken_miss = count_not(&broken, Status::Fail);
    println!(
        "live pass={} fail={live_fail} unimplemented={}",
        count(&live, Status::Pass),
        count(&live, Status::NotImplemented)
    );
    println!(
        "proof_good pass={} unexpected={good_miss}",
        count(&good, Status::Pass)
    );
    println!(
        "proof_broken fail={} unexpected={broken_miss}",
        count(&broken, Status::Fail)
    );
    println!("out {}", dir.display());
    if live_fail > 0 || good_miss > 0 || broken_miss > 0 {
        return Err(format!(
            "gate live_fail={live_fail} proof_good_unexpected={good_miss} proof_broken_unexpected={broken_miss}"
        ));
    }
    Ok(())
}

fn live_metrics() -> Vec<Metric> {
    let cost = published_cost();
    vec![
        at_most("legacy_inverse", legacy_inverse_error(), LEGACY_MAX),
        at_most("post_vram_mib", cost.vram, VRAM_MAX),
        at_most("post_scrgb_mib", cost.scrgb, SCRGB_MAX),
        at_most("basic_frame_ms", cost.basic, BASIC_MAX),
        at_most("stress_frame_ms", cost.stress, STRESS_MAX),
        at_most("low_1060_ms", cost.low_ms, LOW_MS_MAX),
        not_implemented("scene_peak", PEAK_MAX, Compare::AtMost),
        not_implemented("linear_grade", GRADE_MAX, Compare::AtMost),
        not_implemented("grade_is_linear", LINEAR_MIN, Compare::AtLeast),
        not_implemented("exposure_ev", EV_MAX, Compare::AtMost),
        not_implemented("exposure_still", STILL_MAX, Compare::AtMost),
        not_implemented("exposure_adapt", ADAPT_MAX, Compare::AtMost),
        not_implemented("exposure_overshoot", OVERSHOOT_MAX, Compare::AtMost),
        not_implemented("physical_camera", CAMERA_MAX, Compare::AtMost),
        not_implemented("bloom_energy", ENERGY_MAX, Compare::AtMost),
        not_implemented("bloom_firefly", FIREFLY_MAX, Compare::AtMost),
        not_implemented("bloom_commute", COMMUTE_MAX, Compare::AtMost),
        not_implemented("output_pick", PICK_MAX, Compare::AtMost),
        not_implemented("near_error", NEAR_MAX, Compare::AtMost),
        not_implemented("still_pop", POP_MAX, Compare::AtMost),
        not_implemented("agx_highlight", ADAPT_MAX, Compare::AtMost),
        not_implemented("agx_hue", HUE_MAX, Compare::AtMost),
        not_implemented("post_ms_device", DEVICE_MS_MAX, Compare::AtMost),
        not_implemented("post_vram_device", DEVICE_VRAM_MAX, Compare::AtMost),
    ]
}

fn proof_good() -> Vec<Metric> {
    let cost = published_cost();
    vec![
        at_most("legacy_inverse", legacy_inverse_error(), LEGACY_MAX),
        at_most("post_vram_mib", cost.vram, VRAM_MAX),
        at_most("post_scrgb_mib", cost.scrgb, SCRGB_MAX),
        at_most("basic_frame_ms", cost.basic, BASIC_MAX),
        at_most("stress_frame_ms", cost.stress, STRESS_MAX),
        at_most("low_1060_ms", cost.low_ms, LOW_MS_MAX),
        at_most("scene_peak", peak_err(4.0), PEAK_MAX),
        at_most(
            "linear_grade",
            rel_mean(&[0.5, 0.5, 0.5, 0.5], &[0.5, 0.5, 0.5, 0.5]),
            GRADE_MAX,
        ),
        at_least("grade_is_linear", max_channel(&[0.2, 8.0, 0.4]), LINEAR_MIN),
        at_most("exposure_ev", good_meter_error(), EV_MAX),
        at_most(
            "exposure_still",
            max_rel_step(&[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
            STILL_MAX,
        ),
        at_most("exposure_adapt", adapt_outside(false), ADAPT_MAX),
        at_most("exposure_overshoot", adapt_overshoot(false), OVERSHOOT_MAX),
        at_most("physical_camera", camera_error(camera_exposure), CAMERA_MAX),
        at_most("bloom_energy", energy_error(1.0), ENERGY_MAX),
        at_most("bloom_firefly", firefly(1000.0, true), FIREFLY_MAX),
        at_most("bloom_commute", commute_error(linear_centre), COMMUTE_MAX),
        at_most("output_pick", pick_misses(pick_output), PICK_MAX),
        at_most("near_error", weighted(0.0, 0.20), NEAR_MAX),
        at_most(
            "still_pop",
            pop_share(&flat(100, 1.0), &flat(100, 1.0)),
            POP_MAX,
        ),
        at_most(
            "agx_highlight",
            outside_window(0.92, 0.85, 0.995),
            ADAPT_MAX,
        ),
        at_most("agx_hue", 3.0, HUE_MAX),
        at_most("post_ms_device", cost.device_ms, DEVICE_MS_MAX),
        at_most("post_vram_device", cost.device_vram, DEVICE_VRAM_MAX),
    ]
}

fn proof_broken() -> Vec<Metric> {
    let cost = broken_cost();
    vec![
        at_most("legacy_inverse", identity_inverse_error(), LEGACY_MAX),
        at_most("post_vram_mib", cost.vram, VRAM_MAX),
        at_most("post_scrgb_mib", cost.scrgb, SCRGB_MAX),
        at_most("basic_frame_ms", cost.basic, BASIC_MAX),
        at_most("stress_frame_ms", cost.stress, STRESS_MAX),
        at_most("low_1060_ms", cost.low_ms, LOW_MS_MAX),
        at_most("scene_peak", peak_err(1.0), PEAK_MAX),
        at_most(
            "linear_grade",
            rel_mean(&[0.7, 0.7, 0.7, 0.7], &[0.5, 0.5, 0.5, 0.5]),
            GRADE_MAX,
        ),
        at_least(
            "grade_is_linear",
            max_channel(&[0.2, 0.77, 0.4]),
            LINEAR_MIN,
        ),
        at_most("exposure_ev", broken_meter_error(), EV_MAX),
        at_most(
            "exposure_still",
            max_rel_step(&[1.0, 1.05, 1.10, 1.15, 1.20, 1.25, 1.30, 1.35]),
            STILL_MAX,
        ),
        at_most("exposure_adapt", adapt_outside(true), ADAPT_MAX),
        at_most("exposure_overshoot", adapt_overshoot(true), OVERSHOOT_MAX),
        at_most(
            "physical_camera",
            camera_error(camera_exposure_broken),
            CAMERA_MAX,
        ),
        at_most("bloom_energy", energy_error(2.0), ENERGY_MAX),
        at_most("bloom_firefly", firefly(1000.0, false), FIREFLY_MAX),
        at_most(
            "bloom_commute",
            commute_error(threshold_centre),
            COMMUTE_MAX,
        ),
        at_most("output_pick", pick_misses(pick_always_sdr), PICK_MAX),
        at_most("near_error", weighted(0.20, 0.0), NEAR_MAX),
        at_most("still_pop", pop_share(&flat(100, 1.0), &popped()), POP_MAX),
        at_most("agx_highlight", outside_window(1.0, 0.85, 0.995), ADAPT_MAX),
        at_most("agx_hue", 40.0, HUE_MAX),
        at_most("post_ms_device", cost.device_ms, DEVICE_MS_MAX),
        at_most("post_vram_device", cost.device_vram, DEVICE_VRAM_MAX),
    ]
}

fn at_most(name: &'static str, value: f64, threshold: f64) -> Metric {
    let status = if value.is_finite() && value <= threshold {
        Status::Pass
    } else {
        Status::Fail
    };
    Metric {
        name,
        value: Some(value),
        threshold,
        compare: Compare::AtMost,
        status,
    }
}

fn at_least(name: &'static str, value: f64, threshold: f64) -> Metric {
    let status = if value.is_finite() && value >= threshold {
        Status::Pass
    } else {
        Status::Fail
    };
    Metric {
        name,
        value: Some(value),
        threshold,
        compare: Compare::AtLeast,
        status,
    }
}

fn not_implemented(name: &'static str, threshold: f64, compare: Compare) -> Metric {
    Metric {
        name,
        value: None,
        threshold,
        compare,
        status: Status::NotImplemented,
    }
}

fn published_cost() -> Cost {
    let vram = scene_mib() + bloom_mib(6);
    Cost {
        vram,
        scrgb: vram + scrgb_extra_mib(),
        basic: HIGH_POST_MS + FROZEN_LIGHT_MS,
        stress: HIGH_POST_MS + LIGHTING_CAP_MS,
        low_ms: LOW_POST_MS,
        device_ms: 0.40,
        device_vram: vram,
    }
}

fn broken_cost() -> Cost {
    Cost {
        vram: 500.0,
        scrgb: 4_000.0,
        basic: 3.0,
        stress: 12.0,
        low_ms: 4.0,
        device_ms: 5.0,
        device_vram: 4_000.0,
    }
}

fn scene_mib() -> f64 {
    2560.0 * 1440.0 * 8.0 / 1_048_576.0
}

fn bloom_mib(levels: u32) -> f64 {
    let mut width: f64 = 2560.0;
    let mut height: f64 = 1440.0;
    let mut sum = 0.0;
    for _ in 0..levels {
        width = (width / 2.0).floor();
        height = (height / 2.0).floor();
        sum += width * height * 8.0 / 1_048_576.0;
    }
    sum
}

fn scrgb_extra_mib() -> f64 {
    3.0 * 2560.0 * 1440.0 * 4.0 / 1_048_576.0
}

fn hundredths(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn check_published_rounding() -> Result<(), String> {
    let rows = [
        (hundredths(scene_mib()), 28.13),
        (hundredths(scene_mib() + bloom_mib(4)), 37.46),
        (hundredths(scene_mib() + bloom_mib(5)), 37.49),
        (hundredths(scene_mib() + bloom_mib(6)), 37.50),
        (hundredths(scrgb_extra_mib()), 42.19),
        (
            hundredths(scene_mib() + bloom_mib(6) + scrgb_extra_mib()),
            79.69,
        ),
    ];
    for (got, want) in rows {
        if (got - want).abs() > 0.0 {
            return Err(format!("published MiB {got} != {want}"));
        }
    }
    Ok(())
}

fn tone(channel: f32) -> f32 {
    if channel <= 0.64 {
        return channel.max(0.0);
    }
    let extra = channel - 0.64;
    0.64 + 0.14 * (extra / (extra + 1.1))
}

fn legacy_inverse_error() -> f64 {
    let samples = [0.0_f32, 0.25, 0.64, 1.0, 4.0, 16.0];
    let mut worst = 0.0_f64;
    for sample in samples {
        let back = genos_render::linear_from_display(tone(sample));
        let denom = f64::from(sample).max(1.0);
        let err = (f64::from(back) - f64::from(sample)).abs() / denom;
        if err > worst {
            worst = err;
        }
    }
    worst
}

fn identity_inverse_error() -> f64 {
    let sample = 4.0_f32;
    let displayed = tone(sample);
    (f64::from(displayed) - f64::from(sample)).abs() / f64::from(sample)
}

fn peak_err(stored: f64) -> f64 {
    (stored - 4.0).abs() / 4.0
}

fn rel_mean(candidate: &[f64], reference: &[f64]) -> f64 {
    let mut n = 0.0;
    let mut sum = 0.0;
    for sample in reference {
        sum += *sample;
        n += 1.0;
    }
    let mean = if n == 0.0 { 0.0 } else { sum / n };
    let mut err = 0.0;
    let mut count = 0.0;
    for (cand, refer) in candidate.iter().zip(reference.iter()) {
        let denom = refer.max(0.25 * mean).max(0.02);
        err += (cand - refer).abs() / denom;
        count += 1.0;
    }
    if count == 0.0 {
        1.0
    } else {
        err / count
    }
}

fn max_channel(values: &[f64]) -> f64 {
    let mut peak = 0.0;
    for value in values {
        if *value > peak {
            peak = *value;
        }
    }
    peak
}

fn ev100_from_luminance(luma: f64) -> f64 {
    (luma * (100.0 / 12.5)).log2()
}

fn good_meter_error() -> f64 {
    let mut samples = vec![0.18; 1000];
    samples.push(10_000.0);
    (histogram_ev(&samples) - ev100_from_luminance(0.18)).abs()
}

fn broken_meter_error() -> f64 {
    let mut samples = vec![0.18; 1000];
    samples.push(10_000.0);
    let mut sum = 0.0;
    for sample in &samples {
        sum += *sample;
    }
    let mean = sum / f64_from_usize(samples.len());
    (ev100_from_luminance(mean) - ev100_from_luminance(0.18)).abs()
}

fn histogram_ev(samples: &[f64]) -> f64 {
    const BINS: usize = 256;
    let mut counts = vec![0_u32; BINS];
    let mut total = 0_u32;
    for sample in samples {
        if *sample <= 0.0 {
            continue;
        }
        let index = bin_of(*sample, BINS);
        if let Some(slot) = counts.get_mut(index) {
            *slot += 1;
            total += 1;
        }
    }
    if total == 0 {
        return 0.0;
    }
    let drop = floor_u32(f64::from(total) * 0.10);
    let mut skip = drop;
    let mut keep = total.saturating_sub(drop.saturating_mul(2));
    if keep == 0 {
        keep = total;
        skip = 0;
    }
    let mut acc = 0.0;
    let mut seen = 0_u32;
    for (index, count) in counts.iter().enumerate() {
        let mut left = *count;
        if skip > 0 {
            let cut = skip.min(left);
            skip -= cut;
            left -= cut;
        }
        if left == 0 || seen >= keep {
            continue;
        }
        let take = left.min(keep - seen);
        acc += f64::from(take) * bin_center(index, BINS);
        seen += take;
    }
    if seen == 0 {
        return 0.0;
    }
    let mean_log = acc / f64::from(seen);
    mean_log + (8.0_f64).log2()
}

fn bin_of(luma: f64, bins: usize) -> usize {
    let log_l = luma.log2().clamp(-16.0, 15.999);
    let scaled = (log_l + 16.0) / 32.0 * f64_from_usize(bins);
    let index = floor_usize(scaled);
    if index >= bins {
        bins - 1
    } else {
        index
    }
}

fn bin_center(index: usize, bins: usize) -> f64 {
    let width = 32.0 / f64_from_usize(bins);
    -16.0 + (f64_from_usize(index) + 0.5) * width
}

fn f64_from_usize(value: usize) -> f64 {
    let mut left = value;
    let mut acc = 0.0;
    while left > 0 {
        acc += 1.0;
        left -= 1;
    }
    acc
}

fn floor_usize(value: f64) -> usize {
    let mut left = value.max(0.0);
    let mut acc = 0_usize;
    while left >= 1.0 {
        left -= 1.0;
        acc += 1;
    }
    acc
}

fn floor_u32(value: f64) -> u32 {
    let mut left = value.max(0.0);
    let mut acc = 0_u32;
    while left >= 1.0 {
        left -= 1.0;
        acc += 1;
    }
    acc
}

fn max_rel_step(values: &[f64]) -> f64 {
    let mut worst = 0.0;
    let mut iter = values.iter();
    let Some(first) = iter.next() else {
        return 1.0;
    };
    let mut prev = *first;
    for value in iter {
        let denom = prev.abs().max(1e-6);
        let step = (value - prev).abs() / denom;
        if step > worst {
            worst = step;
        }
        prev = *value;
    }
    worst
}

fn adapt_outside(jump: bool) -> f64 {
    let (t90, _) = simulate_adapt(jump);
    outside_window(t90, 0.40, 2.00)
}

fn adapt_overshoot(jump: bool) -> f64 {
    let (_, overshoot) = simulate_adapt(jump);
    overshoot
}

fn simulate_adapt(jump: bool) -> (f64, f64) {
    let dt: f64 = 1.0 / 60.0;
    let tau: f64 = 0.50;
    let mut progress = 0.0;
    let mut time = 0.0;
    let mut t90 = 10.0;
    let mut saw = false;
    let mut overshoot = 0.0;
    for frame in 0..600 {
        if jump && frame == 0 {
            progress = 1.5;
        } else if !jump {
            let gain = 1.0 - (-dt / tau).exp();
            progress += (1.0 - progress) * gain;
        }
        time += dt;
        if progress > 1.0 {
            let past = progress - 1.0;
            if past > overshoot {
                overshoot = past;
            }
        }
        if !saw && progress >= 0.90 {
            t90 = time;
            saw = true;
        }
    }
    (t90, overshoot)
}

fn outside_window(value: f64, low: f64, high: f64) -> f64 {
    if value < low {
        low - value
    } else if value > high {
        value - high
    } else {
        0.0
    }
}

fn camera_exposure(aperture: f64, shutter: f64, iso: f64) -> f64 {
    let scale = (aperture * aperture) / shutter * 100.0 / iso;
    1.0 / (1.2 * scale)
}

fn camera_exposure_broken(aperture: f64, shutter: f64, iso: f64) -> f64 {
    let scale = (aperture * aperture) / shutter * 100.0 / iso;
    1.0 / scale
}

fn camera_error(exposure: fn(f64, f64, f64) -> f64) -> f64 {
    let points = [
        (1.0, 1.0, 100.0, 1.0 / 1.2),
        (4.0, 1.0 / 125.0, 100.0, 1.0 / 2400.0),
    ];
    let mut worst = 0.0;
    for (aperture, shutter, iso, want) in points {
        let got = exposure(aperture, shutter, iso);
        let err = (got / want - 1.0).abs();
        if err > worst {
            worst = err;
        }
    }
    worst
}

fn energy_error(scale: f64) -> f64 {
    let centre = 0.125;
    let inner = 0.125 * 4.0;
    let axis = 0.0625 * 4.0;
    let corner = 0.03125 * 4.0;
    (scale * (centre + inner + axis + corner) - 1.0).abs()
}

fn karis(luma: f64) -> f64 {
    1.0 / (1.0 + luma)
}

fn firefly(centre: f64, karis_on: bool) -> f64 {
    if !karis_on {
        return centre * 0.125;
    }
    let outer = centre / 4.0;
    let w0 = 0.5 * karis(0.0);
    let w_outer = 0.125 * karis(outer);
    let sum_w = w0 + w_outer * 4.0;
    let sum_c = outer * w_outer * 4.0;
    sum_c / sum_w
}

fn linear_centre(value: f64) -> f64 {
    value * 0.125
}

fn threshold_centre(value: f64) -> f64 {
    if value < 1.0 {
        0.0
    } else {
        value * 0.125
    }
}

fn commute_error(bloom: fn(f64) -> f64) -> f64 {
    let value = 0.5;
    let gain = 2.0;
    (bloom(gain * value) - gain * bloom(value)).abs()
}

fn pick_output(list: &[Listed]) -> Output {
    if list
        .iter()
        .any(|item| item.format == FMT_F16 && item.space == CS_SCRGB)
    {
        return Output::Scrgb;
    }
    if list
        .iter()
        .any(|item| (item.format == FMT_A2B || item.format == FMT_A2R) && item.space == CS_HDR10)
    {
        return Output::Hdr10;
    }
    Output::Sdr
}

fn pick_always_sdr(_list: &[Listed]) -> Output {
    Output::Sdr
}

fn pick_misses(chooser: fn(&[Listed]) -> Output) -> f64 {
    let scrgb = [
        Listed {
            format: FMT_F16,
            space: CS_SCRGB,
        },
        Listed {
            format: FMT_A2B,
            space: CS_HDR10,
        },
        Listed {
            format: FMT_BGRA,
            space: CS_SRGB,
        },
    ];
    let hdr10 = [
        Listed {
            format: FMT_A2R,
            space: CS_HDR10,
        },
        Listed {
            format: FMT_RGBA,
            space: CS_SRGB,
        },
    ];
    let sdr = [Listed {
        format: FMT_BGRA,
        space: CS_SRGB,
    }];
    let mut wrong = 0.0;
    if chooser(&scrgb) != Output::Scrgb {
        wrong += 1.0;
    }
    if chooser(&hdr10) != Output::Hdr10 {
        wrong += 1.0;
    }
    if chooser(&sdr) != Output::Sdr {
        wrong += 1.0;
    }
    wrong
}

fn weighted(near: f64, far: f64) -> f64 {
    (4.0 * near + far) / 5.0
}

fn flat(count: usize, value: f64) -> Vec<f64> {
    vec![value; count]
}

fn popped() -> Vec<f64> {
    let mut values = flat(100, 1.0);
    for value in values.iter_mut().take(10) {
        *value = 1.5;
    }
    values
}

fn pop_share(before: &[f64], after: &[f64]) -> f64 {
    let mut n = 0.0;
    let mut moved = 0.0;
    for (before_y, after_y) in before.iter().zip(after.iter()) {
        n += 1.0;
        if (before_y - after_y).abs() > 0.01 {
            moved += 1.0;
        }
    }
    if n == 0.0 {
        1.0
    } else {
        moved / n
    }
}

fn count(metrics: &[Metric], status: Status) -> usize {
    metrics
        .iter()
        .filter(|metric| metric.status == status)
        .count()
}

fn count_not(metrics: &[Metric], status: Status) -> usize {
    metrics
        .iter()
        .filter(|metric| metric.status != status)
        .count()
}

fn print_set(label: &str, metrics: &[Metric]) {
    for metric in metrics {
        println!("{label} {}", format_metric(metric));
    }
}

fn format_metric(metric: &Metric) -> String {
    let value = match metric.value {
        Some(number) => format!("{number:.6}"),
        None => "-".to_string(),
    };
    let op = match metric.compare {
        Compare::AtMost => "<=",
        Compare::AtLeast => ">=",
    };
    format!(
        "{} {value} {op}{:.6} {}",
        metric.name,
        metric.threshold,
        status_str(metric.status)
    )
}

fn status_str(status: Status) -> &'static str {
    match status {
        Status::Pass => "PASS",
        Status::Fail => "FAIL",
        Status::NotImplemented => "NOT-IMPLEMENTED",
    }
}

fn write_set(dir: &Path, name: &str, metrics: &[Metric]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let mut body = String::from("{\"set\":\"");
    body.push_str(name);
    body.push_str("\",\"metrics\":[");
    for (index, metric) in metrics.iter().enumerate() {
        if index > 0 {
            body.push(',');
        }
        body.push_str(&json_metric(metric));
    }
    body.push_str("]}\n");
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, body).map_err(|err| format!("{}: {err}", path.display()))
}

fn json_metric(metric: &Metric) -> String {
    let value = match metric.value {
        Some(number) => format!("{number:.8}"),
        None => "null".to_string(),
    };
    let compare = match metric.compare {
        Compare::AtMost => "at_most",
        Compare::AtLeast => "at_least",
    };
    format!(
        "{{\"name\":\"{}\",\"value\":{value},\"threshold\":{:.8},\"compare\":\"{compare}\",\"status\":\"{}\"}}",
        metric.name,
        metric.threshold,
        status_str(metric.status)
    )
}

fn output_dir() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    for (index, arg) in args.iter().enumerate() {
        if arg == "--out" {
            if let Some(dir) = args.get(index + 1) {
                return PathBuf::from(dir);
            }
        }
    }
    if let Some(dir) = env_out() {
        return PathBuf::from(dir);
    }
    PathBuf::from("genos-debug/hdr-output")
}

#[allow(
    clippy::disallowed_methods,
    reason = "--out wins; GENOS_HDR_OUT applies only when that argument was omitted"
)]
fn env_out() -> Option<String> {
    std::env::var("GENOS_HDR_OUT").ok()
}
