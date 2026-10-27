//! HDR output metric gate. CPU references, plus lavapipe readbacks: the plain
//! compose picture (`legacy_shader`), one fixed `AgX` frame (`gpu_agx`,
//! `gpu_exposure`, `gpu_bloom`), and the HDR encode buffer (`hdr_encode`,
//! `hdr_scrgb`).
//!
//! ```text
//! cargo test -p genos-render --test hdr_output_gate -- --out genos-debug/hdr-output
//! ```
//!
//! `--out` wins. `GENOS_HDR_OUT` applies only when `--out` was omitted.
//!
//! `AgX` rows compare [`genos_render::agx_rec709`] with samples from Eary Chow's
//! `AgX_LUT_Gen` sRGB cube path (power 2.4). `ACES` stays `NOT-IMPLEMENTED` until
//! `aces2_rec709` is the Academy CTL.

use genos_render::{
    aces2_rec709, agx_display_linear, agx_rec709, bloom_commute_error, bloom_downsample,
    bloom_pyramid, bloom_upsample,
    ev100_from_luminance, exposure_adapt_outside, exposure_from_camera, exposure_from_ev100,
    exposure_overshoot, exposure_still_change, exposure_with_compensation, graphics_white_linear,
    hdr10_pq, histogram_ev100, karis_downsample_rgb, karis_firefly_response, linear_from_display,
    pack_f32,
    pick_hdr_output, pq_st2084, rec2020_from_rec709, resolve_swapchain,
    scene_luma, scrgb_code, scrgb_from_display, step_exposure, swapchain_nits, tone, HdrSwapchain,
    HDR_PEAK_NITS,
    Image, ListedFormat, PresentHost, Renderer, World, COLOR_SPACE_HDR10, COLOR_SPACE_SCRGB,
    COLOR_SPACE_SRGB, FORMAT_A2B10, FORMAT_A2R10, FORMAT_BGRA8, FORMAT_R16F, FORMAT_RGBA8,
};

use genos_scene::{Camera, HdrSettings, LightingPreset, LightingSettings, OutputMode};
use genos_window::Window;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const LEGACY_MAX: f64 = 1e-4;
/// Max code delta of the plain compose picture against the stored shoulder.
const LEGACY_CODE_MAX: f64 = 0.0;
/// Max code delta of `hdr_agx` mode 0 against `tone`, after 8-bit dither.
const GPU_LEGACY_MAX: f64 = 1.0;
const EV_MAX: f64 = 1e-9;
const HIST_MAX: f64 = 0.10;
const CAMERA_MAX: f64 = 1e-9;
const BLOOM_MAX: f64 = 1e-4;
const PICK_MAX: f64 = 0.0;
const AGX_DE_MAX: f64 = 0.5;
const AGX_CODE_MAX: f64 = 0.0;
const ACES_MAX: f64 = 1e-3;
const PQ_MAX: f64 = 1e-7;
/// Literal scRGB code of 203 nits. Not `203/80`, so a shared divisor cannot pass itself.
const SCRGB_WHITE: f64 = 2.5375;
const STILL_MAX: f64 = 0.001;
const ADAPT_MAX: f64 = 0.0;
const OVERSHOOT_MAX: f64 = 0.02;
const FIREFLY_MAX: f64 = 1.0;
const COMMUTE_MAX: f64 = 0.01;
const DEVICE_MS_MAX: f64 = 1.5;
const DEVICE_VRAM_MAX: f64 = 96.0;
/// Mean |picture − AgX(exposure × (scene + bloom × k))| on lit pixels.
/// Sits above 8-bit dither and below a skipped `AgX`, a wrong lattice, a ×4 bloom add,
/// and an exposure that stays at 1.
const GPU_AGX_MAX: f64 = 0.02;
/// `|uploaded scale − step_exposure|`. A scale pinned at 1 fails.
const GPU_EXPOSURE_MAX: f64 = 1.0e-4;
/// Mean |uploaded bloom − tap pyramid| . A ×2 pyramid fails.
const GPU_BLOOM_MAX: f64 = 1.0e-3;
/// Mean |encode buffer − `hdr10_pq` / `scrgb_from_display`|. An 80-nit paper white fails.
const HDR_ENCODE_MAX: f64 = 0.02;
/// Fixed EV100. Exposure `1 / (1.2 × 4)` is far from the legacy scale of 1.
const GPU_EV100: f64 = 2.0;
const GPU_TAU: f64 = 0.001;
const GPU_BLOOM_K: f64 = 1.0;
const GPU_BLOOM_LEVELS: u32 = 3;
const GPU_FRAMES: u32 = 2;
/// Plain compose of the shipped hall, Potato, ambient 8, `legacy_bytes`, 160×90.
const LEGACY_SHOULDER: &[u8] = include_bytes!("data/legacy_shoulder_160x90.bgra");

/// Scene-linear Rec.709, then the `AgX_LUT_Gen` sRGB power-2.4 code.
const AGX_LUT: [([f64; 3], [f64; 3]); 10] = [
    ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
    (
        [0.18, 0.18, 0.18],
        [
            0.489_437_089_573_878_3,
            0.489_437_089_573_878_3,
            0.489_437_089_573_878_3,
        ],
    ),
    (
        [1.0, 1.0, 1.0],
        [
            0.783_034_430_557_485_1,
            0.783_034_430_557_484_8,
            0.783_034_430_557_484_9,
        ],
    ),
    (
        [0.01, 0.01, 0.01],
        [
            0.120_774_598_513_341_03,
            0.120_774_598_513_341_05,
            0.120_774_598_513_341_03,
        ],
    ),
    (
        [4.0, 4.0, 4.0],
        [
            0.918_488_543_850_948_8,
            0.918_488_543_850_948_7,
            0.918_488_543_850_948_8,
        ],
    ),
    (
        [1.0, 0.0, 0.0],
        [
            0.865_962_537_870_047_6,
            0.264_522_664_434_557_24,
            0.177_858_573_873_015_76,
        ],
    ),
    (
        [0.0, 1.0, 0.0],
        [
            0.462_392_843_381_622_73,
            0.784_903_172_647_624_8,
            0.354_524_142_682_191_35,
        ],
    ),
    (
        [0.0, 0.0, 1.0],
        [
            0.161_839_758_958_006_4,
            0.364_939_922_484_125_16,
            0.835_264_137_740_130_6,
        ],
    ),
    (
        [0.5, 0.2, 0.1],
        [
            0.701_440_063_265_896_5,
            0.505_681_200_856_255_2,
            0.394_350_144_124_809_57,
        ],
    ),
    (
        [16.0, 16.0, 16.0],
        [
            0.999_184_949_265_366_5,
            0.999_184_949_265_366_4,
            0.999_184_949_265_366_3,
        ],
    ),
];

/// ST 2084 inverse-EOTF codes from the integer constants, not from [`pq_st2084`].
const PQ_CODE: [(f64, f64); 5] = [
    (0.0, 0.000_000_730_955_902_6),
    (100.0, 0.508_078_421_517_394_9),
    (203.0, 0.580_688_881_041_607_8),
    (1_000.0, 0.751_827_096_247_041_8),
    (10_000.0, 1.0),
];

const BT709_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.412_386_563_252_991_8,
        0.357_591_490_920_625_4,
        0.180_450_491_203_563_7,
    ],
    [
        0.212_636_821_677_323_8,
        0.715_182_981_841_250_7,
        0.072_180_196_481_425_5,
    ],
    [
        0.019_330_620_152_484,
        0.119_197_163_640_208_5,
        0.950_372_587_005_435_5,
    ],
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    NotImplemented,
}

#[derive(Clone, Copy)]
enum Compare {
    AtMost,
    Equal,
    /// The value is a lavapipe timestamp. The threshold is the 4070 ceiling and is not the pass test.
    Collected,
}

