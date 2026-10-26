//! HDR output metric gate. Calls the engine references. No GPU pass.
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
    aces2_rec709, agx_rec709, bloom_downsample, bloom_downsample_weight_sum, bloom_upsample,
    bloom_upsample_weight_sum, ev100_from_luminance, exposure_from_camera, exposure_from_ev100,
    histogram_ev100, linear_from_display, pick_hdr_output, pq_st2084, scrgb_code, tone,
    HdrSwapchain, ListedFormat, PresentHost, COLOR_SPACE_HDR10, COLOR_SPACE_SCRGB,
    COLOR_SPACE_SRGB, FORMAT_A2B10, FORMAT_A2R10, FORMAT_BGRA8, FORMAT_R16F, FORMAT_RGBA8,
};

use std::path::{Path, PathBuf};

const LEGACY_MAX: f64 = 1e-4;
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
const PEAK_MAX: f64 = 0.0025;
const GRADE_MAX: f64 = 0.02;
const LINEAR_MIN: f64 = 1.5;
const STILL_MAX: f64 = 0.001;
const ADAPT_MAX: f64 = 0.0;
const OVERSHOOT_MAX: f64 = 0.02;
const FIREFLY_MAX: f64 = 1.0;
const COMMUTE_MAX: f64 = 0.01;
const NEAR_MAX: f64 = 0.05;
const POP_MAX: f64 = 0.001;
const DEVICE_MS_MAX: f64 = 1.5;
const DEVICE_VRAM_MAX: f64 = 96.0;

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
    AtLeast,
    Equal,
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
        not_implemented("scene_peak", PEAK_MAX, Compare::AtMost),
        not_implemented("linear_grade", GRADE_MAX, Compare::AtMost),
        not_implemented("grade_is_linear", LINEAR_MIN, Compare::AtLeast),
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
        not_implemented("exposure_still", STILL_MAX, Compare::AtMost),
        not_implemented("exposure_adapt", ADAPT_MAX, Compare::AtMost),
        not_implemented("exposure_overshoot", OVERSHOOT_MAX, Compare::AtMost),
        at_most(
            "physical_camera",
            camera_error(exposure_from_camera),
            CAMERA_MAX,
        ),
        at_most("bloom_energy", bloom_error(), BLOOM_MAX),
        not_implemented("bloom_firefly", FIREFLY_MAX, Compare::AtMost),
        not_implemented("bloom_commute", COMMUTE_MAX, Compare::AtMost),
        at_most("output_pick", pick_misses(pick_hdr_output), PICK_MAX),
        not_implemented("near_error", NEAR_MAX, Compare::AtMost),
        not_implemented("still_pop", POP_MAX, Compare::AtMost),
        at_most("agx_de", agx_delta_e(agx_rec709), AGX_DE_MAX),
        at_most(
            "agx_highlight",
            agx_highlight_distance(agx_rec709),
            AGX_CODE_MAX,
        ),
        aces_row(aces2_rec709([1.0, 1.0, 1.0])),
        at_most("pq_st2084", pq_error(pq_st2084), PQ_MAX),
        equal("scrgb_code", scrgb_code(203.0), SCRGB_WHITE),
        not_implemented("post_ms_device", DEVICE_MS_MAX, Compare::AtMost),
        not_implemented("post_vram_device", DEVICE_VRAM_MAX, Compare::AtMost),
    ]
}

fn proof_good() -> Vec<Metric> {
    vec![
        at_most("legacy_inverse", legacy_inverse_error(), LEGACY_MAX),
        at_most("scene_peak", peak_err(4.0), PEAK_MAX),
        at_most(
            "linear_grade",
            rel_mean(&[0.5, 0.5, 0.5, 0.5], &[0.5, 0.5, 0.5, 0.5]),
            GRADE_MAX,
        ),
        at_least("grade_is_linear", max_channel(&[0.2, 8.0, 0.4]), LINEAR_MIN),
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
        at_most(
            "exposure_still",
            max_rel_step(&[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
            STILL_MAX,
        ),
        at_most("exposure_adapt", adapt_outside(false), ADAPT_MAX),
        at_most("exposure_overshoot", adapt_overshoot(false), OVERSHOOT_MAX),
        at_most(
            "physical_camera",
            camera_error(exposure_from_camera),
            CAMERA_MAX,
        ),
        at_most("bloom_energy", bloom_error(), BLOOM_MAX),
        at_most("bloom_firefly", karis_firefly(1_000.0), FIREFLY_MAX),
        at_most("bloom_commute", 0.0, COMMUTE_MAX),
        at_most("output_pick", pick_misses(pick_hdr_output), PICK_MAX),
        at_most("near_error", weighted(0.0, 0.20), NEAR_MAX),
        at_most(
            "still_pop",
            pop_share(&flat(100, 1.0), &flat(100, 1.0)),
            POP_MAX,
        ),
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
        at_most(
            "exposure_ev",
            exposure_patch_error(ev100_from_luminance, exposure_without_saturation),
            EV_MAX,
        ),
        at_most("exposure_histogram", histogram_error(mean_ev), HIST_MAX),
        at_most(
            "exposure_still",
            max_rel_step(&[1.0, 1.05, 1.10, 1.15, 1.20, 1.25, 1.30, 1.35]),
            STILL_MAX,
        ),
        at_most("exposure_adapt", adapt_outside(true), ADAPT_MAX),
        at_most("exposure_overshoot", adapt_overshoot(true), OVERSHOOT_MAX),
        at_most(
            "physical_camera",
            camera_error(exposure_without_saturation_camera),
            CAMERA_MAX,
        ),
        at_most("bloom_energy", 1.0, BLOOM_MAX),
        at_most("bloom_firefly", 1_000.0 * 0.125, FIREFLY_MAX),
        at_most("bloom_commute", 0.125, COMMUTE_MAX),
        at_most("output_pick", pick_misses(pick_always_sdr), PICK_MAX),
        at_most("near_error", weighted(0.20, 0.0), NEAR_MAX),
        at_most("still_pop", pop_share(&flat(100, 1.0), &popped()), POP_MAX),
        at_most("agx_de", agx_delta_e(|_scene| [1.0, 1.0, 1.0]), AGX_DE_MAX),
        at_most(
            "agx_highlight",
            outside_window(1.0, 0.998, 0.999_5),
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
    let mut samples = vec![0.18; 1000];
    samples.push(10_000.0);
    (meter(&samples) - spec_ev(0.18)).abs()
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

fn bloom_error() -> f64 {
    let weight = (bloom_downsample_weight_sum() - 1.0)
        .abs()
        .max((bloom_upsample_weight_sum() - 1.0).abs());
    let mut pixels = vec![0.0; 64 * 64];
    pixels[32 * 64 + 32] = 1.0;
    let first = bloom_downsample(&pixels, 64, 64);
    let second = bloom_downsample(&first.pixels, first.width, first.height);
    let up_first = bloom_upsample(&second, first.width, first.height);
    let up = bloom_upsample(&up_first, 64, 64);
    let mut sum = 0.0;
    for sample in &up.pixels {
        sum += *sample;
    }
    weight.max((sum - 1.0).abs())
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
        Compare::AtLeast => ">=",
        Compare::Equal => "==",
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

fn karis_firefly(centre: f64) -> f64 {
    let outer = centre / 4.0;
    let weight = |luma: f64| 1.0 / (1.0 + luma);
    let centre_w = 0.5 * weight(0.0);
    let outer_w = 0.125 * weight(outer);
    (outer * outer_w * 4.0) / (centre_w + outer_w * 4.0)
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
        Compare::Equal => "equal",
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