struct Metric {
    name: &'static str,
    value: Option<f64>,
    threshold: f64,
    compare: Compare,
    status: Status,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
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
    vec![
        at_most("legacy_inverse", legacy_inverse_error(), LEGACY_MAX),
        at_most("legacy_shader", legacy_shader_error(false), LEGACY_CODE_MAX),
        at_most("gpu_legacy", gpu_legacy_error(false), GPU_LEGACY_MAX),
        at_most(
            "exposure_ev",
            exposure_patch_error(ev100_from_luminance, exposure_from_ev100),
            EV_MAX,
        ),
        at_most(
            "exposure_histogram",
            histogram_error(histogram_ev100),
            HIST_MAX,
        ),
        at_most("exposure_still", exposure_still_change(), STILL_MAX),
        at_most("exposure_adapt", exposure_adapt_outside(), ADAPT_MAX),
        at_most("exposure_overshoot", exposure_overshoot(), OVERSHOOT_MAX),
        at_most(
            "physical_camera",
            camera_error(exposure_from_camera),
            CAMERA_MAX,
        ),
        at_most("bloom_energy", bloom_error(), BLOOM_MAX),
        at_most("bloom_firefly", karis_firefly_response(1_000.0), FIREFLY_MAX),
        at_most("bloom_commute", bloom_commute_error(), COMMUTE_MAX),
        at_most("gpu_agx", gpu_agx_error(false), GPU_AGX_MAX),
        at_most("gpu_exposure", gpu_exposure_error(false), GPU_EXPOSURE_MAX),
        at_most("gpu_bloom", gpu_bloom_error(false), GPU_BLOOM_MAX),
        at_most("hdr_encode", hdr_encode_error(false), HDR_ENCODE_MAX),
        at_most("hdr_scrgb", hdr_scrgb_error(false), HDR_ENCODE_MAX),
        at_most("output_pick", live_output_pick(), PICK_MAX),
        at_most("agx_de", agx_delta_e(agx_rec709), AGX_DE_MAX),
        at_most(
            "agx_highlight",
            agx_highlight_distance(agx_rec709),
            AGX_CODE_MAX,
        ),
        aces_row(aces2_rec709([1.0, 1.0, 1.0])),
        at_most("pq_st2084", pq_error(pq_st2084), PQ_MAX),
        equal("scrgb_code", scrgb_code(203.0), SCRGB_WHITE),
        collected_ms("post_ms_device", gpu_post_ms(), DEVICE_MS_MAX),
        at_most("post_vram_device", gpu_post_vram(), DEVICE_VRAM_MAX),
    ]
}

fn proof_good() -> Vec<Metric> {
    vec![
        at_most("legacy_inverse", legacy_inverse_error(), LEGACY_MAX),
        at_most("legacy_shader", legacy_shader_error(false), LEGACY_CODE_MAX),
        at_most("gpu_legacy", gpu_legacy_error(false), GPU_LEGACY_MAX),
        at_most(
            "exposure_ev",
            exposure_patch_error(ev100_from_luminance, exposure_from_ev100),
            EV_MAX,
        ),
        at_most(
            "exposure_histogram",
            histogram_error(histogram_ev100),
            HIST_MAX,
        ),
        at_most("exposure_still", exposure_still_change(), STILL_MAX),
        at_most("exposure_adapt", exposure_adapt_outside(), ADAPT_MAX),
        at_most("exposure_overshoot", exposure_overshoot(), OVERSHOOT_MAX),
        at_most(
            "physical_camera",
            camera_error(exposure_from_camera),
            CAMERA_MAX,
        ),
        at_most("bloom_energy", bloom_error(), BLOOM_MAX),
        at_most("bloom_firefly", karis_firefly_response(1_000.0), FIREFLY_MAX),
        at_most("bloom_commute", bloom_commute_error(), COMMUTE_MAX),
        at_most("gpu_agx", gpu_agx_error(false), GPU_AGX_MAX),
        at_most("gpu_exposure", gpu_exposure_error(false), GPU_EXPOSURE_MAX),
        at_most("gpu_bloom", gpu_bloom_error(false), GPU_BLOOM_MAX),
        at_most("hdr_encode", hdr_encode_error(false), HDR_ENCODE_MAX),
        at_most("hdr_scrgb", hdr_scrgb_error(false), HDR_ENCODE_MAX),
        at_most("output_pick", live_output_pick(), PICK_MAX),
        at_most("agx_de", agx_delta_e(agx_rec709), AGX_DE_MAX),
        at_most(
            "agx_highlight",
            agx_highlight_distance(agx_rec709),
            AGX_CODE_MAX,
        ),
        at_most(
            "aces_ctl",
            channel_error([0.2, 0.4, 0.6], [0.2, 0.4, 0.6]),
            ACES_MAX,
        ),
        at_most("pq_st2084", pq_error(pq_st2084), PQ_MAX),
        equal("scrgb_code", scrgb_code(203.0), SCRGB_WHITE),
        at_most("post_ms_device", 0.40, DEVICE_MS_MAX),
        at_most("post_vram_device", 37.5, DEVICE_VRAM_MAX),
    ]
}

fn proof_broken() -> Vec<Metric> {
    vec![
        at_most("legacy_inverse", identity_inverse_error(), LEGACY_MAX),
        at_most("legacy_shader", legacy_shader_error(true), LEGACY_CODE_MAX),
        at_most("gpu_legacy", gpu_legacy_error(true), GPU_LEGACY_MAX),
        at_most(
            "exposure_ev",
            exposure_patch_error(ev100_from_luminance, exposure_without_saturation),
            EV_MAX,
        ),
        at_most("exposure_histogram", histogram_error(mean_ev), HIST_MAX),
        at_most("exposure_still", exposure_still_stepped(), STILL_MAX),
        at_most("exposure_adapt", adapt_outside(true), ADAPT_MAX),
        at_most("exposure_overshoot", adapt_overshoot(true), OVERSHOOT_MAX),
        at_most(
            "physical_camera",
            camera_error(exposure_without_saturation_camera),
            CAMERA_MAX,
        ),
        at_most("bloom_energy", bloom_energy_doubled(), BLOOM_MAX),
        at_most("bloom_firefly", firefly_without_karis(1_000.0), FIREFLY_MAX),
        at_most("bloom_commute", bloom_commute_dropped_scale(), COMMUTE_MAX),
        at_most("gpu_agx", gpu_agx_error(true), GPU_AGX_MAX),
        at_most("gpu_exposure", gpu_exposure_error(true), GPU_EXPOSURE_MAX),
        at_most("gpu_bloom", gpu_bloom_error(true), GPU_BLOOM_MAX),
        at_most("hdr_encode", hdr_encode_error(true), HDR_ENCODE_MAX),
        at_most("hdr_scrgb", hdr_scrgb_error(true), HDR_ENCODE_MAX),
        at_most(
            "output_pick",
            pick_misses(pick_always_sdr) + force_sdr_misses(pick_ignores_force),
            PICK_MAX,
        ),
        at_most("agx_de", agx_delta_e(|_scene| [1.0, 1.0, 1.0]), AGX_DE_MAX),
        at_most(
            "agx_highlight",
            agx_highlight_distance(agx_hard_clip),
            AGX_CODE_MAX,
        ),
        at_most(
            "aces_ctl",
            channel_error([0.2, 0.4, 0.6], [0.4, 0.4, 0.6]),
            ACES_MAX,
        ),
        at_most("pq_st2084", pq_error(|nits| nits / 10_000.0), PQ_MAX),
        equal("scrgb_code", eighty_nit_paper_white(203.0), SCRGB_WHITE),
        at_most("post_ms_device", 5.0, DEVICE_MS_MAX),
        at_most("post_vram_device", 4_000.0, DEVICE_VRAM_MAX),
    ]
}

fn aces_row(formed: Option<[f64; 3]>) -> Metric {
    match formed {
        Some(rgb) => at_most("aces_ctl", channel_error(rgb, [0.0, 0.0, 0.0]), ACES_MAX),
        None => not_implemented("aces_ctl", ACES_MAX, Compare::AtMost),
    }
}

fn equal(name: &'static str, value: f64, threshold: f64) -> Metric {
    let status = if value.is_finite() && value.to_bits() == threshold.to_bits() {
        Status::Pass
    } else {
        Status::Fail
    };
    Metric {
        name,
        value: Some(value),
        threshold,
        compare: Compare::Equal,
        status,
    }
}

/// Mutation: paper white is 80 nits, so the code is `80 / 80`.
#[allow(
    clippy::eq_op,
    reason = "the broken proof divides an 80-nit paper white by 80 and must not equal 2.5375"
)]
fn eighty_nit_paper_white(_nits: f64) -> f64 {
    80.0 / 80.0
}

fn collected_ms(name: &'static str, value: f64, device_ceiling: f64) -> Metric {
    let status = if value.is_finite() && value > 0.0 {
        Status::Pass
    } else {
        Status::Fail
    };
    Metric {
        name,
        value: Some(value),
        threshold: device_ceiling,
        compare: Compare::Collected,
        status,
    }
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

fn not_implemented(name: &'static str, threshold: f64, compare: Compare) -> Metric {
    Metric {
        name,
        value: None,
        threshold,
        compare,
        status: Status::NotImplemented,
    }
}

fn legacy_inverse_error() -> f64 {
    let samples = [0.0_f32, 0.25, 0.64, 1.0, 4.0, 16.0];
    let mut worst = 0.0;
    for sample in samples {
        let back = linear_from_display(tone(sample));
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

fn spec_ev(luminance: f64) -> f64 {
    (luminance * (100.0 / 12.5)).log2()
}

fn spec_exposure(ev100: f64) -> f64 {
    1.0 / (1.2 * ev100.exp2())
}

fn exposure_patch_error(ev: fn(f64) -> f64, scale: fn(f64) -> f64) -> f64 {
    let patches = [0.01_f64, 0.18, 1.0, 12.5];
    let mut worst = 0.0;
    for luminance in patches {
        let want_ev = spec_ev(luminance);
        let got_ev = ev(luminance);
        let ev_err = (got_ev - want_ev).abs();
        let want_scale = spec_exposure(want_ev);
        let got_scale = scale(got_ev);
        let scale_err = (got_scale / want_scale - 1.0).abs();
        if ev_err > worst {
            worst = ev_err;
        }
        if scale_err > worst {
            worst = scale_err;
        }
    }
    worst
}

fn exposure_without_saturation(ev100: f64) -> f64 {
    1.0 / ev100.exp2()
}

fn histogram_error(meter: fn(&[f64]) -> f64) -> f64 {
    let mut one = vec![0.18; 1000];
    one.push(10_000.0);
    // Nine percent of the samples are the sun. That sits inside the top 10%
    // the meter drops, so the body stays on 0.18. Without the drop it moves
    // by more than a stop.
    let mut heavy = vec![0.18; 910];
    heavy.extend(std::iter::repeat_n(10_000.0, 90));
    let grey = spec_ev(0.18);
    (meter(&one) - grey)
        .abs()
        .max((meter(&heavy) - grey).abs())
}

fn mean_ev(samples: &[f64]) -> f64 {
    let mut sum = 0.0;
    let mut n = 0.0;
    for sample in samples {
        sum += *sample;
        n += 1.0;
    }
    spec_ev(if n == 0.0 { 0.0 } else { sum / n })
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

fn exposure_without_saturation_camera(aperture: f64, shutter: f64, iso: f64) -> f64 {
    let scale = (aperture * aperture) / shutter * 100.0 / iso;
    1.0 / scale
}

fn bloom_field() -> (Vec<f32>, Vec<f64>) {
    let side = 16usize;
    let mut rgba = vec![0.0_f32; side.saturating_mul(side).saturating_mul(4)];
    for index in 0..side.saturating_mul(side) {
        let base = index.saturating_mul(4);
        if let Some(slot) = rgba.get_mut(base) {
            *slot = 0.5;
        }
        if let Some(slot) = rgba.get_mut(base.saturating_add(1)) {
            *slot = 0.25;
        }
        if let Some(slot) = rgba.get_mut(base.saturating_add(2)) {
            *slot = 0.125;
        }
    }
    let mut rgb = Vec::new();
    for index in 0..side.saturating_mul(side) {
        let base = index.saturating_mul(4);
        rgb.push(f64::from(rgba.get(base).copied().unwrap_or(0.0)));
        rgb.push(f64::from(rgba.get(base.saturating_add(1)).copied().unwrap_or(0.0)));
        rgb.push(f64::from(rgba.get(base.saturating_add(2)).copied().unwrap_or(0.0)));
    }
    (rgba, rgb)
}

fn bloom_error() -> f64 {
    let (rgba, rgb) = bloom_field();
    let got = bloom_pyramid(&rgba, 16, 16, 3);
    let want = reference_pyramid(&rgb, 16, 16, 3);
    bloom_delta(&got, &want, 1.0)
}

/// The real pyramid, with the red channel doubled the way a bad store does.
fn bloom_energy_doubled() -> f64 {
    let (rgba, rgb) = bloom_field();
    let mut got = bloom_pyramid(&rgba, 16, 16, 3);
    for index in 0..16usize.saturating_mul(16) {
        let slot = index.saturating_mul(4);
        if let Some(red) = got.get_mut(slot) {
            *red *= 2.0;
        }
    }
    let want = reference_pyramid(&rgb, 16, 16, 3);
    bloom_delta(&got, &want, 1.0)
}

/// `bloom_commute_error` without the `k` on the unscaled side.
fn bloom_commute_dropped_scale() -> f64 {
    let mut pixels = vec![0.0; 16];
    if let Some(slot) = pixels.get_mut(5) {
        *slot = 1.0;
    }
    let scale = 2.0;
    let plain = bloom_downsample(&pixels, 4, 4);
    for sample in &mut pixels {
        *sample *= scale;
    }
    let scaled = bloom_downsample(&pixels, 4, 4);
    let mut worst = 0.0;
    for (left, right) in plain.pixels.iter().zip(scaled.pixels.iter()) {
        let err = (right - left).abs();
        if err > worst {
            worst = err;
        }
    }
    worst
}

/// Same five-box sum as `karis_firefly_response`, with the Karis weight left at 1.
fn firefly_without_karis(centre: f64) -> f64 {
    let outer = centre / 4.0;
    let centre_w = 0.5;
    let outer_w = 0.125;
    (outer * outer_w * 4.0) / (centre_w + outer_w * 4.0)
}

/// `step_exposure` plus a fixed 0.05, so a held target still walks.
fn exposure_still_stepped() -> f64 {
    let target = exposure_from_ev100(0.0);
    let mut scale = target;
    let mut worst = 0.0;
    let mut prev = scale;
    for _ in 0..8 {
        scale = step_exposure(scale, target, 1.0 / 60.0, 0.40, 0.80) + 0.05;
        let step = (scale - prev).abs() / prev.abs().max(1.0e-6);
        if step > worst {
            worst = step;
        }
        prev = scale;
    }
    worst
}

/// Highlight clipped to 1 before `agx_rec709`, so linear 16 never reaches the shoulder.
fn agx_hard_clip(scene: [f64; 3]) -> [f64; 3] {
    agx_rec709([
        scene[0].min(1.0),
        scene[1].min(1.0),
        scene[2].min(1.0),
    ])
}

fn pq_error(encode: fn(f64) -> f64) -> f64 {
    let mut worst = 0.0;
    for (nits, code) in PQ_CODE {
        let err = (encode(nits) - code).abs();
        if err > worst {
            worst = err;
        }
    }
    worst
}

fn agx_delta_e(formed: fn([f64; 3]) -> [f64; 3]) -> f64 {
    let mut worst = 0.0;
    for (scene, code) in AGX_LUT {
        let got = delta_e2000(lab_from_code(formed(scene)), lab_from_code(code));
        if got > worst {
            worst = got;
        }
    }
    worst
}

fn agx_highlight_distance(formed: fn([f64; 3]) -> [f64; 3]) -> f64 {
    let code = formed([16.0, 16.0, 16.0]);
    let peak = code[0].max(code[1]).max(code[2]);
    outside_window(peak, 0.998, 0.999_5)
}

fn channel_error(got: [f64; 3], want: [f64; 3]) -> f64 {
    let mut worst = 0.0;
    for (left, right) in got.iter().zip(want.iter()) {
        let err = (left - right).abs();
        if err > worst {
            worst = err;
        }
    }
    worst
}

fn pick_misses(chooser: fn(PresentHost, &[ListedFormat]) -> HdrSwapchain) -> f64 {
    let both = [
        ListedFormat {
            format: FORMAT_R16F,
            color_space: COLOR_SPACE_SCRGB,
        },
        ListedFormat {
            format: FORMAT_A2B10,
            color_space: COLOR_SPACE_HDR10,
        },
        ListedFormat {
            format: FORMAT_BGRA8,
            color_space: COLOR_SPACE_SRGB,
        },
    ];
    let hdr10 = [
        ListedFormat {
            format: FORMAT_A2R10,
            color_space: COLOR_SPACE_HDR10,
        },
        ListedFormat {
            format: FORMAT_RGBA8,
            color_space: COLOR_SPACE_SRGB,
        },
    ];
    let sdr = [ListedFormat {
        format: FORMAT_BGRA8,
        color_space: COLOR_SPACE_SRGB,
    }];
    let cases = [
        (PresentHost::Linux, both.as_slice(), HdrSwapchain::Hdr10),
        (PresentHost::Windows, both.as_slice(), HdrSwapchain::Scrgb),
        (PresentHost::Linux, sdr.as_slice(), HdrSwapchain::Sdr),
        (PresentHost::Windows, hdr10.as_slice(), HdrSwapchain::Hdr10),
        (PresentHost::Mac, sdr.as_slice(), HdrSwapchain::Sdr),
        (PresentHost::Mac, hdr10.as_slice(), HdrSwapchain::Hdr10),
    ];
    let mut wrong = 0.0;
    for (host, list, want) in cases {
        if chooser(host, list) != want {
            wrong += 1.0;
        }
    }
    wrong
}

fn pick_always_sdr(_host: PresentHost, _list: &[ListedFormat]) -> HdrSwapchain {
    HdrSwapchain::Sdr
}

fn live_output_pick() -> f64 {
    pick_misses(pick_hdr_output) + force_sdr_misses(resolve_swapchain)
}

fn force_sdr_misses(
    chooser: fn(PresentHost, &[ListedFormat], OutputMode, bool) -> HdrSwapchain,
) -> f64 {
    let both = [
        ListedFormat {
            format: FORMAT_R16F,
            color_space: COLOR_SPACE_SCRGB,
        },
        ListedFormat {
            format: FORMAT_A2B10,
            color_space: COLOR_SPACE_HDR10,
        },
        ListedFormat {
            format: FORMAT_BGRA8,
            color_space: COLOR_SPACE_SRGB,
        },
    ];
    let mut wrong = 0.0;
    let cases = [
        (PresentHost::Linux, OutputMode::Auto, true, HdrSwapchain::Sdr),
        (PresentHost::Linux, OutputMode::Auto, false, HdrSwapchain::Hdr10),
        (
            PresentHost::Windows,
            OutputMode::Auto,
            false,
            HdrSwapchain::Scrgb,
        ),
        (PresentHost::Linux, OutputMode::Sdr, false, HdrSwapchain::Sdr),
        (
            PresentHost::Windows,
            OutputMode::Hdr10,
            true,
            HdrSwapchain::Sdr,
        ),
    ];
    for (host, output, force, want) in cases {
        if chooser(host, &both, output, force) != want {
            wrong += 1.0;
        }
    }
    // HDR was asked and that pair is not listed. The result is SDR, not the
    // missing mode and not the other HDR pair that happens to be listed.
    let sdr_only = [ListedFormat {
        format: FORMAT_BGRA8,
        color_space: COLOR_SPACE_SRGB,
    }];
    let scrgb_only = [
        ListedFormat {
            format: FORMAT_R16F,
            color_space: COLOR_SPACE_SCRGB,
        },
        ListedFormat {
            format: FORMAT_BGRA8,
            color_space: COLOR_SPACE_SRGB,
        },
    ];
    let unlisted = [
        (PresentHost::Linux, &sdr_only[..], OutputMode::Hdr10),
        (PresentHost::Linux, &scrgb_only[..], OutputMode::Hdr10),
        (PresentHost::Windows, &sdr_only[..], OutputMode::Scrgb),
    ];
    for (host, list, output) in unlisted {
        if chooser(host, list, output, false) != HdrSwapchain::Sdr {
            wrong += 1.0;
        }
    }
    let white = swapchain_nits(HdrSwapchain::Scrgb, 203.0);
    if white.to_bits() != scrgb_code(203.0).to_bits() {
        wrong += 1.0;
    }
    let pq = swapchain_nits(HdrSwapchain::Hdr10, 203.0);
    if pq.to_bits() != pq_st2084(203.0).to_bits() {
        wrong += 1.0;
    }
    wrong
}

/// Mutation: Potato and Low keep the host HDR pair.
fn pick_ignores_force(
    host: PresentHost,
    list: &[ListedFormat],
    _output: OutputMode,
    _force: bool,
) -> HdrSwapchain {
    pick_hdr_output(host, list)
}

fn adapt_outside(jump: bool) -> f64 {
    outside_window(simulate_adapt(jump).0, 0.40, 2.00)
}

fn adapt_overshoot(jump: bool) -> f64 {
    simulate_adapt(jump).1
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

fn lab_from_code(code: [f64; 3]) -> [f64; 3] {
    let linear = [
        power_decode(code[0]),
        power_decode(code[1]),
        power_decode(code[2]),
    ];
    let xyz = mul(BT709_TO_XYZ, linear);
    let white = mul(BT709_TO_XYZ, [1.0, 1.0, 1.0]);
    let x = lab_f(xyz[0] / white[0]);
    let y = lab_f(xyz[1] / white[1]);
    let z = lab_f(xyz[2] / white[2]);
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

fn power_decode(code: f64) -> f64 {
    if code <= 0.0 {
        0.0
    } else {
        code.powf(2.4)
    }
}

fn lab_f(t: f64) -> f64 {
    let delta = 6.0 / 29.0;
    if t > delta * delta * delta {
        t.cbrt()
    } else {
        t / (3.0 * delta * delta) + 4.0 / 29.0
    }
}

fn delta_e2000(first: [f64; 3], second: [f64; 3]) -> f64 {
    if (first[0] - second[0]).abs() < 1e-12
        && (first[1] - second[1]).abs() < 1e-12
        && (first[2] - second[2]).abs() < 1e-12
    {
        return 0.0;
    }
    let (l1, a1, b1) = (first[0], first[1], first[2]);
    let (l2, a2, b2) = (second[0], second[1], second[2]);
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let c_bar = f64::midpoint(c1, c2);
    let c_bar7 = c_bar.powi(7);
    let g = 0.5 * (1.0 - (c_bar7 / (c_bar7 + 6_103_515_625.0)).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();
    let h1p = hue_deg(b1, a1p);
    let h2p = hue_deg(b2, a2p);
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let mut dh = h2p - h1p;
    if c1p * c2p == 0.0 {
        dh = 0.0;
    } else if dh > 180.0 {
        dh -= 360.0;
    } else if dh < -180.0 {
        dh += 360.0;
    }
    let dh = 2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin();
    let l_bar = f64::midpoint(l1, l2);
    let c_bar_p = f64::midpoint(c1p, c2p);
    let h_bar = mean_hue(h1p, h2p, c1p, c2p);
    let t = 1.0 - 0.17 * (h_bar - 30.0).to_radians().cos()
        + 0.24 * (2.0 * h_bar).to_radians().cos()
        + 0.32 * (3.0 * h_bar + 6.0).to_radians().cos()
        - 0.20 * (4.0 * h_bar - 63.0).to_radians().cos();
    let sl = 1.0 + (0.015 * (l_bar - 50.0).powi(2)) / (20.0 + (l_bar - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * c_bar_p;
    let sh = 1.0 + 0.015 * c_bar_p * t;
    let c_bar_p7 = c_bar_p.powi(7);
    let rt = -2.0
        * (c_bar_p7 / (c_bar_p7 + 6_103_515_625.0)).sqrt()
        * (60.0 * (-((h_bar - 275.0) / 25.0).powi(2)).exp())
            .to_radians()
            .sin();
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dh / sh).powi(2) + rt * (dc / sc) * (dh / sh)).sqrt()
}

fn hue_deg(b: f64, a: f64) -> f64 {
    if a == 0.0 && b == 0.0 {
        return 0.0;
    }
    let degrees = b.atan2(a).to_degrees();
    if degrees < 0.0 {
        degrees + 360.0
    } else {
        degrees
    }
}

fn mean_hue(h1: f64, h2: f64, c1: f64, c2: f64) -> f64 {
    if c1 * c2 == 0.0 {
        return h1 + h2;
    }
    let sum = h1 + h2;
    let diff = (h1 - h2).abs();
    if diff <= 180.0 {
        sum / 2.0
    } else if sum < 360.0 {
        f64::midpoint(sum, 360.0)
    } else {
        f64::midpoint(sum, -360.0)
    }
}

fn mul(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
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
        Some(number) => fmt_num(number),
        None => "-".to_string(),
    };
    let op = match metric.compare {
        Compare::AtMost => "<=",
        Compare::Equal => "==",
        Compare::Collected => "collected, device <=",
    };
    format!(
        "{} {value} {op}{} {}",
        metric.name,
        fmt_num(metric.threshold),
        status_str(metric.status)
    )
}

fn fmt_num(number: f64) -> String {
    if number != 0.0 && number.abs() < 1e-4 {
        format!("{number:.3e}")
    } else {
        format!("{number:.6}")
    }
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
        Compare::Equal => "equal",
        Compare::Collected => "collected",
    };
    format!(
        "{{\"name\":\"{}\",\"value\":{value},\"threshold\":{:.8},\"compare\":\"{compare}\",\"status\":\"{}\"}}",
        metric.name,
        metric.threshold,
        status_str(metric.status)
    )
}

struct Grab {
    ok: bool,
    width: usize,
    height: usize,
    bgra: bool,
    pixels: Vec<u8>,
    scene: Vec<f64>,
    bloom_ref: Vec<f64>,
    bloom_buf: Vec<f32>,
    bloom_add: Vec<f32>,
    exposure_got: f64,
    exposure_want: f64,
    gap_x4: f64,
    post_ms: f64,
    post_vram: f64,
}

fn gpu_grab() -> &'static Grab {
    static GRAB: OnceLock<Grab> = OnceLock::new();
    GRAB.get_or_init(capture_gpu)
}

fn capture_gpu() -> Grab {
    let scene = match load_hall() {
        Ok(scene) => scene,
        Err(err) => return fail_grab(&err),
    };
    let mut world = World::from_scene(scene);
    world.scene.lighting = LightingSettings::preset(LightingPreset::Potato);
    world.scene.hdr = gpu_settings();
    let opened = open_grade_renderer();
    let (mut window, mut renderer) = match opened {
        Ok(pair) => pair,
        Err(err) => return fail_grab(&err),
    };
    let camera = Camera::opening();
    if let Err(err) = draw_once(&mut window, &mut renderer, &world, &camera) {
        return fail_grab(&err);
    }
    // The second frame is a different lamp, so a pyramid of the previous
    // frame does not match this frame's bloom.
    if let Some(light) = world.scene.lights.first_mut() {
        light.color = [6.0, 0.3, 0.1];
    }
    let pixels = match draw_once(&mut window, &mut renderer, &world, &camera) {
        Ok(pixels) => pixels,
        Err(err) => return fail_grab(&err),
    };
    let scene2 = match renderer.read_scene() {
        Ok(scene) => scene,
        Err(err) => return fail_grab(&err),
    };
    let bloom_buf = match renderer.read_bloom() {
        Ok(bloom) => bloom,
        Err(err) => return fail_grab(&err),
    };
    let bloom_add = match renderer.read_hdr_encode() {
        Ok(encoded) => encoded,
        Err(err) => return fail_grab(&err),
    };
    let mut grab = finish_grab(&renderer, pixels, &scene2, bloom_buf, bloom_add);
    let mut post_ms = renderer.post_ms_device();
    if post_ms <= 0.0 {
        if draw_once(&mut window, &mut renderer, &world, &camera).is_err() {
            return fail_grab("the timestamp frame returned no readback");
        }
        post_ms = renderer.post_ms_device();
    }
    grab.post_ms = post_ms;
    grab.post_vram = renderer.post_vram_device();
    grab
}

fn gpu_post_ms() -> f64 {
    let ms = gpu_grab().post_ms;
    if ms.is_finite() && ms > 0.0 {
        ms
    } else {
        DEVICE_MS_MAX + 1.0
    }
}

fn gpu_post_vram() -> f64 {
    let mib = gpu_grab().post_vram;
    if mib.is_finite() && mib > 0.0 {
        mib
    } else {
        DEVICE_VRAM_MAX + 1.0
    }
}

fn load_hall() -> Result<genos_scene::Scene, String> {
    genos_scene::load_path(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
}

fn open_grade_renderer() -> Result<(Window, Renderer), String> {
    let mut window = Window::open_proof(160, 90)?;
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok((window, renderer))
}

fn draw_once(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> Result<Vec<u8>, String> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)?
        .ok_or_else(|| "the frame returned no readback".to_string())
}

fn gpu_settings() -> HdrSettings {
    let mut hdr = HdrSettings::for_preset(LightingPreset::High);
    hdr.exposure_mode = genos_scene::ExposureMode::Fixed;
    hdr.ev100 = GPU_EV100;
    hdr.compensation = 0.0;
    hdr.adapt_down = GPU_TAU;
    hdr.adapt_up = GPU_TAU;
    hdr.display = genos_scene::DisplayTransform::Agx;
    hdr.bloom = true;
    hdr.bloom_intensity = GPU_BLOOM_K;
    hdr.bloom_levels = GPU_BLOOM_LEVELS;
    hdr.output = OutputMode::Sdr;
    hdr.legacy_bytes = false;
    hdr
}

fn wanted_exposure() -> f64 {
    let target = exposure_with_compensation(GPU_EV100, 0.0);
    let mut scale = 1.0;
    let frames = usize::try_from(GPU_FRAMES).unwrap_or(0);
    for _ in 0..frames {
        scale = step_exposure(scale, target, 1.0 / 60.0, GPU_TAU, GPU_TAU);
    }
    scale
}

struct LegacyGrab {
    ok: bool,
    pixels: Vec<u8>,
}

fn legacy_grab() -> &'static LegacyGrab {
    static GRAB: OnceLock<LegacyGrab> = OnceLock::new();
    GRAB.get_or_init(capture_legacy)
}

/// Plain compose (`legacy_bytes`) against the stored shoulder picture.
/// Two settled frames must match, so a change to `tone_channel` cannot hide in noise.
fn legacy_shader_error(broken: bool) -> f64 {
    let grab = legacy_grab();
    if !grab.ok {
        return 255.0;
    }
    if grab.pixels.len() != LEGACY_SHOULDER.len() {
        return 255.0;
    }
    let mut worst = 0i32;
    for (index, (got, want)) in grab.pixels.iter().zip(LEGACY_SHOULDER.iter()).enumerate() {
        let mut code = i32::from(*got);
        if broken && index % 4 != 3 {
            code = (code + 8).min(255);
        }
        let err = (code - i32::from(*want)).abs();
        if err > worst {
            worst = err;
        }
    }
    f64::from(worst)
}

fn capture_legacy() -> LegacyGrab {
    let scene = match load_hall() {
        Ok(scene) => scene,
        Err(err) => {
            eprintln!("hdr gate legacy: {err}");
            return LegacyGrab {
                ok: false,
                pixels: Vec::new(),
            };
        }
    };
    let mut world = World::from_scene(scene);
    world.scene.lighting = LightingSettings::preset(LightingPreset::Potato);
    // Potato's ambient sits under 0.64, so the shoulder never runs. Raised
    // ambient puts the plain compose onto the curve this row grades.
    world.scene.lighting.ambient.intensity = 8.0;
    world.scene.hdr = HdrSettings::legacy_bytes();
    let opened = open_grade_renderer();
    let (mut window, mut renderer) = match opened {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("hdr gate legacy: {err}");
            return LegacyGrab {
                ok: false,
                pixels: Vec::new(),
            };
        }
    };
    let camera = Camera::opening();
    if let Err(err) = draw_once(&mut window, &mut renderer, &world, &camera) {
        eprintln!("hdr gate legacy: {err}");
        return LegacyGrab {
            ok: false,
            pixels: Vec::new(),
        };
    }
    let first = match draw_once(&mut window, &mut renderer, &world, &camera) {
        Ok(pixels) => pixels,
        Err(err) => {
            eprintln!("hdr gate legacy: {err}");
            return LegacyGrab {
                ok: false,
                pixels: Vec::new(),
            };
        }
    };
    let second = match draw_once(&mut window, &mut renderer, &world, &camera) {
        Ok(pixels) => pixels,
        Err(err) => {
            eprintln!("hdr gate legacy: {err}");
            return LegacyGrab {
                ok: false,
                pixels: Vec::new(),
            };
        }
    };
    let mut differ = 0usize;
    for (left, right) in first.iter().zip(second.iter()) {
        if left != right {
            differ += 1;
        }
    }
    if differ > 0 || first.len() != second.len() {
        eprintln!("hdr gate legacy: {differ} bytes moved between frames");
        return LegacyGrab {
            ok: false,
            pixels: Vec::new(),
        };
    }
    LegacyGrab {
        ok: true,
        pixels: first,
    }
}

struct LegacyModeGrab {
    ok: bool,
    width: usize,
    height: usize,
    bgra: bool,
    pixels: Vec<u8>,
    scene: Vec<f64>,
    exposure: f64,
    shoulder_pixels: u32,
}

fn legacy_mode_grab() -> &'static LegacyModeGrab {
    static GRAB: OnceLock<LegacyModeGrab> = OnceLock::new();
    GRAB.get_or_init(capture_legacy_mode)
}

/// `hdr_agx.comp` mode 0 against `tone`. Not the plain compose compile.
fn gpu_legacy_error(broken: bool) -> f64 {
    let grab = legacy_mode_grab();
    if !grab.ok || grab.shoulder_pixels == 0 {
        return 255.0;
    }
    let pixels = grab.width.saturating_mul(grab.height);
    let mut worst = 0.0;
    for index in 0..pixels {
        let scene = rgb3(&grab.scene, index);
        let got = displayed(&grab.pixels, index, grab.bgra);
        for channel in 0..3 {
            let exposed = grab.exposure * scene[channel];
            let formed = f64::from(tone(pack_f32(exposed))).clamp(0.0, 1.0);
            let want = (formed * 255.0).round();
            let mut code = got[channel] * 255.0;
            if broken {
                code += 8.0;
            }
            let err = (code - want).abs();
            if err > worst {
                worst = err;
            }
        }
    }
    worst
}

fn capture_legacy_mode() -> LegacyModeGrab {
    let empty = LegacyModeGrab {
        ok: false,
        width: 0,
        height: 0,
        bgra: false,
        pixels: Vec::new(),
        scene: Vec::new(),
        exposure: 0.0,
        shoulder_pixels: 0,
    };
    let scene = match load_hall() {
        Ok(scene) => scene,
        Err(err) => {
            eprintln!("hdr gate gpu_legacy: {err}");
            return empty;
        }
    };
    let mut world = World::from_scene(scene);
    world.scene.lighting = LightingSettings::preset(LightingPreset::Potato);
    world.scene.lighting.ambient.intensity = 8.0;
    let mut hdr = HdrSettings::for_preset(LightingPreset::Potato);
    hdr.display = genos_scene::DisplayTransform::Legacy;
    hdr.exposure_mode = genos_scene::ExposureMode::Fixed;
    hdr.ev100 = 0.0;
    hdr.compensation = 0.0;
    hdr.bloom = false;
    hdr.legacy_bytes = false;
    world.scene.hdr = hdr;
    let opened = open_grade_renderer();
    let (mut window, mut renderer) = match opened {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("hdr gate gpu_legacy: {err}");
            return empty;
        }
    };
    let camera = Camera::opening();
    if let Err(err) = draw_once(&mut window, &mut renderer, &world, &camera) {
        eprintln!("hdr gate gpu_legacy: {err}");
        return empty;
    }
    let pixels = match draw_once(&mut window, &mut renderer, &world, &camera) {
        Ok(pixels) => pixels,
        Err(err) => {
            eprintln!("hdr gate gpu_legacy: {err}");
            return empty;
        }
    };
    let scene = match renderer.read_scene() {
        Ok(scene) => scene,
        Err(err) => {
            eprintln!("hdr gate gpu_legacy: {err}");
            return empty;
        }
    };
    let exposure = renderer.hdr_exposure();
    let width = usize::try_from(scene.width).unwrap_or(0);
    let height = usize::try_from(scene.height).unwrap_or(0);
    let mut shoulder_pixels = 0u32;
    for index in 0..width.saturating_mul(height) {
        let color = rgb3(&scene.rgb, index);
        if color.iter().any(|channel| exposure * *channel > 0.64) {
            shoulder_pixels = shoulder_pixels.saturating_add(1);
        }
    }
    let (format, _) = renderer.hdr_surface();
    LegacyModeGrab {
        ok: true,
        width,
        height,
        bgra: format == FORMAT_BGRA8,
        pixels,
        scene: scene.rgb,
        exposure,
        shoulder_pixels,
    }
}

fn fail_grab(why: &str) -> Grab {
    eprintln!("hdr gate gpu: {why}");
    Grab {
        ok: false,
        width: 0,
        height: 0,
        bgra: false,
        pixels: Vec::new(),
        scene: Vec::new(),
        bloom_ref: Vec::new(),
        bloom_buf: Vec::new(),
        bloom_add: Vec::new(),
        exposure_got: 0.0,
        exposure_want: wanted_exposure(),
        gap_x4: 0.0,
        post_ms: 0.0,
        post_vram: DEVICE_VRAM_MAX + 1.0,
    }
}

fn finish_grab(
    renderer: &Renderer,
    pixels: Vec<u8>,
    scene2: &genos_render::SceneRead,
    bloom_buf: Vec<f32>,
    bloom_add: Vec<f32>,
) -> Grab {
    let width = usize::try_from(scene2.width).unwrap_or(0);
    let height = usize::try_from(scene2.height).unwrap_or(0);
    let bloom_ref = reference_pyramid(&scene2.rgb, width, height, GPU_BLOOM_LEVELS);
    let (format, _) = renderer.hdr_surface();
    let exposure_want = wanted_exposure();
    let gap_x4 = formula_gap(&scene2.rgb, &bloom_ref, exposure_want);
    Grab {
        ok: true,
        width,
        height,
        bgra: format == FORMAT_BGRA8,
        pixels,
        scene: scene2.rgb.clone(),
        bloom_ref,
        bloom_buf,
        bloom_add,
        exposure_got: renderer.hdr_exposure(),
        exposure_want,
        gap_x4,
        post_ms: 0.0,
        post_vram: 0.0,
    }
}

fn gpu_agx_error(broken: bool) -> f64 {
    let grab = gpu_grab();
    if !grab.ok {
        return 1.0;
    }
    let err = picture_error(grab, broken);
    if !broken && grab.gap_x4 < GPU_AGX_MAX * 2.0 {
        return 1.0;
    }
    err
}

fn gpu_exposure_error(broken: bool) -> f64 {
    let grab = gpu_grab();
    let got = if broken { 1.0 } else { grab.exposure_got };
    (got - grab.exposure_want).abs()
}

fn gpu_bloom_error(broken: bool) -> f64 {
    let grab = gpu_grab();
    if !grab.ok {
        return 1.0;
    }
    let scale = if broken { 2.0 } else { 1.0 };
    let pyramid = bloom_delta(&grab.bloom_buf, &grab.bloom_ref, scale);
    let added = bloom_delta(&grab.bloom_add, &grab.bloom_ref, scale);
    pyramid.max(added)
}

struct EncodeGrab {
    ok: bool,
    hdr10_surface: bool,
    scrgb_surface: bool,
    scene_hdr10: Vec<f64>,
    encode_hdr10: Vec<f32>,
    scene_scrgb: Vec<f64>,
    encode_scrgb: Vec<f32>,
    exposure: f64,
}

fn encode_grab() -> &'static EncodeGrab {
    static GRAB: OnceLock<EncodeGrab> = OnceLock::new();
    GRAB.get_or_init(capture_encode)
}

fn fail_encode(why: &str) -> EncodeGrab {
    eprintln!("hdr gate encode: {why}");
    EncodeGrab {
        ok: false,
        hdr10_surface: false,
        scrgb_surface: false,
        scene_hdr10: Vec::new(),
        encode_hdr10: Vec::new(),
        scene_scrgb: Vec::new(),
        encode_scrgb: Vec::new(),
        exposure: wanted_exposure(),
    }
}

fn encode_settings(output: OutputMode) -> HdrSettings {
    let mut hdr = HdrSettings::for_preset(LightingPreset::Medium);
    hdr.exposure_mode = genos_scene::ExposureMode::Fixed;
    hdr.ev100 = GPU_EV100;
    hdr.compensation = 0.0;
    hdr.adapt_down = GPU_TAU;
    hdr.adapt_up = GPU_TAU;
    hdr.display = genos_scene::DisplayTransform::Agx;
    hdr.bloom = false;
    hdr.bloom_intensity = 0.0;
    hdr.bloom_levels = 0;
    hdr.output = output;
    hdr.legacy_bytes = false;
    hdr
}

fn encode_lighting() -> LightingSettings {
    let mut lighting = LightingSettings::preset(LightingPreset::Medium);
    lighting.gi.enabled = false;
    lighting.bounce.count = 0;
    lighting
}

fn surface_is(renderer: &Renderer, hdr10: bool) -> bool {
    let (format, space) = renderer.hdr_surface();
    let sdr = (format == FORMAT_BGRA8 || format == FORMAT_RGBA8) && space == COLOR_SPACE_SRGB;
    if hdr10 {
        let pair = (format == FORMAT_A2B10 || format == FORMAT_A2R10) && space == COLOR_SPACE_HDR10;
        sdr || pair
    } else {
        let pair = format == FORMAT_R16F && space == COLOR_SPACE_SCRGB;
        sdr || pair
    }
}

fn capture_encode() -> EncodeGrab {
    let scene = match load_hall() {
        Ok(scene) => scene,
        Err(err) => return fail_encode(&err),
    };
    let mut world = World::from_scene(scene);
    world.scene.lighting = encode_lighting();
    world.scene.hdr = encode_settings(OutputMode::Hdr10);
    let opened = open_grade_renderer();
    let (mut window, mut renderer) = match opened {
        Ok(pair) => pair,
        Err(err) => return fail_encode(&err),
    };
    let camera = Camera::opening();
    if let Err(err) = draw_once(&mut window, &mut renderer, &world, &camera) {
        return fail_encode(&err);
    }
    if let Err(err) = draw_once(&mut window, &mut renderer, &world, &camera) {
        return fail_encode(&err);
    }
    let scene_hdr10 = match renderer.read_scene() {
        Ok(scene) => scene.rgb,
        Err(err) => return fail_encode(&err),
    };
    let encode_hdr10 = match renderer.read_hdr_encode() {
        Ok(encoded) => encoded,
        Err(err) => return fail_encode(&err),
    };
    let hdr10_surface = surface_is(&renderer, true);
    world.scene.hdr.output = OutputMode::Scrgb;
    if let Err(err) = draw_once(&mut window, &mut renderer, &world, &camera) {
        return fail_encode(&err);
    }
    if let Err(err) = draw_once(&mut window, &mut renderer, &world, &camera) {
        return fail_encode(&err);
    }
    let scene_scrgb = match renderer.read_scene() {
        Ok(scene) => scene.rgb,
        Err(err) => return fail_encode(&err),
    };
    let encode_scrgb = match renderer.read_hdr_encode() {
        Ok(encoded) => encoded,
        Err(err) => return fail_encode(&err),
    };
    EncodeGrab {
        ok: true,
        hdr10_surface,
        scrgb_surface: surface_is(&renderer, false),
        scene_hdr10,
        encode_hdr10,
        scene_scrgb,
        encode_scrgb,
        exposure: wanted_exposure(),
    }
}

fn hdr_encode_error(broken: bool) -> f64 {
    let grab = encode_grab();
    if !grab.ok || !grab.hdr10_surface {
        return 1.0;
    }
    encode_delta(&grab.scene_hdr10, &grab.encode_hdr10, grab.exposure, broken, false)
}

fn hdr_scrgb_error(broken: bool) -> f64 {
    let grab = encode_grab();
    if !grab.ok || !grab.scrgb_surface {
        return 1.0;
    }
    encode_delta(&grab.scene_scrgb, &grab.encode_scrgb, grab.exposure, broken, true)
}

fn encode_delta(scene: &[f64], encoded: &[f32], exposure: f64, broken: bool, scrgb: bool) -> f64 {
    let pixels = scene.len() / 3;
    if pixels == 0 || encoded.len() < pixels.saturating_mul(4) {
        return 1.0;
    }
    let mut sum = 0.0;
    let mut gap = 0.0;
    let mut seen = 0.0;
    for index in 0..pixels {
        let color = rgb3(scene, index);
        if scene_luma(color) < 0.02 {
            continue;
        }
        let linear = agx_display_linear([
            exposure * color[0],
            exposure * color[1],
            exposure * color[2],
        ]);
        let want = if scrgb {
            if broken {
                scrgb_at_paper(linear, 80.0)
            } else {
                scrgb_from_display(linear)
            }
        } else if broken {
            hdr10_at_paper(linear, 80.0)
        } else {
            hdr10_pq(linear)
        };
        let paper = if scrgb {
            scrgb_at_paper(linear, 80.0)
        } else {
            hdr10_at_paper(linear, 80.0)
        };
        let live = if scrgb {
            scrgb_from_display(linear)
        } else {
            hdr10_pq(linear)
        };
        let base = index.saturating_mul(4);
        for channel in 0..3 {
            let got = f64::from(encoded.get(base.saturating_add(channel)).copied().unwrap_or(0.0));
            sum += (got - want[channel]).abs();
            gap += (live[channel] - paper[channel]).abs();
            seen += 1.0;
        }
    }
    if seen < 64.0 * 3.0 {
        return 1.0;
    }
    let mean = sum / seen;
    if !broken && gap / seen < HDR_ENCODE_MAX * 2.0 {
        return 1.0;
    }
    mean
}

fn nits_at_paper(display: f64, white: f64, paper: f64) -> f64 {
    if !display.is_finite() || display <= 0.0 {
        return 0.0;
    }
    let white = white.clamp(1.0e-4, 0.999);
    let d = display.min(1.0);
    if d <= white {
        d / white * paper
    } else {
        let t = (d - white) / (1.0 - white);
        paper + t * (HDR_PEAK_NITS - paper)
    }
}

fn hdr10_at_paper(display: [f64; 3], paper: f64) -> [f64; 3] {
    let white = graphics_white_linear();
    let rec2020 = rec2020_from_rec709(display);
    [
        pq_st2084(nits_at_paper(rec2020[0], white, paper)),
        pq_st2084(nits_at_paper(rec2020[1], white, paper)),
        pq_st2084(nits_at_paper(rec2020[2], white, paper)),
    ]
}

fn scrgb_at_paper(display: [f64; 3], paper: f64) -> [f64; 3] {
    let white = graphics_white_linear();
    [
        nits_at_paper(display[0], white, paper) / 80.0,
        nits_at_paper(display[1], white, paper) / 80.0,
        nits_at_paper(display[2], white, paper) / 80.0,
    ]
}

fn picture_error(grab: &Grab, linear: bool) -> f64 {
    let pixels = grab.width.saturating_mul(grab.height);
    if pixels == 0 || grab.scene.len() < pixels.saturating_mul(3) {
        return 1.0;
    }
    if grab.bloom_ref.len() < pixels.saturating_mul(3) || grab.pixels.len() < pixels.saturating_mul(4)
    {
        return 1.0;
    }
    let mut sum = 0.0;
    let mut seen = 0.0;
    for index in 0..pixels {
        let scene = rgb3(&grab.scene, index);
        let bloom = rgb3(&grab.bloom_ref, index);
        if scene_luma(scene) < 0.02 && scene_luma(bloom) < 0.02 {
            continue;
        }
        let linear_rgb = exposed(scene, bloom, grab.exposure_want);
        let want = if linear {
            clamp01(linear_rgb)
        } else {
            agx_rec709(linear_rgb)
        };
        let got = displayed(&grab.pixels, index, grab.bgra);
        sum += (got[0] - want[0]).abs() + (got[1] - want[1]).abs() + (got[2] - want[2]).abs();
        seen += 3.0;
    }
    if seen < 64.0 * 3.0 {
        1.0
    } else {
        sum / seen
    }
}

fn formula_gap(scene: &[f64], bloom: &[f64], exposure: f64) -> f64 {
    let pixels = scene.len() / 3;
    if pixels == 0 || bloom.len() < scene.len() {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut seen = 0.0;
    for index in 0..pixels {
        let color = rgb3(scene, index);
        let add = rgb3(bloom, index);
        if scene_luma(color) < 0.02 && scene_luma(add) < 0.02 {
            continue;
        }
        let once = agx_rec709(exposed(color, add, exposure));
        let quad = agx_rec709(exposed_k(color, add, exposure, GPU_BLOOM_K * 4.0));
        sum += (once[0] - quad[0]).abs() + (once[1] - quad[1]).abs() + (once[2] - quad[2]).abs();
        seen += 3.0;
    }
    if seen == 0.0 {
        0.0
    } else {
        sum / seen
    }
}

fn exposed(scene: [f64; 3], bloom: [f64; 3], exposure: f64) -> [f64; 3] {
    exposed_k(scene, bloom, exposure, GPU_BLOOM_K)
}

fn exposed_k(scene: [f64; 3], bloom: [f64; 3], exposure: f64, intensity: f64) -> [f64; 3] {
    [
        exposure * (scene[0] + bloom[0] * intensity),
        exposure * (scene[1] + bloom[1] * intensity),
        exposure * (scene[2] + bloom[2] * intensity),
    ]
}

fn clamp01(color: [f64; 3]) -> [f64; 3] {
    [
        color[0].clamp(0.0, 1.0),
        color[1].clamp(0.0, 1.0),
        color[2].clamp(0.0, 1.0),
    ]
}

fn bloom_delta(buffer: &[f32], reference: &[f64], scale: f64) -> f64 {
    let pixels = reference.len() / 3;
    if pixels == 0 || buffer.len() < pixels.saturating_mul(4) {
        return 1.0;
    }
    let mut sum = 0.0;
    let mut energy = 0.0;
    for index in 0..pixels {
        let want = rgb3(reference, index);
        let base = index.saturating_mul(4);
        for (channel, want_c) in want.iter().enumerate() {
            let got = f64::from(buffer.get(base.saturating_add(channel)).copied().unwrap_or(0.0)) * scale;
            sum += (got - want_c).abs();
            energy += want_c.abs();
        }
    }
    let n = f64::from(u32::try_from(pixels.saturating_mul(3)).unwrap_or(u32::MAX));
    if n == 0.0 || energy / n < 1.0e-2 {
        1.0
    } else {
        sum / n
    }
}

fn rgb3(packed: &[f64], index: usize) -> [f64; 3] {
    let base = index.saturating_mul(3);
    [
        packed.get(base).copied().unwrap_or(0.0),
        packed.get(base.saturating_add(1)).copied().unwrap_or(0.0),
        packed.get(base.saturating_add(2)).copied().unwrap_or(0.0),
    ]
}

fn displayed(bytes: &[u8], index: usize, bgra: bool) -> [f64; 3] {
    let base = index.saturating_mul(4);
    let b0 = f64::from(bytes.get(base).copied().unwrap_or(0)) / 255.0;
    let b1 = f64::from(bytes.get(base.saturating_add(1)).copied().unwrap_or(0)) / 255.0;
    let b2 = f64::from(bytes.get(base.saturating_add(2)).copied().unwrap_or(0)) / 255.0;
    if bgra {
        [b2, b1, b0]
    } else {
        [b0, b1, b2]
    }
}

/// Tap pyramid. Same chain as `bloom_pyramid`, without calling it, so a ×2
/// inside `bloom_pyramid` disagrees with this buffer.
fn reference_pyramid(rgb: &[f64], width: usize, height: usize, levels: u32) -> Vec<f64> {
    let pixels = width.saturating_mul(height);
    if levels == 0 || width == 0 || height == 0 || rgb.len() < pixels.saturating_mul(3) {
        return vec![0.0; pixels.saturating_mul(3)];
    }
    let mut chain = vec![karis_downsample_rgb(rgb, width, height)];
    let count = usize::try_from(levels).unwrap_or(1).max(1);
    for _ in 1..count {
        let Some(current) = chain.last() else {
            break;
        };
        chain.push(downsample_linear(current));
    }
    let Some(mut acc) = chain.pop() else {
        return vec![0.0; pixels.saturating_mul(3)];
    };
    while let Some(finer) = chain.pop() {
        acc = upsample_add(&acc, &finer, true);
    }
    let full = upsample_add(
        &acc,
        &Image {
            pixels: vec![0.0; rgb.len()],
            width,
            height,
        },
        false,
    );
    full.pixels
}

fn downsample_linear(current: &Image) -> Image {
    let dst_w = (current.width / 2).max(1);
    let dst_h = (current.height / 2).max(1);
    let mut pixels = vec![0.0; dst_w.saturating_mul(dst_h).saturating_mul(3)];
    for channel in 0..3 {
        let plane: Vec<f64> = current
            .pixels
            .iter()
            .skip(channel)
            .step_by(3)
            .copied()
            .collect();
        let down = bloom_downsample(&plane, current.width, current.height);
        for (index, sample) in down.pixels.iter().enumerate() {
            let slot = index.saturating_mul(3).saturating_add(channel);
            if let Some(dst) = pixels.get_mut(slot) {
                *dst = *sample;
            }
        }
    }
    Image {
        pixels,
        width: dst_w,
        height: dst_h,
    }
}

fn upsample_add(coarse: &Image, finer: &Image, add_finer: bool) -> Image {
    let mut pixels = vec![0.0; finer.pixels.len()];
    for channel in 0..3 {
        let plane: Vec<f64> = coarse
            .pixels
            .iter()
            .skip(channel)
            .step_by(3)
            .copied()
            .collect();
        let image = Image {
            pixels: plane,
            width: coarse.width,
            height: coarse.height,
        };
        let up = bloom_upsample(&image, finer.width, finer.height);
        for (index, sample) in up.pixels.iter().enumerate() {
            let slot = index.saturating_mul(3).saturating_add(channel);
            let base = if add_finer {
                finer.pixels.get(slot).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            if let Some(dst) = pixels.get_mut(slot) {
                *dst = base + *sample;
            }
        }
    }
    Image {
        pixels,
        width: finer.width,
        height: finer.height,
    }
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
